// SOL 보내기 — 솔라나 지갑이니 SOL 도 받고 보낼 수 있게.
//
// x402 결제와 **다른 길**이다 — AI(MCP)는 이 길에 닿지 못한다. 시작점은 큰 창의 «SOL 보내기» 시트 하나이고,
// 언제나 사람이 비밀번호로 서명한다(자율 결제·작업 예산이 없다). USDC 한도(결제당·하루)는 USDC 에만 걸리므로
// 여기엔 적용하지 않는다 — 대신 긴급 잠금은 똑같이 막고, 보내기 전에 받는 주소·금액·수수료를 시트에서 다시 보여 준다.
//
// 순서 «비번 → 블록해시(클러스터 확인) → 잔액·받는 계정 확인 → 조립 → 자체 검증 → 서명 → 먼저 적기 → 보내기 → 확인».
// 트랜잭션은 레거시 메시지 하나, 명령은 System Program Transfer 하나 — 손으로 직렬화한다(svm_x402 와 같은 원칙).
//
// 기록: `~/.budu/sol_sends(-체인번호)(-a계정).json`(`chain_file` 규칙 — 데브넷은 접미사 없음, 메인넷은 `-101`) — USDC 결제 내역(history)과 **따로** 둔다. 한 파일에 섞으면
// 장부 합계·판매자 묶음·영수증 CSV·MCP get_history 가 전부 SOL 금액을 USDC 로 읽는다.
//
// 🔴 «나갔나 모름» 을 끝내는 법은 정산 확정(confirm.rs)과 같다: 서명이 체인에 있으면 그 결과, 없으면 그 블록해시의
//    마지막 유효 높이가 finalized 된 뒤에만 «만료 · 안 나감». 보내기 RPC 가 HTTP 단에서 끊기면(응답 없음) 나갔을 수 있다 —
//    «보내는 중» 으로 남기고 다시 보내지 말라고 말한다.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::fs;
use std::path::PathBuf;
use zeroize::Zeroizing;

use crate::balance::parse_to_addr;
use crate::chain::{active_chain, chain_file, with_pinned_chain};
use crate::i18n::{tf, ts};
use crate::keys::SolKeypair;
use crate::lock::read_lock;
use crate::settings::{effective_rpc, redact_urls};
use crate::solana::{self, parse_pubkey, rpc_call, RpcError};
use crate::store::{budu_dir, now_secs, write_json};
use crate::wallet::{account_file_name, active_account_index, unlock_signer_keep, with_pinned_account};

pub(crate) const SYSTEM_PROGRAM: &str = "11111111111111111111111111111111";
const SOL_DECIMALS: u32 = 9;
/// 서명 하나짜리 트랜잭션의 기본 수수료(5,000 lamports/서명). 우선 수수료는 넣지 않는다.
pub(crate) const FEE_LAMPORTS: u64 = 5_000;
/// 데이터 0바이트 계정의 rent 면제 최소 잔액 — 이보다 적게 남거나(0 은 괜찮다) 새 계정에 이보다 적게 보내면 런타임이 거절한다.
pub(crate) const RENT_EXEMPT_MIN: u64 = 890_880;
/// 파일에 남길 최근 기록 수.
const KEEP: usize = 200;
/// 만료 판정 여유 블록(confirm.rs 와 같은 값).
const EXPIRY_MARGIN: u64 = 32;
const IX_TRANSFER: u32 = 2;

/// 보낸 기록 한 줄.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub(crate) struct SolSend {
    pub(crate) ts: u64,
    pub(crate) to: String,
    pub(crate) lamports: u64,
    /// 우리 서명 = tx id (서명자가 우리 하나라 첫 서명이 곧 tx id — 탐색기에서 바로 찾힌다).
    pub(crate) signature: String,
    /// "sending"(서명해서 보냈거나 보내는 중 — 나갔을 수 있음) · "confirmed"(나감) · "failed"(안 나감) · "expired"(안 나감)
    pub(crate) status: String,
    #[serde(default)]
    pub(crate) detail: String,
    /// 이 블록해시의 마지막 유효 높이 — 만료 판정의 열쇠.
    pub(crate) valid_until: u64,
}

// ── 금액 ─────────────────────────────────────────────────────────────────────────────────

/// "0.5" → 500_000_000 lamports. 숫자·점만, 소수 9자리까지, 0 보다 커야 한다 (순수 — 테스트용).
pub(crate) fn parse_sol(s: &str) -> Result<u64, String> {
    let bad = || -> String { ts!("SOL 금액을 숫자로 적어 주세요 (소수 9자리까지)", "Enter the SOL amount as a number (up to 9 decimals)").into() };
    let s = s.trim();
    let (int, frac) = s.split_once('.').unwrap_or((s, ""));
    if (int.is_empty() && frac.is_empty())
        || !int.bytes().all(|b| b.is_ascii_digit())
        || !frac.bytes().all(|b| b.is_ascii_digit())
        || frac.len() > SOL_DECIMALS as usize
    {
        return Err(bad());
    }
    let int: u64 = if int.is_empty() { 0 } else { int.parse().map_err(|_| bad())? };
    let mut f = frac.to_string();
    while f.len() < SOL_DECIMALS as usize {
        f.push('0');
    }
    let frac: u64 = f.parse().map_err(|_| bad())?;
    let v = int
        .checked_mul(10u64.pow(SOL_DECIMALS))
        .and_then(|x| x.checked_add(frac))
        .ok_or_else(bad)?;
    if v == 0 {
        return Err(ts!("0보다 큰 금액이어야 해요", "The amount must be greater than 0").into());
    }
    Ok(v)
}

/// 잔액·받는 계정으로 이 송금이 체인에서 받아들여질지 미리 본다 (순수 — 테스트용).
/// 런타임 규칙: 보내고 남는 SOL 은 0 이거나 rent 면제 최소 이상, 아직 없는 계정에 처음 보내는 SOL 도 그 최소 이상.
pub(crate) fn check_amounts(balance: u64, lamports: u64, to_exists: bool) -> Result<(), String> {
    let min = solana::format_units(RENT_EXEMPT_MIN, SOL_DECIMALS as u8);
    let need = lamports.checked_add(FEE_LAMPORTS).ok_or("overflow")?;
    if need > balance {
        let fee = solana::format_units(FEE_LAMPORTS, 9);
        let need = solana::format_units(need, 9);
        let have = solana::format_units(balance, 9);
        return Err(tf!(
            "SOL 이 모자라요 — 수수료 {fee} SOL 까지 {need} SOL 이 필요한데 {have} SOL 이 있어요",
            "Not enough SOL — this needs {need} SOL including the {fee} SOL fee, and you have {have} SOL"
        ));
    }
    let rest = balance - need;
    if rest != 0 && rest < RENT_EXEMPT_MIN {
        return Err(tf!(
            "보내고 남는 SOL 이 너무 적어요 — 전부 보내거나 {min} SOL 이상 남겨 주세요",
            "That would leave too little SOL — send all of it, or keep at least {min} SOL"
        ));
    }
    if !to_exists && lamports < RENT_EXEMPT_MIN {
        return Err(tf!(
            "처음 쓰는 주소에는 {min} SOL 이상 보내야 해요 (솔라나 계정 최소 잔액)",
            "A brand-new address needs at least {min} SOL (Solana's minimum account balance)"
        ));
    }
    Ok(())
}

// ── 조립·검증·서명 ───────────────────────────────────────────────────────────────────────

fn push_compact_u16(out: &mut Vec<u8>, mut n: usize) {
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        b |= 0x80;
        out.push(b);
    }
}

/// 레거시 메시지: 헤더 [서명 1, 읽기전용 서명 0, 읽기전용 비서명 1] · 계정 [나, 받는 쪽, System] · 블록해시 ·
/// 명령 1개 = System Transfer(프로그램 2, 계정 [0, 1], 데이터 [2u32 LE, lamports u64 LE]).
pub(crate) fn build_message(from: &[u8; 32], to: &[u8; 32], lamports: u64, blockhash: &[u8; 32]) -> Result<Vec<u8>, String> {
    let system = parse_pubkey(SYSTEM_PROGRAM).expect("상수");
    if from == to {
        return Err("받는 주소가 내 주소".into());
    }
    if *to == system {
        return Err("받는 주소가 System Program".into());
    }
    if lamports == 0 {
        return Err("금액이 0".into());
    }
    let mut m = Vec::with_capacity(150);
    m.extend_from_slice(&[1, 0, 1]);
    push_compact_u16(&mut m, 3);
    m.extend_from_slice(from);
    m.extend_from_slice(to);
    m.extend_from_slice(&system);
    m.extend_from_slice(blockhash);
    push_compact_u16(&mut m, 1);
    m.push(2);
    push_compact_u16(&mut m, 2);
    m.extend_from_slice(&[0, 1]);
    push_compact_u16(&mut m, 12);
    m.extend_from_slice(&IX_TRANSFER.to_le_bytes());
    m.extend_from_slice(&lamports.to_le_bytes());
    Ok(m)
}

/// 만든 바이트를 다시 읽어 «내가 승인한 그 송금 하나뿐» 인지 본다 — 조립 코드가 틀려도 서명 전에 멈추게.
pub(crate) fn verify_message(m: &[u8], from: &[u8; 32], to: &[u8; 32], lamports: u64, blockhash: &[u8; 32]) -> Result<(), String> {
    // 고정 길이: 헤더3 + 1 + 32*3 + 32 + 1 + 1 + 1 + 2 + 1 + 12 = 150
    if m.len() != 150 {
        return Err("메시지 길이".into());
    }
    if m[..4] != [1, 0, 1, 3] {
        return Err("헤더".into());
    }
    if &m[4..36] != from || &m[36..68] != to || m[68..100] != parse_pubkey(SYSTEM_PROGRAM).expect("상수") {
        return Err("계정".into());
    }
    if &m[100..132] != blockhash {
        return Err("블록해시".into());
    }
    if m[132..138] != [1, 2, 2, 0, 1, 12] {
        return Err("명령 모양".into());
    }
    if m[138..142] != IX_TRANSFER.to_le_bytes() || m[142..150] != lamports.to_le_bytes() {
        return Err("명령 데이터".into());
    }
    Ok(())
}

/// 조립 → 자체 검증 → 서명 → (트랜잭션 바이트, 서명).
pub(crate) fn sign_transfer(kp: &SolKeypair, to: &[u8; 32], lamports: u64, blockhash: &[u8; 32]) -> Result<(Vec<u8>, [u8; 64]), String> {
    let from = kp.public_key();
    let msg = build_message(&from, to, lamports, blockhash)?;
    verify_message(&msg, &from, to, lamports, blockhash)?;
    let sig = kp.sign(&msg);
    if !kp.verify(&msg, &sig) {
        return Err("서명 확인 실패".into());
    }
    let mut tx = Vec::with_capacity(1 + 64 + msg.len());
    push_compact_u16(&mut tx, 1);
    tx.extend_from_slice(&sig);
    tx.extend_from_slice(&msg);
    Ok((tx, sig))
}

// ── 기록 ─────────────────────────────────────────────────────────────────────────────────

static FILE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(account_file_name(&chain_file("sol_sends"), active_account_index())))
}

fn read_at(p: &PathBuf) -> Result<Vec<SolSend>, String> {
    match fs::read_to_string(p) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.to_string()),
    }
}

/// 맨 앞에 넣는다. 파일이 깨졌으면 덮지 않고 실패 — 그러면 서명도 내보내지 않는다(기록 없는 송금을 만들지 않는다).
fn prepend(row: SolSend) -> Result<(), String> {
    let _g = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let p = path()?;
    let mut list = read_at(&p)?;
    list.insert(0, row);
    list.truncate(KEEP);
    write_json(p, &list)
}

fn update(signature: &str, status: &str, detail: &str) {
    let _g = FILE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let Ok(p) = path() else { return };
    let Ok(mut list) = read_at(&p) else { return };
    if let Some(r) = list.iter_mut().find(|r| r.signature == signature) {
        r.status = status.into();
        r.detail = detail.into();
        let _ = write_json(p, &list);
    }
}

// ── 체인 ─────────────────────────────────────────────────────────────────────────────────

fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| e.to_string())
}

/// `getSignatureStatuses` 한 건 → Some(성공했나) · None(아직 없음). commitment 가 confirmed 이상일 때만 결말로 본다.
fn status_of(v: &Value) -> Option<bool> {
    let s = v.pointer("/value/0")?;
    if s.is_null() {
        return None;
    }
    let level = s.get("confirmationStatus").and_then(Value::as_str).unwrap_or("");
    if level != "confirmed" && level != "finalized" {
        return None;
    }
    Some(s.get("err").is_none_or(Value::is_null))
}

async fn signature_status(c: &reqwest::Client, rpc: &str, sig: &str) -> Result<Option<bool>, RpcError> {
    let v = rpc_call(c, rpc, "getSignatureStatuses", json!([[sig], {"searchTransactionHistory": true}])).await?;
    Ok(status_of(&v))
}

/// 송금 결과(화면용).
#[derive(Serialize)]
pub(crate) struct SolSendResult {
    signature: String,
    status: String,
}

/// SOL 을 보낸다 — 비밀번호 승인. AI 쪽(MCP)에는 이 명령이 없다.
#[tauri::command]
pub(crate) async fn send_sol(
    password: String,
    to: String,
    amount: String,
    expected_chain_id: u64,
) -> Result<SolSendResult, String> {
    let password = Zeroizing::new(password);
    // 사람이 확인 화면에서 본 네트워크와 지금 보낼 네트워크가 다르면 멈춘다(x402 의 ensure_request_chain 과 같은 뜻).
    if active_chain().chain_id != expected_chain_id {
        return Err(ts!(
            "확인 화면을 연 뒤 네트워크 설정이 바뀌었어요. 보내지 않았어요 — 시트를 다시 여세요.",
            "The network setting changed after you opened this sheet. Nothing was sent — reopen it."
        )
        .into());
    }
    // 한 번에 한 건 — 이중 송금 가드(has_pending)와 기록(prepend) 사이를 동시 호출이 지나가지 못하게.
    let _one = SEND_LOCK.lock().await;
    // 대조한 그 체인으로 고정한다(설정을 다시 읽지 않는다).
    with_pinned_chain(
        expected_chain_id,
        with_pinned_account(active_account_index(), async move { send_inner(&password, &to, &amount).await }),
    )
    .await
}

fn lock_msg() -> String {
    ts!(
        "긴급 잠금이 켜져 있어 보내지 않았어요. 해제 후 다시 시도하세요.",
        "Emergency lock is on, so nothing was sent. Turn it off and try again."
    )
    .into()
}

async fn send_inner(password: &str, to: &str, amount: &str) -> Result<SolSendResult, String> {
    let lamports = parse_sol(amount)?;
    let to = parse_to_addr(to)?;
    let to_key = parse_pubkey(&to).ok_or("bad address")?;
    if read_lock() {
        return Err(lock_msg());
    }
    // 🔴 결말을 모르는 앞 송금이 있으면 새로 보내지 않는다 — 사람이 «안 갔나?» 하고 같은 걸 다시 보내면 이중 송금이다.
    //    먼저 체인에 물어 끝낼 수 있는 건 끝내고, 그래도 남으면 거절한다(블록해시 만료까지 1~2분이면 결말이 난다).
    if has_pending() {
        check_inner().await;
        if has_pending() {
            return Err(ts!(
                "앞서 보낸 송금의 결과를 아직 몰라요. 나갔을 수 있으니 1~2분 뒤 다시 시도하세요. 계속 이러면 지정한 RPC 가 결과 조회를 못 하는 것일 수 있어요 — 설정의 RPC 서버를 확인하세요.",
                "An earlier transfer's outcome isn't known yet. It may have gone through — try again in a minute or two. If this keeps happening, your RPC may not support looking up results — check the RPC server in Settings."
            )
            .into());
        }
    }
    // 12단어는 바로 버린다(Zeroizing) — SOL 보내기는 작업 예산처럼 키를 들고 있지 않는다.
    let (signer, _phrase) = unlock_signer_keep(password)?;
    if to == signer.address() {
        return Err(ts!("받는 주소가 이 지갑 주소예요", "That's this wallet's own address").into());
    }
    if to == SYSTEM_PROGRAM {
        return Err(ts!("받는 주소가 올바른 솔라나 주소가 아니에요", "That recipient isn't a valid Solana address").into());
    }
    let chain = active_chain();
    let rpc = effective_rpc();
    let c = client()?;
    let not_sent = |e: String| -> String {
        let e = redact_urls(&e);
        tf!(
            "네트워크 확인에 실패해 보내지 않았어요 ({e})",
            "Couldn't check the network, so nothing was sent ({e})"
        )
    };
    // 블록해시는 클러스터 확인(genesis)과 같이 받는다 — 지정 RPC 가 딴 클러스터면 여기서 멈춘다.
    let latest = solana::fetch_latest_blockhash(&c, &rpc, chain.genesis_hash).await.map_err(not_sent)?;
    let owner = signer.address();
    let (bal, info) = tokio::join!(
        rpc_call(&c, &rpc, "getBalance", json!([owner, {"commitment": "confirmed"}])),
        rpc_call(&c, &rpc, "getAccountInfo", json!([to, {"commitment": "confirmed", "encoding": "base64"}])),
    );
    let balance = bal
        .map_err(|e| not_sent(e.to_string()))?
        .pointer("/value")
        .and_then(Value::as_u64)
        .ok_or_else(|| not_sent("getBalance".into()))?;
    let info = info.map_err(|e| not_sent(e.to_string()))?;
    let acct = info.pointer("/value").filter(|v| !v.is_null());
    let to_exists = acct.is_some();
    // 프로그램 소유 계정(토큰 계정·PDA)이나 프로그램에 보내면 SOL 이 묶인다 — 일반 지갑 주소에만 보낸다.
    if let Some(a) = acct {
        let owner_prog = a.get("owner").and_then(Value::as_str).unwrap_or("");
        let executable = a.get("executable").and_then(Value::as_bool).unwrap_or(false);
        if executable || owner_prog != SYSTEM_PROGRAM {
            return Err(ts!(
                "받는 주소가 일반 지갑이 아니에요(토큰 계정이나 프로그램). 여기로 보낸 SOL 은 꺼낼 수 없을 수 있어 보내지 않았어요.",
                "That address isn't a regular wallet (a token account or a program). SOL sent there may be stuck, so nothing was sent."
            )
            .into());
        }
    }
    check_amounts(balance, lamports, to_exists)?;
    // 비번 복호화·RPC 사이에 잠금이 켜졌으면 서명하지 않는다.
    if read_lock() {
        return Err(lock_msg());
    }
    let (tx, sig) = sign_transfer(&signer, &to_key, lamports, &latest.blockhash).map_err(|e| {
        tf!(
            "송금 트랜잭션 자체 검증에 실패해 서명하지 않았어요 ({e})",
            "The transfer failed its self-check, so nothing was signed ({e})"
        )
    })?;
    let signature = bs58::encode(sig).into_string();

    // 🔴 먼저 적는다 — 못 적으면 보내지 않는다(기록 없는 송금은 «나갔나» 를 다시 물을 길이 없다).
    prepend(SolSend {
        ts: now_secs(),
        to: to.clone(),
        lamports,
        signature: signature.clone(),
        status: "sending".into(),
        detail: String::new(),
        valid_until: latest.last_valid_block_height,
    })
    .map_err(|_| {
        String::from(ts!(
            "보낸 기록을 저장하지 못해 보내지 않았어요 — 디스크 공간을 확인하세요.",
            "Couldn't save the record, so nothing was sent — check disk space."
        ))
    })?;

    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    let sent = rpc_call(
        &c,
        &rpc,
        "sendTransaction",
        json!([B64.encode(&tx), {"encoding": "base64", "preflightCommitment": "confirmed"}]),
    )
    .await;
    match sent {
        Ok(_) => {}
        // 노드가 «받지 않았다» 가 확실한 오류로 답했다(사전 시뮬레이션·서명 검증 실패, 잘못된 인자) — 안 나감.
        // 그 밖의 JSON-RPC 오류(-32603 내부 오류·게이트웨이 타임아웃 등)는 넘긴 뒤에 났을 수 있다 → 아래 «보내는 중».
        Err(RpcError::Rpc { code, message }) if surely_rejected(code, &message) => {
            let why = redact_urls(&message);
            update(&signature, "failed", &why);
            return Err(tf!(
                "네트워크가 송금을 받지 않았어요. 아무것도 나가지 않았습니다 ({why})",
                "The network refused the transfer. Nothing was sent ({why})"
            ));
        }
        // 응답이 끊겼거나 애매한 오류 — 들어갔을 수 있다. «보내는 중» 으로 두고 아래에서 체인에 물어본다.
        Err(_) => {}
    }
    // 25초까지 기다려 본다(RPC 가 느려도 전체 상한). 못 보면 «보내는 중» 으로 두고 화면이 나중에 다시 확인한다.
    let poll = async {
        for _ in 0..14 {
            tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
            if let Ok(Some(ok)) = signature_status(&c, &rpc, &signature).await {
                return Some(ok);
            }
        }
        None
    };
    if let Ok(Some(ok)) = tokio::time::timeout(std::time::Duration::from_secs(25), poll).await {
        let status = if ok { "confirmed" } else { "failed" };
        update(&signature, status, "");
        return Ok(SolSendResult { signature, status: status.into() });
    }
    Ok(SolSendResult { signature, status: "sending".into() })
}

/// 노드가 트랜잭션을 **받지 않았다** 가 확실한 JSON-RPC 오류 코드.
/// -32002 사전 시뮬레이션 실패 · -32003 서명 검증 실패 · -32602 잘못된 인자 · -32600 잘못된 요청.
/// 단 «이미 처리됨»(게이트웨이가 같은 tx 를 다시 보내 두 번째 preflight 가 받은 답)은 나간 것이라 빼고 체인에 묻는다.
fn surely_rejected(code: i64, message: &str) -> bool {
    let m = message.to_ascii_lowercase();
    if m.contains("alreadyprocessed") || m.contains("already been processed") {
        return false;
    }
    matches!(code, -32002 | -32003 | -32602 | -32600)
}

static SEND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

fn has_pending() -> bool {
    get_sol_sends().iter().any(|r| r.status == "sending")
}

/// 최근 보낸 기록 — 화면용. 못 읽으면 빈 목록.
#[tauri::command]
pub(crate) fn get_sol_sends() -> Vec<SolSend> {
    path().and_then(|p| read_at(&p)).unwrap_or_default()
}

/// «보내는 중» 기록의 결말을 체인에 묻는다. 서명이 있으면 그 결과, 없으면 마지막 유효 높이가 finalized 된 뒤에만 «만료».
/// 바뀐 게 있으면 true.
#[tauri::command]
pub(crate) async fn check_sol_sends() -> bool {
    with_pinned_chain(active_chain().chain_id, with_pinned_account(active_account_index(), check_inner())).await
}

async fn check_inner() -> bool {
    let pending: Vec<SolSend> = get_sol_sends().into_iter().filter(|r| r.status == "sending").collect();
    if pending.is_empty() {
        return false;
    }
    let Ok(c) = client() else { return false };
    let chain = active_chain();
    let rpc = effective_rpc();
    // 딴 클러스터 RPC 에 물으면 «없음» 이 거짓이 된다.
    match rpc_call(&c, &rpc, "getGenesisHash", json!([])).await {
        Ok(g) if g.as_str() == Some(chain.genesis_hash) => {}
        _ => return false,
    }
    // 높이를 **먼저** 읽는다 — 그 뒤에 서명 상태가 «없음» 이면, 그 높이까지 확정된 블록 어디에도 없다는 뜻이다.
    let Some(finalized) = rpc_call(&c, &rpc, "getBlockHeight", json!([{"commitment": "finalized"}]))
        .await
        .ok()
        .and_then(|v| v.as_u64())
    else {
        return false;
    };
    let mut changed = false;
    for r in pending {
        match signature_status(&c, &rpc, &r.signature).await {
            Ok(Some(ok)) => {
                update(&r.signature, if ok { "confirmed" } else { "failed" }, "");
                changed = true;
            }
            Ok(None) if finalized > r.valid_until.saturating_add(EXPIRY_MARGIN) => {
                update(&r.signature, "expired", "");
                changed = true;
            }
            _ => {}
        }
    }
    changed
}

/// 화면에 쓸 상수(수수료·최소 잔액) — 시트가 «전부 보내기» 와 안내를 같은 값으로 계산하게.
#[derive(Serialize)]
pub(crate) struct SolRules {
    fee_lamports: u64,
    rent_exempt_min: u64,
}

#[tauri::command]
pub(crate) fn sol_rules() -> SolRules {
    SolRules { fee_lamports: FEE_LAMPORTS, rent_exempt_min: RENT_EXEMPT_MIN }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_sure_rpc_errors_mean_not_sent() {
        assert!(surely_rejected(-32002, "Transaction simulation failed: insufficient funds"));
        assert!(surely_rejected(-32003, "Transaction signature verification failure"));
        // 내부 오류·타임아웃은 넘긴 뒤일 수 있다 — «안 나감» 으로 확정하지 않는다.
        assert!(!surely_rejected(-32603, "Internal error"));
        assert!(!surely_rejected(-32000, "timeout"));
        assert!(!surely_rejected(-32005, "Node is unhealthy"));
        // 게이트웨이 재전송으로 «이미 처리됨» — 나간 것.
        assert!(!surely_rejected(-32002, "Transaction simulation failed: This transaction has already been processed"));
        assert!(!surely_rejected(-32002, "AlreadyProcessed"));
    }

    #[test]
    fn parse_sol_units_and_rejects() {
        assert_eq!(parse_sol("1").unwrap(), 1_000_000_000);
        assert_eq!(parse_sol(" 0.5 ").unwrap(), 500_000_000);
        assert_eq!(parse_sol(".000000001").unwrap(), 1);
        assert_eq!(parse_sol("2.").unwrap(), 2_000_000_000);
        for bad in ["", ".", "0", "0.0", "-1", "1e3", "1,5", "0.0000000001", "abc", "18446744074"] {
            assert!(parse_sol(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn amount_rules_follow_runtime() {
        let one = 1_000_000_000;
        assert!(check_amounts(one, one, true).is_err(), "수수료 모자람");
        assert!(check_amounts(one, one - FEE_LAMPORTS, true).is_ok(), "전부 보내기(0 남김)");
        assert!(check_amounts(one, one - FEE_LAMPORTS - 1, true).is_err(), "1 lamport 남김 = rent 미달");
        assert!(check_amounts(one, one - FEE_LAMPORTS - RENT_EXEMPT_MIN, true).is_ok());
        assert!(check_amounts(one, RENT_EXEMPT_MIN - 1, false).is_err(), "새 계정에 최소 미만");
        assert!(check_amounts(one, RENT_EXEMPT_MIN, false).is_ok());
        assert!(check_amounts(one, 1, true).is_ok(), "있는 계정엔 1 lamport 도 된다");
    }

    fn kp() -> SolKeypair {
        SolKeypair::from_secret_bytes(&[7u8; 32])
    }

    #[test]
    fn message_layout_is_a_single_system_transfer() {
        let k = kp();
        let to = parse_pubkey("2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4").unwrap();
        let bh = [9u8; 32];
        let m = build_message(&k.public_key(), &to, 1234, &bh).unwrap();
        assert_eq!(m.len(), 150);
        verify_message(&m, &k.public_key(), &to, 1234, &bh).unwrap();
        // System Program = 32바이트 0.
        assert_eq!(&m[68..100], &[0u8; 32]);
        assert_eq!(&m[138..142], &[2, 0, 0, 0]);
        assert_eq!(u64::from_le_bytes(m[142..150].try_into().unwrap()), 1234);
        // 한 바이트라도 다르면 거절.
        for i in 0..m.len() {
            let mut t = m.clone();
            t[i] ^= 1;
            assert!(verify_message(&t, &k.public_key(), &to, 1234, &bh).is_err(), "바이트 {i}");
        }
        assert!(verify_message(&m, &k.public_key(), &to, 1235, &bh).is_err());
    }

    #[test]
    fn refuses_self_system_and_zero() {
        let k = kp();
        let me = k.public_key();
        assert!(build_message(&me, &me, 1, &[0; 32]).is_err());
        assert!(build_message(&me, &[0u8; 32], 1, &[0; 32]).is_err());
        let to = parse_pubkey("2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4").unwrap();
        assert!(build_message(&me, &to, 0, &[0; 32]).is_err());
    }

    #[test]
    fn signed_tx_verifies_with_our_key() {
        let k = kp();
        let to = parse_pubkey("2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4").unwrap();
        let (tx, sig) = sign_transfer(&k, &to, 5, &[1; 32]).unwrap();
        assert_eq!(tx[0], 1);
        assert_eq!(&tx[1..65], &sig);
        assert!(k.verify(&tx[65..], &sig));
    }

    #[test]
    fn status_reading() {
        assert_eq!(status_of(&json!({"value": [null]})), None);
        assert_eq!(status_of(&json!({"value": [{"confirmationStatus": "processed", "err": null}]})), None);
        assert_eq!(status_of(&json!({"value": [{"confirmationStatus": "confirmed", "err": null}]})), Some(true));
        assert_eq!(status_of(&json!({"value": [{"confirmationStatus": "finalized", "err": {"InstructionError": [0, "x"]}}]})), Some(false));
    }

    #[test]
    fn records_round_trip_and_refuse_broken_file() {
        let _d = crate::store::test_dir::use_temp("sol-sends");
        assert!(get_sol_sends().is_empty());
        let row = SolSend { ts: 1, to: "a".into(), lamports: 2, signature: "s".into(), status: "sending".into(), detail: String::new(), valid_until: 9 };
        prepend(row.clone()).unwrap();
        update("s", "confirmed", "");
        assert_eq!(get_sol_sends()[0].status, "confirmed");
        fs::write(path().unwrap(), "{broken").unwrap();
        assert!(prepend(row).is_err(), "깨진 파일은 덮지 않는다 → 보내지도 않는다");
    }

    // 라이브 — 데브넷 RPC 에 손 직렬화한 송금을 시뮬레이션시킨다. `cargo test -- --ignored live_sol_send` 로만.
    // 데브넷 RPC 에어드롭이 닫혀(410) 새 키를 채울 수 없다 → 돈이 있는 공개 계정(데모 페이실리테이터)을 «보내는 쪽» 으로 두고
    // 서명 검사 없이(sigVerify false) 실행시켜 명령 모양을, 우리 키로 서명한 것은 서명 검사를 켜고 돌려 서명·메시지 바이트를 본다.
    #[tokio::test]
    #[ignore]
    async fn live_sol_send_devnet_simulation() {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        let rpc = "https://api.devnet.solana.com";
        let c = client().unwrap();
        let funded = parse_pubkey("CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5").unwrap();
        let to = parse_pubkey("2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4").unwrap();
        let latest = solana::fetch_latest_blockhash(&c, rpc, solana::GENESIS_DEVNET).await.unwrap();
        // ① 명령 실행 — 서명 자리는 0, 검사 끔.
        let msg = build_message(&funded, &to, RENT_EXEMPT_MIN, &latest.blockhash).unwrap();
        let mut tx = vec![1u8];
        tx.extend_from_slice(&[0u8; 64]);
        tx.extend_from_slice(&msg);
        let v = rpc_call(&c, rpc, "simulateTransaction", json!([B64.encode(&tx), {"encoding": "base64", "sigVerify": false, "commitment": "confirmed"}]))
            .await
            .unwrap();
        eprintln!("① {v}");
        assert!(v.pointer("/value/err").is_some_and(Value::is_null), "송금 명령이 실패: {v}");
        // ② 우리 키 서명 — 검사 켬. 그 키의 계정 상태 때문에 실행은 다른 이유로 멈출 수 있지만(실측: InvalidAccountForFee),
        //    서명·메시지 바이트가 틀리면 그보다 먼저 «SignatureFailure»/서명 검사 오류로 거절된다 — 그것만 없으면 된다.
        let k = kp();
        let (tx2, _) = sign_transfer(&k, &to, RENT_EXEMPT_MIN, &latest.blockhash).unwrap();
        let v2 = rpc_call(&c, rpc, "simulateTransaction", json!([B64.encode(&tx2), {"encoding": "base64", "sigVerify": true, "commitment": "confirmed"}]))
            .await;
        eprintln!("② {v2:?}");
        let text = format!("{v2:?}");
        assert!(v2.is_ok(), "{text}");
        assert!(!text.contains("SignatureFailure") && !text.to_lowercase().contains("signature verification"), "서명 검사에서 걸림: {text}");
    }
}
