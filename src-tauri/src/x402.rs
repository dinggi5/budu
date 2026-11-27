// x402 솔라나 결제 — 정책 껍데기(긴급 잠금 · 단일/일일 한도 예약 · 실패 시 환불 · 내역) + 서명 코어 호출.
//
// 순서: 이 함수에 들어왔다는 건 사람이 이미 승인했거나
// (비번) 자율 승인 세션이 통과시켰다는 뜻이다. 그 **뒤에** 블록해시를 받는다 — 402 를 받자마자 조립하면 사람이 1분
// 고민한 결제는 블록해시(60~90초)가 죽어 정산이 실패한다.
//
//   조건 대조(MCP 를 믿지 않는다) → 잠금 → 한도 예약 → 잠금 재확인 → 블록해시 → 조립·자체 검증·서명(svm_x402) → 먼저 적기 → 반환
//
// 보안: 부분 서명된 트랜잭션도 «내 USDC 를 빼갈 권한» 이다(페이실리테이터가 자기 서명만 더해 낸다) → 서명 시점에
// 누적 사용액에 기록한다(보수적: 서명 = 인출 권한 부여). 정산 확정·만료 환불은 `confirm.rs`.

use crate::i18n::ts;
use zeroize::Zeroizing;

use crate::balance::parse_to_addr;
use crate::chain::{active_chain, with_pinned_chain};
use crate::history::log_attempt;
use crate::keys::SolKeypair;
use crate::limits::{parse_limit_usdc, parse_usdc_nonneg, refund_spend, reserve_spend};
use crate::lock::read_lock;
use crate::policy::{SvmSigned, SvmTerms};
use crate::settings::{effective_rpc, read_settings, redact_urls};
use crate::solana::{self, encode_pubkey, parse_pubkey};
use crate::svm_x402::{self, PayTerms, MAX_MEMO_BYTES};
use crate::trusted::record_trusted;
use crate::wallet::{active_account_index, unlock_signer_keep, with_pinned_account};

/// 활성 클러스터의 CAIP-2 (`solana:` + genesis 앞 32자).
pub(crate) fn caip2_of(genesis: &str) -> String {
    format!("solana:{}", &genesis[..32.min(genesis.len())])
}

/// 조건을 블록해시 없이 해석·대조한다 (순수 — 테스트가 직접 부른다).
///
/// 🔴 MCP 가 고른 조건을 그대로 믿지 않는다 — 요청 파일은 같은 사용자 권한의 아무 프로세스나 쓸 수 있다.
/// 사람이 승인 창에서 본 것은 `to`·`amount`(십진) 이므로 서명할 조건이 그 둘과 **정확히** 같아야 하고,
/// 네트워크·민트는 활성 클러스터의 것이어야 한다.
pub(crate) fn resolve_terms(
    terms: &SvmTerms,
    to: &str,
    amount_usdc: &str,
    owner: [u8; 32],
) -> Result<PayTerms, String> {
    let chain = active_chain();
    let bad = |what: &str| -> String {
        format!(
            "{} ({what})",
            ts!(
                "결제 조건이 이 지갑과 맞지 않아 서명하지 않았어요. 아무것도 결제하지 않았습니다",
                "The payment terms don't match this wallet, so nothing was signed or paid"
            )
        )
    };
    if terms.network != caip2_of(chain.genesis_hash) {
        return Err(bad("network"));
    }
    if terms.asset != chain.usdc_mint {
        return Err(bad("asset"));
    }
    // base58 은 대소문자를 가린다 — 정확히 같아야 같은 주소.
    if terms.pay_to != to.trim() {
        return Err(bad("payTo"));
    }
    let pay_to = parse_pubkey(&terms.pay_to).ok_or_else(|| bad("payTo"))?;
    let fee_payer = parse_pubkey(&terms.fee_payer).ok_or_else(|| bad("feePayer"))?;
    if fee_payer == owner {
        return Err(bad("feePayer"));
    }
    let amount: u64 = terms
        .amount
        .trim()
        .parse()
        .ok()
        .filter(|&a| a > 0 && terms.amount.bytes().all(|b| b.is_ascii_digit()))
        .ok_or_else(|| bad("amount"))?;
    let shown: u128 = parse_usdc_nonneg(amount_usdc.trim(), chain.usdc_decimals)?;
    if shown != u128::from(amount) {
        return Err(bad("amount"));
    }
    let memo = match &terms.memo {
        Some(m) => {
            // 숨은 문자(제어·방향) 검사는 MCP 도 하지만 요청 파일은 아무나 쓴다 — 자율·작업 예산 경로엔 사람 눈이 없다.
            if m.is_empty() || m.len() > MAX_MEMO_BYTES || m.chars().any(crate::policy::is_hidden_char) {
                return Err(bad("memo"));
            }
            m.as_bytes().to_vec()
        }
        None => svm_x402::random_memo(),
    };
    Ok(PayTerms {
        fee_payer,
        owner,
        mint: parse_pubkey(chain.usdc_mint).ok_or_else(|| bad("asset"))?,
        pay_to,
        amount,
        decimals: chain.usdc_decimals,
        memo,
        blockhash: [0u8; 32], // 한도 예약 뒤에 받는다
    })
}

/// x402 결제를 서명한다 — 비번 승인 경로. 서명 성공 + 자동 학습 켬 = 신뢰 주소 학습.
/// **`#[tauri::command]` 가 아니다** — 시작점은 승인(`approve_payment`) 하나뿐이다.
/// `keep_key` = 사람이 작업 예산을 열었다 → 서명에 쓴 12단어를 돌려준다(호출자가 `job.rs` 에 메모리로만 든다).
#[allow(clippy::too_many_arguments)]
pub(crate) async fn sign_x402_payment(
    password: String,
    to: String,
    amount_usdc: String,
    terms: SvmTerms,
    resource: String,
    job: String,
    keep_key: bool,
) -> Result<(SvmSigned, Option<Zeroizing<String>>), String> {
    let password = Zeroizing::new(password);
    // 진입 시 체인·계정을 한 번 고정 — 비번 복호화 사이에 설정이 바뀌어도 조건 대조·내역이 진입 때 것.
    with_pinned_chain(
        active_chain().chain_id,
        with_pinned_account(active_account_index(), async move {
            let (signer, phrase) = match unlock_signer_keep(&password) {
                Ok(s) => s,
                Err(e) => {
                    log_attempt("USDC", to.trim(), amount_usdc.trim(), "failed", &e);
                    return Err(e);
                }
            };
            let to_addr = to.clone();
            let signed = do_sign_x402(&signer, to, amount_usdc, terms, resource, job).await?;
            // 자동 학습은 사용자가 켰을 때만(기본 꺼짐, 개발 14) — 화이트리스트는 직접 넣은 주소가 기본이다.
            if crate::settings::read_settings().auto_learn_trusted {
                record_trusted(&to_addr);
            }
            Ok((signed, keep_key.then_some(phrase)))
        }),
    )
    .await
}

/// x402 서명 코어 — 서명자가 이미 있는 상태에서 실행한다(비번 래퍼와 자율 승인 경로가 공유).
pub(crate) async fn do_sign_x402(
    signer: &SolKeypair,
    to: String,
    amount_usdc: String,
    terms: SvmTerms,
    resource: String,
    job: String,
) -> Result<SvmSigned, String> {
    with_pinned_chain(
        active_chain().chain_id,
        with_pinned_account(
            active_account_index(),
            do_sign_x402_inner(signer, to, amount_usdc, terms, resource, job),
        ),
    )
    .await
}

fn lock_blocked(to: &str, amt: &str) -> String {
    log_attempt(
        "USDC",
        to,
        amt,
        "blocked",
        ts!("긴급 잠금 (x402 서명)", "Emergency lock (x402 signature)"),
    );
    ts!(
        "긴급 잠금이 켜져 있어 결제가 차단됐어요. 해제 후 다시 시도하세요.",
        "Emergency lock is on, so the payment was blocked. Turn it off and try again."
    )
    .into()
}

async fn do_sign_x402_inner(
    signer: &SolKeypair,
    to: String,
    amount_usdc: String,
    terms: SvmTerms,
    resource: String,
    job: String,
) -> Result<SvmSigned, String> {
    let chain = active_chain();
    let dec = chain.usdc_decimals;
    let amt = amount_usdc.trim();
    let value: u128 = parse_usdc_nonneg(amt, dec)?;
    if value == 0 {
        return Err(ts!("0보다 큰 금액이어야 해요", "The amount must be greater than 0").into());
    }
    let to_owned = parse_to_addr(&to)?;
    let to = to_owned.as_str();
    let mut pay = match resolve_terms(&terms, to, amt, signer.public_key()) {
        Ok(p) => p,
        Err(e) => {
            log_attempt("USDC", to, amt, "failed", &e);
            return Err(e);
        }
    };

    // 긴급 잠금: 켜져 있으면 결제를 가장 먼저 차단한다 (비상 스위치).
    if read_lock() {
        return Err(lock_blocked(to, amt));
    }

    // 한도 검사 + 예약. 락은 빠른 파일 I/O 구간만 잡는다. 한도 초과면 여기서 거부.
    let settings = read_settings();
    let single: u128 = parse_limit_usdc(&settings.single_usdc, dec)?;
    let daily: u128 = parse_limit_usdc(&settings.daily_usdc, dec)?;
    let reserved_day = match reserve_spend("USDC", value, single, daily, dec).await {
        Ok(d) => d,
        Err(e) => {
            log_attempt("USDC", to, amt, "blocked", &e);
            return Err(e);
        }
    };
    // 예약까지 오는 사이(한도 잠금 대기) 긴급 잠금이 켜졌으면 서명하지 않는다.
    if read_lock() {
        refund_spend("USDC", value, reserved_day).await;
        return Err(lock_blocked(to, amt));
    }

    // 블록해시 — 승인 **뒤**. 판매자 힌트(`extra.recentBlockhash`)는 쓰지 않고 우리가 직접 받는다:
    // 승인 대기가 힌트의 수명을 넘기기 쉽고, 정산 확정에 쓸 lastValidBlockHeight 를 우리 손으로 가져야 한다.
    let latest = async {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(15))
            .build()
            .map_err(|e| e.to_string())?;
        solana::fetch_latest_blockhash(&client, &effective_rpc(), chain.genesis_hash).await
    }
    .await;
    let latest = match latest {
        Ok(l) => l,
        Err(e) => {
            refund_spend("USDC", value, reserved_day).await;
            let msg = format!(
                "{} ({})",
                ts!(
                    "블록해시를 받지 못해 서명하지 않았어요. 아무것도 결제하지 않았습니다 — 네트워크를 확인하고 다시 시도하세요",
                    "Couldn't get a recent blockhash, so nothing was signed or paid — check the network and try again"
                ),
                redact_urls(&e)
            );
            log_attempt("USDC", to, amt, "failed", &msg);
            return Err(msg);
        }
    };
    pay.blockhash = latest.blockhash;

    // 조립 → 자체 검증 → 서명. 하나라도 어긋나면 서명하지 않는다.
    let (tx, sig) = match svm_x402::sign_payment(signer, &pay) {
        Ok(v) => v,
        Err(e) => {
            refund_spend("USDC", value, reserved_day).await;
            let msg = format!(
                "{} ({e})",
                ts!(
                    "결제 트랜잭션 자체 검증에 실패해 서명하지 않았어요. 아무것도 결제하지 않았습니다",
                    "The payment transaction failed its self-check, so nothing was signed or paid"
                )
            );
            log_attempt("USDC", to, amt, "failed", &msg);
            return Err(msg);
        }
    };
    let signature = bs58::encode(sig).into_string();

    // 🔴 **못 적으면 서명을 내보내지 않는다**. 내보낸 서명은 판매자가 정산할 수 있는 돈인데, 기록이 없으면
    // 다시 켠 뒤 중복 검사가 그 결제를 못 본다. detail = 우리 서명.
    //   `Err(None)`  = 줄이 없다 → 아무 데도 안 나간 서명이니 지금 환불.
    //   `Err(Some)` = 줄은 제자리 「정산 대기」 → 여기서 환불하지 않는다(블록해시 만료 뒤 `confirm.rs` 가 돌려준다).
    let extra = crate::history::RowExtra {
        reserved_day,
        resource: &resource,
        job: &job,
        valid_until: latest.last_valid_block_height,
        sign_slot: latest.slot,
    };
    if let Err(row) = crate::history::log_signed("USDC", to, amt, &signature, extra) {
        if row.is_none() {
            refund_spend("USDC", value, reserved_day).await;
        }
        return Err(ts!(
            "결제 기록을 저장하지 못해 서명을 보내지 않았어요. 아무것도 결제하지 않았습니다 — 디스크 공간을 확인하고 다시 시도하세요.",
            "Couldn't save the payment record, so the signature wasn't sent. Nothing was paid — check disk space and try again."
        )
        .into());
    }

    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    Ok(SvmSigned {
        transaction: B64.encode(&tx),
        signature,
        blockhash: encode_pubkey(&latest.blockhash),
        last_valid_block_height: latest.last_valid_block_height,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAYTO: &str = "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4";
    const FEEPAYER: &str = "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5";

    fn terms() -> SvmTerms {
        SvmTerms {
            network: caip2_of(crate::chain::SOLANA_DEVNET.genesis_hash),
            asset: solana::USDC_MINT_DEVNET.into(),
            pay_to: PAYTO.into(),
            amount: "10000".into(),
            fee_payer: FEEPAYER.into(),
            memo: Some("inv-1".into()),
        }
    }

    async fn on_devnet<F: std::future::Future>(f: F) -> F::Output {
        with_pinned_chain(crate::chain::SOLANA_DEVNET.chain_id, f).await
    }

    #[tokio::test]
    async fn resolve_terms_matches_and_rejects() {
        on_devnet(async {
            let owner = [5u8; 32];
            let p = resolve_terms(&terms(), PAYTO, "0.01", owner).unwrap();
            assert_eq!(p.amount, 10_000);
            assert_eq!(p.memo, b"inv-1");
            assert_eq!(p.decimals, 6);
            // 사람이 본 금액·받는 주소와 하나라도 다르면 거절
            assert!(resolve_terms(&terms(), PAYTO, "0.02", owner).is_err());
            assert!(resolve_terms(&terms(), FEEPAYER, "0.01", owner).is_err());
            // 대소문자만 다른 주소도 다른 주소
            assert!(resolve_terms(&terms(), &PAYTO.to_lowercase(), "0.01", owner).is_err());
            for f in [
                |t: &mut SvmTerms| t.network = "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp".into(),
                |t: &mut SvmTerms| t.asset = solana::USDC_MINT_MAINNET.into(),
                |t: &mut SvmTerms| t.fee_payer = "nope".into(),
                |t: &mut SvmTerms| t.amount = "0".into(),
                |t: &mut SvmTerms| t.amount = "+10000".into(),
                |t: &mut SvmTerms| t.amount = "1e4".into(),
                |t: &mut SvmTerms| t.memo = Some(String::new()),
                |t: &mut SvmTerms| t.memo = Some("a".repeat(MAX_MEMO_BYTES + 1)),
                |t: &mut SvmTerms| t.memo = Some("inv\u{202E}1".into()), // 방향 문자
                |t: &mut SvmTerms| t.memo = Some("inv\n1".into()),       // 제어 문자
            ] {
                let mut t = terms();
                f(&mut t);
                assert!(resolve_terms(&t, PAYTO, "0.01", owner).is_err());
            }
            // 수수료 지불자가 나 자신이면 거절
            let me = parse_pubkey(FEEPAYER).unwrap();
            assert!(resolve_terms(&terms(), PAYTO, "0.01", me).is_err());
            // memo 없으면 난수 hex 32자
            let mut t = terms();
            t.memo = None;
            assert_eq!(resolve_terms(&t, PAYTO, "0.01", owner).unwrap().memo.len(), 32);
        })
        .await;
    }

    /// 🔴 **데브넷 실결제**. 네트워크·데모 판매자가 필요해 무시해 둔다:
    ///   `cd budu-port && npm start` 를 띄운 뒤
    ///   `cargo test --lib live_devnet_x402_payment -- --ignored --nocapture`
    /// 지갑 = 공개 테스트 니모닉(abandon ×11 about)의 첫 계정(데브넷 USDC 0.24 남짓). ~/.budu 를 건드리지 않으려고 정책 껍데기
    /// (한도·내역 파일) 대신 같은 순수 함수들(resolve_terms → 블록해시 → sign_payment)을 그대로 부른다.
    #[tokio::test]
    #[ignore]
    async fn live_devnet_x402_payment() {
        use base64::{engine::general_purpose::STANDARD as B64, Engine};
        use serde_json::{json, Value};
        let url = std::env::var("BUDU_PORT_URL").unwrap_or("http://localhost:4021/wave".into());
        let http = reqwest::Client::new();
        let r = http.get(&url).send().await.unwrap();
        assert_eq!(r.status().as_u16(), 402);
        let pr: Value = serde_json::from_slice(
            &B64.decode(r.headers()["payment-required"].to_str().unwrap()).unwrap(),
        )
        .unwrap();
        let req = pr["accepts"][0].clone();
        let terms = SvmTerms {
            network: req["network"].as_str().unwrap().into(),
            asset: req["asset"].as_str().unwrap().into(),
            pay_to: req["payTo"].as_str().unwrap().into(),
            amount: req["amount"].as_str().unwrap().into(),
            fee_payer: req["extra"]["feePayer"].as_str().unwrap().into(),
            memo: None,
        };
        let kp = crate::keys::keypair_for_account(
            "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about",
            0,
        )
        .unwrap();
        let signed_tx = on_devnet(async {
            let chain = active_chain();
            let mut pay = resolve_terms(&terms, &terms.pay_to, "0.01", kp.public_key()).unwrap();
            let lb = solana::fetch_latest_blockhash(&http, chain.default_rpc, chain.genesis_hash)
                .await
                .unwrap();
            pay.blockhash = lb.blockhash;
            svm_x402::sign_payment(&kp, &pay).unwrap()
        })
        .await;
        let (tx, sig) = signed_tx;
        let ours = bs58::encode(sig).into_string();
        let envelope = json!({"x402Version":2,"resource":pr["resource"],"accepted":req,
            "payload":{"transaction":B64.encode(&tx)}});
        let paid = http
            .get(&url)
            .header("PAYMENT-SIGNATURE", B64.encode(envelope.to_string()))
            .send()
            .await
            .unwrap();
        let status = paid.status().as_u16();
        let settle = paid
            .headers()
            .get("payment-response")
            .map(|h| String::from_utf8(B64.decode(h.to_str().unwrap()).unwrap()).unwrap());
        let body = paid.text().await.unwrap();
        println!("HTTP {status}\nPAYMENT-RESPONSE {settle:?}\nbody {body}\nours {ours}");
        assert_eq!(status, 200, "{body}");
        let settle: Value = serde_json::from_str(&settle.unwrap()).unwrap();
        assert_eq!(settle["success"], true);
        let txid = settle["transaction"].as_str().unwrap().to_string();
        // 판매자가 준 tx id 를 믿지 않고 체인에서 그 tx 의 signatures[1] 이 우리 서명인지 본다.
        let mut found = Value::Null;
        for _ in 0..20 {
            let v = solana::rpc_call(
                &http,
                crate::chain::SOLANA_DEVNET.default_rpc,
                "getTransaction",
                json!([txid, {"commitment":"confirmed","encoding":"json","maxSupportedTransactionVersion":0}]),
            )
            .await
            .unwrap();
            if !v.is_null() {
                found = v;
                break;
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }
        println!("explorer https://explorer.solana.com/tx/{txid}?cluster=devnet");
        assert_eq!(found["transaction"]["signatures"][1], json!(ours));
        assert_eq!(found["meta"]["err"], Value::Null);
    }

    #[test]
    fn caip2_is_genesis_prefix() {
        assert_eq!(caip2_of(solana::GENESIS_DEVNET), "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1");
        assert_eq!(caip2_of(solana::GENESIS_MAINNET), "solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp");
    }
}
