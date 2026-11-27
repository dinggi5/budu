// 정산 확정.
//
// 솔라나 tx 는 블록해시가 만료되면 영원히 못 들어간다. 그래서 «정산 대기» 는 유한 시간 안에 둘 중 하나로 끝난다:
//   - 체인에 **우리 서명**(tx 의 `signatures[1]`)이 든 tx 가 있다 → 성공이면 「settled · 체인 확인」, 실패면 「reverted」
//   - 그 tx 의 마지막 유효 블록 높이가 **finalized** 된 뒤에도 없다 → 「expired · 안 나감」
// 「안 나감」(expired·reverted)이면 일일 한도를 **기록 하나당 한 번** 돌려주고(`limits::refund_once`), 작업 예산도 돌려준다.
//
// 판매자가 준 정산 tx id 는 믿지 않는다 — 그 tx 의 signatures[1] 이 우리 서명인지 본다. 못 맞추면 내 USDC 계정(ATA)의
// 서명 목록에서 우리 서명이 든 tx 를 찾는다(tx id 는 수수료 지불자의 첫 서명이라 우리 서명으로 바로 못 찾는다).
//
// 🔴 「없음」 은 **finalized 높이가 마지막 유효 높이를 넘은 뒤, finalized 기준으로 목록을 끝까지 본 뒤에만** 말한다.
//    그 높이까지의 블록이 다 확정됐으니 우리 tx 가 들어갔다면 그 목록에 있어야 한다. 목록을 끝까지 못 봤으면(페이지 상한·RPC 오류)
//    판정을 미룬다 — 「나갔는데 안 나갔다고 적고 한도를 돌려주는」 쪽이 위험한 실패다.

use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Mutex;

use crate::chain::{active_chain, with_pinned_chain};
use crate::history::HistoryEntry;
use crate::settings::effective_rpc;
use crate::solana::{self, rpc_call};

/// 확인 주기(초).
const TICK_SECS: u64 = 10;
/// 서명 목록 한 페이지·최대 페이지 — 1,000건 안에서 서명 시각보다 오래된 데까지 못 내려가면 판정을 미룬다.
const PAGE: u64 = 100;
const MAX_PAGES: usize = 10;
/// 블록해시를 받은 slot 보다 이만큼 이른 tx 까지 본다(RPC 노드 사이 slot 차이 — 1차 리뷰 P2: 예전엔 이 컴퓨터 시계로 끊어서
/// 맥 시계가 빠르면 우리 tx 앞에서 목록을 끊고 「만료」 로 적을 수 있었다).
const SLACK_SLOTS: u64 = 300;
/// 만료 판정 여유 — finalized 높이가 마지막 유효 높이를 이만큼 넘은 뒤에만 「없음」 을 말한다(노드 사이 색인 차이).
const EXPIRY_MARGIN: u64 = 32;

/// 체인이 정한 결말.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Verdict {
    /// 우리 서명이 든 tx 가 성공했다(tx = 그 tx id).
    Settled { tx: String },
    /// 들어갔지만 실패했다 — 결제액은 안 나갔다.
    Reverted { tx: String },
    /// 마지막 유효 높이가 확정된 뒤에도 없다 — 영원히 안 나감.
    Expired,
}

/// (commitment, tx id) → (signatures[1], 실패했나). 한 번 본 tx 는 다시 받지 않는다.
/// commitment 를 열쇠에 넣는다 — confirmed 로 본 tx 는 아직 갈래(fork)에서 떨어질 수 있어,
/// 그걸 «만료 · 안 나감» 을 말하는 finalized 판정이 재사용하면 안 된다. finalized 로 본 것은 confirmed 질문에도 쓴다(더 강하다).
type SeenMap = HashMap<(String, String), (String, bool)>;
static SEEN: Mutex<Option<SeenMap>> = Mutex::new(None);

/// 이 commitment 질문에 쓸 수 있는 열쇠들 — 강한 것부터 (순수 — 테스트용).
fn seen_keys(tx: &str, commitment: &str) -> Vec<(String, String)> {
    let mut keys = vec![("finalized".to_string(), tx.to_string())];
    if commitment != "finalized" {
        keys.push((commitment.to_string(), tx.to_string()));
    }
    keys
}

fn seen_get(tx: &str, commitment: &str) -> Option<(String, bool)> {
    let g = SEEN.lock().ok()?;
    let m = g.as_ref()?;
    seen_keys(tx, commitment).iter().find_map(|k| m.get(k).cloned())
}

fn seen_put(tx: &str, commitment: &str, v: (String, bool)) {
    if let Ok(mut g) = SEEN.lock() {
        let m = g.get_or_insert_with(HashMap::new);
        if m.len() > 4096 {
            m.clear();
        }
        m.insert((commitment.to_string(), tx.to_string()), v);
    }
}

/// `getTransaction` 결과에서 (signatures[1], 실패했나). 없거나 모양이 다르면 None (순수 — 테스트용).
/// 서명이 하나뿐인 tx(입금·faucet 등 — 우리 결제 모양이 아님)는 `("", _)` — 「우리 것 아님」 으로 캐시된다(
/// 예전엔 None → 「못 읽음」 이라 그런 tx 가 내 계정에 하나만 있어도 모든 판정이 영원히 보류됐다).
fn second_sig_of(v: &Value) -> Option<(String, bool)> {
    let sigs = v["transaction"]["signatures"].as_array()?;
    let sig = match sigs.get(1) {
        Some(s) => s.as_str()?.to_string(),
        None if !sigs.is_empty() => String::new(),
        None => return None,
    };
    let failed = !v["meta"]["err"].is_null();
    Some((sig, failed))
}

async fn fetch_tx(c: &reqwest::Client, rpc: &str, tx: &str, commitment: &str) -> Result<Option<(String, bool)>, ()> {
    if let Some(v) = seen_get(tx, commitment) {
        return Ok(Some(v));
    }
    let v = rpc_call(
        c,
        rpc,
        "getTransaction",
        json!([tx, {"commitment": commitment, "encoding": "json", "maxSupportedTransactionVersion": 0}]),
    )
    .await
    .map_err(|_| ())?;
    if v.is_null() {
        return Ok(None);
    }
    let got = second_sig_of(&v).ok_or(())?;
    seen_put(tx, commitment, got.clone());
    Ok(Some(got))
}

/// 내 ATA 의 서명 목록 — slot 이 `since_slot` 보다 충분히 이른 tx 가 나올 때까지(또는 목록 끝까지). 끝까지 봤으면 complete=true.
/// `since_slot` 0(모름)이면 목록 끝까지 가야만 complete. 반환 = (tx id 들, complete).
async fn list_ata(
    c: &reqwest::Client,
    rpc: &str,
    ata: &str,
    since_slot: u64,
    commitment: &str,
) -> Result<(Vec<String>, bool), ()> {
    let mut out = Vec::new();
    let mut before: Option<String> = None;
    for _ in 0..MAX_PAGES {
        let mut cfg = json!({"limit": PAGE, "commitment": commitment});
        if let Some(b) = &before {
            cfg["before"] = json!(b);
        }
        let v = rpc_call(c, rpc, "getSignaturesForAddress", json!([ata, cfg]))
            .await
            .map_err(|_| ())?;
        let items = v.as_array().ok_or(())?;
        for it in items {
            let Some(sig) = it["signature"].as_str() else { continue };
            // 블록해시를 받기 충분히 전의 slot 이면 그 뒤(더 오래된 것)는 볼 필요가 없다 — 그 블록해시로 만든 tx 는 그 뒤에만 들어간다.
            if since_slot > 0 && it["slot"].as_u64().is_some_and(|s| s + SLACK_SLOTS < since_slot) {
                return Ok((out, true));
            }
            out.push(sig.to_string());
        }
        if (items.len() as u64) < PAGE {
            return Ok((out, true));
        }
        before = items.last().and_then(|i| i["signature"].as_str()).map(String::from);
    }
    Ok((out, false))
}

/// 목록의 tx 들에서 우리 서명을 찾는다. 하나라도 못 읽으면 Err(판정 보류).
async fn find_ours(
    c: &reqwest::Client,
    rpc: &str,
    txs: &[String],
    ours: &str,
    commitment: &str,
) -> Result<Option<(String, bool)>, ()> {
    for tx in txs {
        match fetch_tx(c, rpc, tx, commitment).await? {
            Some((second, failed)) if second == ours => return Ok(Some((tx.clone(), failed))),
            Some(_) => {}
            // 목록엔 있는데 아직 못 받는 tx — 판정 보류(그게 우리 것일 수 있다).
            None => return Err(()),
        }
    }
    Ok(None)
}

fn verdict_of(tx: String, failed: bool) -> Verdict {
    if failed {
        Verdict::Reverted { tx }
    } else {
        Verdict::Settled { tx }
    }
}

/// 「실패」 판정은 finalized 로 다시 본 뒤에만 — 그 판정에 한도·예산 환불이 붙는다. confirmed 블록이
/// 갈래에서 떨어지고 같은 tx 가 다른 갈래에서 성공하면 돈은 나갔는데 «안 나감» 으로 돌려주게 된다. 성공은 confirmed 로 둔다.
async fn verdict_checked(c: &reqwest::Client, rpc: &str, ours: &str, tx: String, failed: bool, commitment: &str) -> Option<Verdict> {
    if !failed || commitment == "finalized" {
        return Some(verdict_of(tx, failed));
    }
    match fetch_tx(c, rpc, &tx, "finalized").await {
        Ok(Some((second, f))) if second == ours => Some(verdict_of(tx, f)),
        _ => None, // 아직 finalized 아님 — 다음 차례에 다시
    }
}

/// 기록 하나의 결말을 찾는다. None = 아직 모름(다음 차례에 다시).
async fn judge(c: &reqwest::Client, rpc: &str, mint: &str, e: &HistoryEntry, finalized: u64) -> Option<Verdict> {
    let ours = e.detail.as_str();
    // ① 판매자가 준 tx id — 그 tx 의 signatures[1] 이 우리 서명일 때만 믿는다.
    if solana::is_signature(&e.settle_tx) {
        if let Ok(Some((second, failed))) = fetch_tx(c, rpc, &e.settle_tx, "confirmed").await {
            if second == ours {
                // 실패가 finalized 로 아직 확인 안 되면 ② 로 넘긴다 — 여기서 끝내면 갈래에서 떨어진 실패 tx 가 캐시에 남아
                // 만료 판정까지 영영 못 갔다.
                if let Some(v) = verdict_checked(c, rpc, ours, e.settle_tx.clone(), failed, "confirmed").await {
                    return Some(v);
                }
            }
        }
    }
    // ② 내 USDC 계정의 서명 목록에서 우리 서명이 든 tx 를 찾는다.
    let ata = solana::associated_token_address(&e.from, mint)?;
    let expired = finalized > e.valid_until.saturating_add(EXPIRY_MARGIN);
    // 만료가 확정됐으면 finalized 기준으로 끝까지 본다 — 「없음」 을 말하려면 그래야 한다.
    let commitment = if expired { "finalized" } else { "confirmed" };
    let (txs, complete) = list_ata(c, rpc, &ata, e.sign_slot, commitment).await.ok()?;
    match find_ours(c, rpc, &txs, ours, commitment).await {
        Ok(Some((tx, failed))) => verdict_checked(c, rpc, ours, tx, failed, commitment).await,
        Ok(None) if expired && complete => Some(Verdict::Expired),
        _ => None,
    }
}

/// 한 차례 — 활성 체인의 확인할 기록을 다 본다.
pub(crate) async fn tick() {
    let chain = active_chain();
    with_pinned_chain(chain.chain_id, async move {
        let candidates = crate::history::chain_candidates();
        if candidates.is_empty() {
            return;
        }
        let Ok(c) = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
        else {
            return;
        };
        let rpc = effective_rpc();
        // 지정 RPC 가 이 클러스터인지 — 다른 클러스터에서 「없음」 을 보고 만료로 적으면 안 된다.
        match rpc_call(&c, &rpc, "getGenesisHash", json!([])).await {
            Ok(Value::String(g)) if g == chain.genesis_hash => {}
            _ => return,
        }
        let Some(finalized) = rpc_call(&c, &rpc, "getBlockHeight", json!([{"commitment": "finalized"}]))
            .await
            .ok()
            .and_then(|v| v.as_u64())
        else {
            return;
        };
        let mut by_account: HashMap<u32, Vec<(String, Verdict)>> = HashMap::new();
        for (index, e) in &candidates {
            if let Some(v) = judge(&c, &rpc, chain.usdc_mint, e, finalized).await {
                by_account.entry(*index).or_default().push((e.detail.clone(), v));
            }
        }
        let mut changed_any = false;
        for (index, verdicts) in by_account {
            for e in crate::history::apply_chain_verdicts(index, &verdicts) {
                changed_any = true;
                if matches!(e.status.as_str(), "expired" | "reverted") {
                    refund(&e).await;
                }
            }
        }
        if changed_any {
            CHANGED.store(true, std::sync::atomic::Ordering::Relaxed);
        }
    })
    .await;
}

/// 「안 나감」 으로 닫힌 결제의 한도를 기록 하나당 한 번 돌려준다(예약한 날의 장부에만). 작업 예산도 돌려준다.
async fn refund(e: &HistoryEntry) {
    let dec = active_chain().usdc_decimals;
    let Ok(value) = crate::limits::parse_usdc_nonneg(&e.amount, dec) else {
        return;
    };
    let key = if e.id.is_empty() { e.detail.as_str() } else { e.id.as_str() };
    if e.reserved_day != 0 {
        crate::limits::refund_once("USDC", value, e.reserved_day, key).await;
    }
    crate::job::release_signature(&e.detail);
}

/// 확인이 내역을 바꿨다는 표식 — 화면이 1초 폴링에서 가져가 내역·잔액을 다시 읽는다.
static CHANGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// 화면이 묻는다 — 체인 확인이 내역을 바꿨나(묻는 순간 표식을 지운다).
#[tauri::command]
pub(crate) fn take_chain_updates() -> bool {
    CHANGED.swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// 확인 스레드를 띄운다 (setup 에서 한 번). 자기 스레드에서 차례마다 비동기 한 덩이를 끝까지 돌린다.
pub(crate) fn spawn() {
    std::thread::spawn(|| loop {
        tauri::async_runtime::block_on(tick());
        std::thread::sleep(std::time::Duration::from_secs(TICK_SECS));
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    // confirmed 로 본 tx 는 finalized 질문(«만료 · 안 나감» 판정)에 재사용하지 않는다. 반대는 된다.
    #[test]
    fn seen_cache_respects_commitment() {
        let tx = "seen-cache-test-tx";
        seen_put(tx, "confirmed", ("ours".into(), false));
        assert_eq!(seen_get(tx, "confirmed"), Some(("ours".into(), false)));
        assert_eq!(seen_get(tx, "finalized"), None);
        let tx2 = "seen-cache-test-tx2";
        seen_put(tx2, "finalized", ("ours".into(), true));
        assert_eq!(seen_get(tx2, "confirmed"), Some(("ours".into(), true)));
        assert_eq!(seen_get(tx2, "finalized"), Some(("ours".into(), true)));
    }

    #[test]
    fn second_signature_and_failure_are_read() {
        let ok = json!({"transaction": {"signatures": ["FEE", "OURS"]}, "meta": {"err": null}});
        assert_eq!(second_sig_of(&ok), Some(("OURS".into(), false)));
        let bad = json!({"transaction": {"signatures": ["FEE", "OURS"]}, "meta": {"err": {"InstructionError": [2, "Custom"]}}});
        assert_eq!(second_sig_of(&bad), Some(("OURS".into(), true)));
        // 서명이 하나뿐인 tx(입금 등)는 「우리 것 아님」 — 빈 서명으로 읽혀 캐시된다(판정을 막지 않는다).
        assert_eq!(second_sig_of(&json!({"transaction": {"signatures": ["FEE"]}, "meta": {"err": null}})), Some((String::new(), false)));
        assert_eq!(second_sig_of(&json!({"transaction": {}})), None);
    }

    /// 🔴 만료 판정 실물: 서명만 하고 **아무 데도 안 보낸** 결제는 마지막 유효 높이가 확정된 뒤 `Expired` 로 닫힌다.
    /// 그 전에는 None(판정 보류)이어야 한다. 1~2분 걸린다.
    /// `cargo test --lib live_unsent_signature_expires -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_unsent_signature_expires() {
        use crate::policy::SvmTerms;
        let c = reqwest::Client::new();
        let chain = crate::chain::SOLANA_DEVNET;
        let rpc = chain.default_rpc;
        let kp = crate::keys::keypair_for_account(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            0,
        )
        .unwrap();
        let terms = SvmTerms {
            network: crate::x402::caip2_of(chain.genesis_hash),
            asset: solana::USDC_MINT_DEVNET.into(),
            pay_to: "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4".into(),
            amount: "10000".into(),
            fee_payer: "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5".into(),
            memo: None,
        };
        let (sig, lvh, slot) = with_pinned_chain(chain.chain_id, async {
            let mut pay = crate::x402::resolve_terms(&terms, &terms.pay_to, "0.01", kp.public_key()).unwrap();
            let lb = solana::fetch_latest_blockhash(&c, rpc, chain.genesis_hash).await.unwrap();
            pay.blockhash = lb.blockhash;
            let (_tx, sig) = crate::svm_x402::sign_payment(&kp, &pay).unwrap();
            (bs58::encode(sig).into_string(), lb.last_valid_block_height, lb.slot)
        })
        .await;
        let e = HistoryEntry {
            ts: crate::store::now_secs(),
            status: "signed".into(),
            detail: sig.clone(),
            from: kp.address(),
            valid_until: lvh,
            sign_slot: slot,
            amount: "0.01".into(),
            ..Default::default()
        };
        let height = || async {
            rpc_call(&c, rpc, "getBlockHeight", json!([{"commitment": "finalized"}]))
                .await
                .unwrap()
                .as_u64()
                .unwrap()
        };
        let h0 = height().await;
        println!("signed {sig}\nvalid until {lvh}, finalized now {h0}");
        assert_eq!(judge(&c, rpc, solana::USDC_MINT_DEVNET, &e, h0).await, None, "만료 전엔 보류");
        let mut verdict = None;
        for _ in 0..40 {
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            let h = height().await;
            if h > lvh + EXPIRY_MARGIN {
                verdict = judge(&c, rpc, solana::USDC_MINT_DEVNET, &e, h).await;
                println!("finalized {h} > {lvh} → {verdict:?}");
                if verdict.is_some() {
                    break;
                }
            }
        }
        assert_eq!(verdict, Some(Verdict::Expired));
    }

    /// 🔴 데브넷 실물: 판매자 tx id 의 signatures[1] 이 그때 저장한 우리 서명과 같고, 성공으로 읽힌다.
    /// `cargo test --lib live_confirm_reads_devnet_payment -- --ignored --nocapture`
    #[tokio::test]
    #[ignore]
    async fn live_confirm_reads_devnet_payment() {
        let c = reqwest::Client::new();
        let rpc = crate::chain::SOLANA_DEVNET.default_rpc;
        // 개발 4 실결제 — 판매자가 준 tx id.
        let tx = "5fe1Q947rzY4FZqwn5Ztwf2yc9rBzzpm63DstCzH4Rv4QzSHFMMZ1PvHfg3tMPqWCFJoNigjoV4NJTxdRTTfV5ur";
        let got = fetch_tx(&c, rpc, tx, "confirmed").await.unwrap().unwrap();
        println!("signatures[1] = {}  failed = {}", got.0, got.1);
        assert!(!got.1);
        assert!(solana::is_signature(&got.0));
        // 그 서명을 우리 것으로 두고 ATA 목록에서 다시 찾는다(테스트 지갑 = abandon×11 about 계정 0).
        let owner = crate::keys::keypair_for_account(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            0,
        )
        .unwrap()
        .address();
        let ata = solana::associated_token_address(&owner, solana::USDC_MINT_DEVNET).unwrap();
        let (txs, complete) = list_ata(&c, rpc, &ata, 0, "finalized").await.unwrap(); // slot 모름 → 끝까지
        println!("ATA {ata}: {} txs, complete={complete}", txs.len());
        let found = find_ours(&c, rpc, &txs, &got.0, "finalized").await;
        println!("found = {found:?}");
        assert!(matches!(found, Ok(Some((ref t, false))) if t == tx));
    }
}
