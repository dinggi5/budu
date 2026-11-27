// 솔라나 공용 코드 — GUI(src-tauri)와 MCP(budu-mcp)가 **같은 소스 파일**을 컴파일한다.
//
// policy.rs 와 같은 방식(`#[path]` 모듈, 크레이트 아님)이지만 조건이 다르다: 여기는 RPC 를 부르므로
// 두 크레이트가 같은 버전의 `reqwest`(0.12)·`bs58`·`sha2`·`curve25519-dalek`·`serde_json` 을 가진다.
// 두 크레이트 어느 쪽 모듈도 참조하지 않는다 — i18n 문구는 각 크레이트가 `BalanceError` 를 받아 붙인다.
//
// 담는 것: 주소(base58) · ATA(PDA) 계산 · 잔액 JSON-RPC · 단위 문자열. 키·서명은 없다(키는 GUI 만 — keys.rs).
// 메시지 직렬화·명령은 개발 3 에서 GUI 쪽에 손으로 짠다.

use serde_json::{json, Value};
use sha2::{Digest, Sha256};

// ── 프로그램 · 민트 · 클러스터 ─────────────────────────────────────────────────────────────────

/// 기존 SPL Token 프로그램.
pub const TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
/// Associated Token Account 프로그램.
pub const ASSOCIATED_TOKEN_PROGRAM_ID: &str = "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL";

/// Circle USDC 민트 (6 decimals). 출처: x402 svm 스펙·Circle 문서.
pub const USDC_MINT_MAINNET: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDC_MINT_DEVNET: &str = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";

/// `getGenesisHash` 전체 값. 지정 RPC 가 고른 클러스터에 붙어 있는지 이걸로 본다.
///
pub const GENESIS_MAINNET: &str = "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdpKuc147dw2N9d";
pub const GENESIS_DEVNET: &str = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";

/// 잔액을 읽는 commitment. 정산 확정은 따로 정한다.
const COMMITMENT: &str = "confirmed";

// ── 주소 ─────────────────────────────────────────────────────────────────────────────────

/// base58 문자열 → 32바이트. 길이가 정확히 32 가 아니면 None (앞뒤 공백은 허용).
/// 곡선 위 여부는 보지 않는다 — PDA(토큰 계정)도 주소다. 「지갑 주소여야 하는 자리」는 `is_on_curve` 를 따로 본다.
pub fn parse_pubkey(s: &str) -> Option<[u8; 32]> {
    let v = bs58::decode(s.trim()).into_vec().ok()?;
    v.try_into().ok()
}

/// 32바이트 → base58 문자열.
pub fn encode_pubkey(b: &[u8; 32]) -> String {
    bs58::encode(b).into_string()
}

/// ed25519 곡선 위의 점인가 (= 개인키가 있을 수 있는 주소). PDA 는 일부러 곡선 밖이다.
pub fn is_on_curve(b: &[u8; 32]) -> bool {
    curve25519_dalek::edwards::CompressedEdwardsY(*b)
        .decompress()
        .is_some()
}

/// `find_program_address` — 시드 + bump(255→0) + 프로그램 id + "ProgramDerivedAddress" 의 sha256 이
/// 곡선 밖에 떨어지는 첫 값. 솔라나 런타임과 같은 규칙.
fn find_program_address(seeds: &[&[u8]], program: &[u8; 32]) -> Option<([u8; 32], u8)> {
    for bump in (0u8..=255).rev() {
        let mut h = Sha256::new();
        for s in seeds {
            h.update(s);
        }
        h.update([bump]);
        h.update(program);
        h.update(b"ProgramDerivedAddress");
        let out: [u8; 32] = h.finalize().into();
        if !is_on_curve(&out) {
            return Some((out, bump));
        }
    }
    None
}

/// 지갑 `owner` 의 `mint` 연결 토큰 계정(ATA) 주소. 시드 = [owner, 토큰 프로그램, mint].
/// 입력이 base58 32바이트가 아니면 None.
pub fn associated_token_address(owner: &str, mint: &str) -> Option<String> {
    let owner = parse_pubkey(owner)?;
    let mint = parse_pubkey(mint)?;
    let token = parse_pubkey(TOKEN_PROGRAM_ID)?;
    let ata_prog = parse_pubkey(ASSOCIATED_TOKEN_PROGRAM_ID)?;
    find_program_address(&[&owner, &token, &mint], &ata_prog).map(|(a, _)| encode_pubkey(&a))
}

// ── 단위 ─────────────────────────────────────────────────────────────────────────────────

/// 정수 base unit → 십진 문자열, 소수 자리를 `decimals` 만큼 채운다(`1500000`, 6 → `"1.500000"`).
/// alloy `format_units` 와 같은 모양 — 화면·MCP 가 이 모양을 다듬는다.
pub fn format_units(raw: u64, decimals: u8) -> String {
    if decimals == 0 {
        return raw.to_string();
    }
    let d = decimals as u32;
    let base = 10u128.pow(d);
    let raw = raw as u128;
    format!("{}.{:0width$}", raw / base, raw % base, width = d as usize)
}

// ── JSON-RPC ──────────────────────────────────────────────────────────────────────────────

/// RPC 한 번의 실패. 문구는 영어 원문 그대로(RPC 가 준 것) — 사람용 문장은 각 크레이트가 감싼다.
/// 🔴 `Http` 의 문자열엔 URL(지정 RPC 의 API 키 포함 가능)이 들어 있을 수 있다 → 내보내기 전에 `redact_urls`.
#[derive(Debug)]
pub enum RpcError {
    Http(String),
    Rpc { code: i64, message: String },
    Shape(String),
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RpcError::Http(e) => write!(f, "{e}"),
            RpcError::Rpc { code, message } => write!(f, "RPC {code}: {message}"),
            RpcError::Shape(e) => write!(f, "unexpected RPC response: {e}"),
        }
    }
}

/// JSON-RPC 2.0 호출 하나 → `result`.
pub async fn rpc_call(
    client: &reqwest::Client,
    url: &str,
    method: &str,
    params: Value,
) -> Result<Value, RpcError> {
    let body = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let resp = client
        .post(url)
        .json(&body)
        .send()
        .await
        .map_err(|e| RpcError::Http(e.to_string()))?;
    let status = resp.status();
    let v: Value = resp
        .json()
        .await
        .map_err(|e| RpcError::Http(format!("HTTP {status}: {e}")))?;
    parse_rpc_response(v)
}

/// 응답 본문 → `result` 또는 오류 (순수 — 테스트가 직접 부른다).
fn parse_rpc_response(mut v: Value) -> Result<Value, RpcError> {
    if let Some(err) = v.get("error") {
        return Err(RpcError::Rpc {
            code: err.get("code").and_then(Value::as_i64).unwrap_or(0),
            message: err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        });
    }
    match v.get_mut("result") {
        Some(r) => Ok(r.take()),
        None => Err(RpcError::Shape("no result".into())),
    }
}

/// `getBalance` 결과(`{context, value}`) → lamports.
fn lamports_from(result: &Value) -> Result<u64, RpcError> {
    result
        .get("value")
        .and_then(Value::as_u64)
        .ok_or_else(|| RpcError::Shape("getBalance value".into()))
}

/// ATA 의 `getAccountInfo`(jsonParsed) 결과 → 토큰 base unit.
///
/// - `value: null` = 계정이 아직 없다 = **0**.
/// - 있으면 **그 계정이 정말 이 지갑·이 민트·기존 토큰 프로그램 것인지** 본다. ATA 주소는 셋에서
///   파생되니 정상이면 늘 맞는다 — 안 맞으면 RPC 가 엉뚱한 답을 준 것이라 0 으로 덮지 않고 오류로 낸다.
/// - decimals 가 기대값과 다르면 오류(금액이 10^n 배 어긋난 화면보다 「못 읽음」이 낫다).
///
/// `getTokenAccountBalance` 를 쓰지 않은 이유: 계정이 없으면 오류(-32602 "could not find account")로 답해
/// 「없음」과 「진짜 오류」를 문구 매칭으로 갈라야 한다. 여기는 null 하나로 갈린다.
fn token_units_from(
    result: &Value,
    owner: &str,
    mint: &str,
    decimals: u8,
) -> Result<u64, RpcError> {
    let value = result
        .get("value")
        .ok_or_else(|| RpcError::Shape("getAccountInfo value".into()))?;
    if value.is_null() {
        return Ok(0);
    }
    if value.get("owner").and_then(Value::as_str) != Some(TOKEN_PROGRAM_ID) {
        return Err(RpcError::Shape("token account is not owned by the SPL Token program".into()));
    }
    let info = value
        .pointer("/data/parsed/info")
        .ok_or_else(|| RpcError::Shape("token account is not jsonParsed".into()))?;
    if info.get("mint").and_then(Value::as_str) != Some(mint) {
        return Err(RpcError::Shape("token account mint mismatch".into()));
    }
    if info.get("owner").and_then(Value::as_str) != Some(owner) {
        return Err(RpcError::Shape("token account owner mismatch".into()));
    }
    let amt = info
        .get("tokenAmount")
        .ok_or_else(|| RpcError::Shape("tokenAmount".into()))?;
    if amt.get("decimals").and_then(Value::as_u64) != Some(decimals as u64) {
        return Err(RpcError::Shape("token decimals mismatch".into()));
    }
    amt.get("amount")
        .and_then(Value::as_str)
        .and_then(|s| s.parse::<u64>().ok())
        .ok_or_else(|| RpcError::Shape("tokenAmount.amount".into()))
}

/// 한 클러스터의 잔액 조회에 필요한 값 (두 크레이트의 체인 설정에서 채운다).
pub struct BalanceQuery<'a> {
    pub rpc_url: &'a str,
    pub owner: &'a str,
    pub usdc_mint: &'a str,
    pub usdc_decimals: u8,
    pub genesis_hash: &'a str,
}

/// 잔액 원시값 (lamports · USDC base unit).
#[derive(Debug, PartialEq)]
pub struct RawBalances {
    pub lamports: u64,
    pub usdc_units: u64,
}

/// 잔액 조회 실패의 갈래 — 각 크레이트가 갈래마다 사람 말로 바꾼다.
#[derive(Debug)]
pub enum BalanceError {
    /// 주소가 base58 32바이트가 아니다.
    BadAddress,
    /// 지정 RPC 가 다른 클러스터다 (`getGenesisHash` 가 다름). 값은 RPC 가 준 해시.
    WrongCluster(String),
    Genesis(RpcError),
    Sol(RpcError),
    Usdc(RpcError),
}

/// SOL + USDC 잔액. 세 호출(클러스터 확인 · SOL · USDC ATA)을 **동시에** 보낸다.
///
/// 🔴 클러스터 확인을 빼지 않는다 — 데브넷을 고르고 메인넷 RPC 를 적으면 조회가 「성공」하며 남의 클러스터 잔액을
/// 이 클러스터 것으로 보인다.
pub async fn fetch_balances(
    client: &reqwest::Client,
    q: &BalanceQuery<'_>,
) -> Result<RawBalances, BalanceError> {
    if parse_pubkey(q.owner).is_none() {
        return Err(BalanceError::BadAddress);
    }
    let ata = associated_token_address(q.owner, q.usdc_mint).ok_or(BalanceError::BadAddress)?;
    let cfg = json!({"commitment": COMMITMENT});
    let (genesis, sol, usdc) = tokio::join!(
        rpc_call(client, q.rpc_url, "getGenesisHash", json!([])),
        rpc_call(client, q.rpc_url, "getBalance", json!([q.owner, cfg])),
        rpc_call(
            client,
            q.rpc_url,
            "getAccountInfo",
            json!([ata, {"commitment": COMMITMENT, "encoding": "jsonParsed"}]),
        ),
    );
    let genesis = genesis.map_err(BalanceError::Genesis)?;
    let got = genesis
        .as_str()
        .ok_or_else(|| BalanceError::Genesis(RpcError::Shape("genesis hash".into())))?;
    if got != q.genesis_hash {
        return Err(BalanceError::WrongCluster(got.to_string()));
    }
    let lamports = sol.and_then(|v| lamports_from(&v)).map_err(BalanceError::Sol)?;
    let usdc_units = usdc
        .and_then(|v| token_units_from(&v, q.owner, q.usdc_mint, q.usdc_decimals))
        .map_err(BalanceError::Usdc)?;
    Ok(RawBalances {
        lamports,
        usdc_units,
    })
}

/// 결제 트랜잭션에 넣을 최신 블록해시와 그 마지막 유효 블록 높이.
#[derive(Debug, Clone, PartialEq)]
pub struct LatestBlockhash {
    pub blockhash: [u8; 32],
    pub last_valid_block_height: u64,
    /// 응답의 `context.slot` — 정산 확정이 내 계정 서명 목록을 **이 컴퓨터 시계 대신 체인 기준**으로 끊는 데 쓴다.
    /// 없으면 0(그때 확인은 목록을 끝까지 봐야만 「없음」 을 말한다).
    pub slot: u64,
}

/// `getLatestBlockhash` — 클러스터 확인(`getGenesisHash`)과 **동시에** 보낸다. 다른 클러스터의 블록해시로 서명하면
/// 정산이 실패할 뿐 돈은 안 나가지만, 지정 RPC 가 딴 클러스터인 걸 결제 순간에 정직하게 말하려고 같이 본다.
/// 오류 문자열엔 URL 이 들어 있을 수 있다 — 내보내기 전에 `redact_urls`.
pub async fn fetch_latest_blockhash(
    client: &reqwest::Client,
    rpc_url: &str,
    genesis_hash: &str,
) -> Result<LatestBlockhash, String> {
    let (genesis, latest) = tokio::join!(
        rpc_call(client, rpc_url, "getGenesisHash", json!([])),
        rpc_call(client, rpc_url, "getLatestBlockhash", json!([{"commitment": COMMITMENT}])),
    );
    let genesis = genesis.map_err(|e| e.to_string())?;
    if genesis.as_str() != Some(genesis_hash) {
        return Err(format!(
            "RPC is on a different cluster ({})",
            genesis.as_str().and_then(cluster_name_of).unwrap_or("unknown")
        ));
    }
    latest_blockhash_from(&latest.map_err(|e| e.to_string())?)
}

/// `getLatestBlockhash` 결과 → 값 (순수 — 테스트가 직접 부른다).
fn latest_blockhash_from(v: &Value) -> Result<LatestBlockhash, String> {
    let val = v.get("value").ok_or("getLatestBlockhash: no value")?;
    let blockhash = val
        .get("blockhash")
        .and_then(Value::as_str)
        .and_then(parse_pubkey)
        .ok_or("getLatestBlockhash: bad blockhash")?;
    let last_valid_block_height = val
        .get("lastValidBlockHeight")
        .and_then(Value::as_u64)
        .ok_or("getLatestBlockhash: bad lastValidBlockHeight")?;
    let slot = v.pointer("/context/slot").and_then(Value::as_u64).unwrap_or(0);
    Ok(LatestBlockhash { blockhash, last_valid_block_height, slot })
}

/// 받는 쪽(`owner`)의 `mint` 토큰 계정(ATA)이 체인에 있나.
/// 없으면 TransferChecked 가 실패해 정산이 안 될 결제를 사람 앞에 띄우는 셈이다. 있는데 모양이 이상하면(남의 민트·
/// Token-2022·decimals 다름) 오류 — 우리가 만들 송금이 그 계정에 들어갈 수 없다. 클러스터 확인을 같이 한다.
#[allow(dead_code)] // MCP 만 부른다 (GUI 빌드에선 안 쓰임)
pub async fn token_account_exists(
    client: &reqwest::Client,
    rpc_url: &str,
    genesis_hash: &str,
    owner: &str,
    mint: &str,
    decimals: u8,
) -> Result<bool, String> {
    let ata = associated_token_address(owner, mint).ok_or("bad address")?;
    let (genesis, info) = tokio::join!(
        rpc_call(client, rpc_url, "getGenesisHash", json!([])),
        rpc_call(
            client,
            rpc_url,
            "getAccountInfo",
            json!([ata, {"commitment": COMMITMENT, "encoding": "jsonParsed"}]),
        ),
    );
    let genesis = genesis.map_err(|e| e.to_string())?;
    if genesis.as_str() != Some(genesis_hash) {
        return Err("RPC is on a different cluster".into());
    }
    let info = info.map_err(|e| e.to_string())?;
    if info.get("value").is_some_and(Value::is_null) {
        return Ok(false);
    }
    token_units_from(&info, owner, mint, decimals).map_err(|e| e.to_string())?;
    // 동결(frozen)된 계정은 받지 못한다 — 승인 뒤 정산이 실패하니 창을 띄우기 전에 멈춘다.
    // 모양 검사 **뒤에** 본다(앞에 두면 모양 이상·동결이 「계정 없음」 안내로 가려졌다).
    match info.pointer("/value/data/parsed/info/state").and_then(Value::as_str) {
        Some("initialized") => Ok(true),
        Some(s) => Err(format!("seller token account is {s}")),
        None => Err("token account state missing".into()),
    }
}

/// 64바이트 ed25519 서명의 base58 꼴인가 — 판매자가 준 정산 tx id 를 탐색기 링크로 만들어도 되는지.
#[allow(dead_code)] // MCP 만 부른다
pub fn is_signature(s: &str) -> bool {
    !s.is_empty() && s.len() <= 88 && bs58::decode(s).into_vec().map(|v| v.len() == 64).unwrap_or(false)
}

/// 사람이 보는 클러스터 이름 (WrongCluster 문구용). 모르는 해시는 None.
pub fn cluster_name_of(genesis: &str) -> Option<&'static str> {
    match genesis {
        GENESIS_MAINNET => Some("mainnet-beta"),
        GENESIS_DEVNET => Some("devnet"),
        "4uhcVJyU9pJkvQyS88uRDiswHXSCkY3zQawwpjk2NsNY" => Some("testnet"),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pubkey_roundtrip_and_rejects() {
        let k = parse_pubkey(TOKEN_PROGRAM_ID).unwrap();
        assert_eq!(encode_pubkey(&k), TOKEN_PROGRAM_ID);
        assert_eq!(parse_pubkey(&format!("  {TOKEN_PROGRAM_ID}\n")), Some(k));
        assert!(parse_pubkey("").is_none());
        assert!(parse_pubkey("0x036CbD53842c5426634e7929541eC2318f3dCF7e").is_none()); // EVM 주소
        assert!(parse_pubkey("1111").is_none()); // 길이 틀림
        assert!(parse_pubkey("Tokenkeg0").is_none()); // base58 에 없는 글자 0
    }

    #[test]
    fn ata_is_off_curve_and_deterministic() {
        let owner = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
        let a = associated_token_address(owner, USDC_MINT_DEVNET).unwrap();
        assert_eq!(a, associated_token_address(owner, USDC_MINT_DEVNET).unwrap());
        assert!(!is_on_curve(&parse_pubkey(&a).unwrap()));
        assert_ne!(a, associated_token_address(owner, USDC_MINT_MAINNET).unwrap());
        assert!(associated_token_address("nope", USDC_MINT_DEVNET).is_none());
    }

    #[test]
    fn units_format() {
        assert_eq!(format_units(0, 6), "0.000000");
        assert_eq!(format_units(1_500_000, 6), "1.500000");
        assert_eq!(format_units(10_000, 6), "0.010000");
        assert_eq!(format_units(1_000_000_000, 9), "1.000000000");
        assert_eq!(format_units(u64::MAX, 9), "18446744073.709551615");
        assert_eq!(format_units(7, 0), "7");
    }

    #[test]
    fn rpc_response_parsing() {
        assert_eq!(
            parse_rpc_response(json!({"jsonrpc":"2.0","id":1,"result":"abc"})).unwrap(),
            json!("abc")
        );
        match parse_rpc_response(
            json!({"jsonrpc":"2.0","id":1,"error":{"code":-32602,"message":"bad"}}),
        ) {
            Err(RpcError::Rpc { code, message }) => {
                assert_eq!(code, -32602);
                assert_eq!(message, "bad");
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            parse_rpc_response(json!({"jsonrpc":"2.0","id":1})),
            Err(RpcError::Shape(_))
        ));
        assert_eq!(
            lamports_from(&json!({"context":{"slot":1},"value":2_039_280})).unwrap(),
            2_039_280
        );
        assert!(lamports_from(&json!({"value":"x"})).is_err());
    }

    fn token_account(owner: &str, mint: &str, prog: &str, dec: u64, amount: &str) -> Value {
        json!({"context":{"slot":1},"value":{
            "owner": prog, "lamports": 2_039_280, "executable": false,
            "data": {"program":"spl-token","space":165,"parsed":{"type":"account","info":{
                "mint": mint, "owner": owner, "state":"initialized", "isNative": false,
                "tokenAmount": {"amount": amount, "decimals": dec, "uiAmountString":"x"}
            }}}
        }})
    }

    #[test]
    fn token_balance_null_is_zero_and_checks_fields() {
        let o = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
        let m = USDC_MINT_DEVNET;
        assert_eq!(
            token_units_from(&json!({"context":{"slot":1},"value":null}), o, m, 6).unwrap(),
            0
        );
        assert_eq!(
            token_units_from(&token_account(o, m, TOKEN_PROGRAM_ID, 6, "12000000"), o, m, 6)
                .unwrap(),
            12_000_000
        );
        // 남의 계정·딴 민트·Token-2022·decimals 틀림·amount 깨짐 = 0 이 아니라 오류
        assert!(token_units_from(&token_account(m, m, TOKEN_PROGRAM_ID, 6, "1"), o, m, 6).is_err());
        assert!(token_units_from(&token_account(o, o, TOKEN_PROGRAM_ID, 6, "1"), o, m, 6).is_err());
        assert!(token_units_from(
            &token_account(o, m, "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb", 6, "1"),
            o,
            m,
            6
        )
        .is_err());
        assert!(token_units_from(&token_account(o, m, TOKEN_PROGRAM_ID, 9, "1"), o, m, 6).is_err());
        assert!(token_units_from(&token_account(o, m, TOKEN_PROGRAM_ID, 6, "-1"), o, m, 6).is_err());
        assert!(token_units_from(&json!({"context":{}}), o, m, 6).is_err());
    }

    // ATA 정답지 — 데브넷 USDC 실거래(getTransaction 의 postTokenBalances: 소유자 → 토큰 계정)에서 떠 온 쌍.
    // 우리 PDA 계산이 체인 위 실제 ATA 와 같아야 잔액 조회(지금)와 TransferChecked 받는 계정이 맞는다.
    #[test]
    fn ata_matches_devnet_onchain_accounts() {
        for (owner, ata) in [
            ("AYG63YgrKLbp9B23ntcRemU8kSD7rZ7cNFGDo8DbEfTd", "2cDia1Ga9XDBGWVyFmaqiE7FKMDEtcy3xoRCW7B7S4es"),
            ("FLUKaShMGJz2E3yy2kqqvQL3Sk1dWEVGRCa8fVRy93QX", "9k5eMQU6sGaeSBCubw6ykezVFp2stSSnUsJ7YaRPqUkA"),
            ("8sh86hmWL4ka7U44dFn3U72ZagLsAME4iRMwajfgR8QT", "8CvwyW7amb4MB547dqWh633vsKPTiQrmDsKxn3p2Jcn3"),
        ] {
            assert_eq!(associated_token_address(owner, USDC_MINT_DEVNET).unwrap(), ata, "{owner}");
        }
    }

    #[test]
    fn latest_blockhash_parsing_and_signature_shape() {
        let ok = json!({"context":{"slot":1},"value":{"blockhash":"EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k","lastValidBlockHeight":291470237u64}});
        let lb = latest_blockhash_from(&ok).unwrap();
        assert_eq!(lb.last_valid_block_height, 291_470_237);
        assert_eq!(lb.slot, 1);
        assert_eq!(encode_pubkey(&lb.blockhash), "EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k");
        assert!(latest_blockhash_from(&json!({"value":{"blockhash":"x","lastValidBlockHeight":1}})).is_err());
        assert!(latest_blockhash_from(&json!({"value":{"blockhash":"EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k"}})).is_err());
        let sig = bs58::encode([9u8; 64]).into_string();
        assert!(is_signature(&sig));
        assert!(!is_signature(TOKEN_PROGRAM_ID)); // 32바이트 = 주소
        assert!(!is_signature(&format!("0x{}", "ab".repeat(32))));
        assert!(!is_signature("https://evil.example/x"));
        assert!(!is_signature(""));
    }

    #[test]
    fn cluster_names() {
        assert_eq!(cluster_name_of(GENESIS_DEVNET), Some("devnet"));
        assert_eq!(cluster_name_of(GENESIS_MAINNET), Some("mainnet-beta"));
        assert_eq!(cluster_name_of("x"), None);
        // CAIP-2 값 = genesis 앞 32자
        assert_eq!(&GENESIS_DEVNET[..32], "EtWTRABZaYq6iMfeYKouRu166VU2xqa1");
        assert_eq!(&GENESIS_MAINNET[..32], "5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
    }
}
