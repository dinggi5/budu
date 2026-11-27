use super::*;
use crate::chain::{SOLANA_DEVNET, SOLANA_MAINNET};
use serde_json::json;

const PAYTO: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const FEEPAYER: &str = "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5";

fn good() -> Value {
    json!({"scheme":"exact","network":"solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1","amount":"10000",
        "asset":"4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU","payTo":PAYTO,"maxTimeoutSeconds":60,
        "extra":{"feePayer":FEEPAYER}})
}

fn pr_of(accepts: Vec<Value>) -> PaymentRequired {
    let raw = json!({"x402Version":2,"resource":{"url":"https://x/wave"},"accepts":accepts});
    parse_required(Some(&B64.encode(raw.to_string())), "").unwrap()
}

#[test]
fn tx_hash_shape_is_solana_signature() {
    let sig = bs58::encode([7u8; 64]).into_string();
    assert!(is_tx_hash(&sig));
    assert!(!is_tx_hash(&format!("0x{}", "aB".repeat(32))));
    assert!(!is_tx_hash(PAYTO)); // 32바이트 = 주소
    assert!(!is_tx_hash("https://evil.example/0x00"));
}

#[test]
fn settlement_failure_is_read_without_tx() {
    let enc = |j: &str| B64.encode(j);
    assert!(settlement_says_failed(&enc(r#"{"success":false}"#)));
    assert!(!settlement_says_failed(&enc(r#"{"success":true,"transaction":"x"}"#)));
    assert!(!settlement_says_failed(&enc(r#"{"transaction":"x"}"#)));
    assert!(!settlement_says_failed(""));
    assert!(!settlement_says_failed("%%%"));
}

#[test]
fn clip_strips_controls_and_bidi_and_counts_after_filtering() {
    assert_eq!(clip("solana:abc"), "solana:abc");
    assert_eq!(clip("a\u{202E}b\nc"), "abc");
    let long: String = "가".repeat(70);
    assert_eq!(clip(&long), format!("{}…", "가".repeat(64)));
    assert_eq!(clip(&format!("{}\u{7}", "x".repeat(64))), "x".repeat(64));
}

/// 정상 솔라나 데브넷 요구 → 결제 조건이 그대로 GUI 몫으로 옮겨진다.
#[test]
fn picks_devnet_usdc_exact() {
    let mut e = good();
    e["extra"]["memo"] = json!("inv-7");
    e["extra"]["transactionVersions"] = json!([0, 1]);
    e["extra"]["recentBlockhash"] = json!("EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k");
    let r = pick_requirement_for(&pr_of(vec![e]), &SOLANA_DEVNET).unwrap();
    assert_eq!(r.amount, "10000");
    assert_eq!(r.terms.pay_to, PAYTO);
    assert_eq!(r.terms.fee_payer, FEEPAYER);
    assert_eq!(r.terms.memo.as_deref(), Some("inv-7"));
    assert_eq!(r.terms.asset, SOLANA_DEVNET.usdc_mint);
    assert_eq!(r.terms.network, SOLANA_DEVNET.x402_network_caip2);
}

/// EVM 요구가 먼저 와도 건너뛰고 솔라나를 고른다.
#[test]
fn skips_other_chains_and_picks_first_valid() {
    let evm = json!({"scheme":"exact","network":"eip155:84532","amount":"10000",
        "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e","payTo":"0x209693Bc6afc0C5328bA36FaF03C514EF312287C",
        "extra":{"name":"USDC","version":"2"}});
    let r = pick_requirement_for(&pr_of(vec![evm, good()]), &SOLANA_DEVNET).unwrap();
    assert_eq!(r.pay_to, PAYTO);
}

/// 🔴 닫힌 쪽 — x402 svm 스펙의 받는 쪽 조건 하나하나.
#[test]
fn rejects_everything_outside_the_fence() {
    type Mutate = Box<dyn Fn(&mut Value)>;
    let cases: Vec<(&str, Mutate)> = vec![
        ("scheme", Box::new(|e| e["scheme"] = json!("upto"))),
        ("network mainnet", Box::new(|e| e["network"] = json!("solana:5eykt4UsFv8P8NJdTREpY1vzqKqZKvdp"))),
        ("asset mainnet", Box::new(|e| e["asset"] = json!(SOLANA_MAINNET.usdc_mint))),
        ("asset lower", Box::new(|e| e["asset"] = json!("4zmmc9srt5ri5x14ggxhaHii3GnPAEERYPJgZJDncDU"))),
        ("amount 0", Box::new(|e| e["amount"] = json!("0"))),
        ("amount num", Box::new(|e| e["amount"] = json!(10000))),
        ("amount lead0", Box::new(|e| e["amount"] = json!("010000"))),
        ("amount neg", Box::new(|e| e["amount"] = json!("-1"))),
        ("amount dec", Box::new(|e| e["amount"] = json!("0.01"))),
        ("amount overflow", Box::new(|e| e["amount"] = json!("18446744073709551616"))),
        ("payTo evm", Box::new(|e| e["payTo"] = json!("0x209693Bc6afc0C5328bA36FaF03C514EF312287C"))),
        ("payTo space", Box::new(|e| e["payTo"] = json!(format!(" {PAYTO}")))),
        ("no extra", Box::new(|e| { e.as_object_mut().unwrap().remove("extra"); })),
        ("no feePayer", Box::new(|e| e["extra"] = json!({}))),
        ("bad feePayer", Box::new(|e| e["extra"]["feePayer"] = json!("nope"))),
        ("memo long", Box::new(|e| e["extra"]["memo"] = json!("a".repeat(257)))),
        ("memo empty", Box::new(|e| e["extra"]["memo"] = json!(""))),
        ("memo num", Box::new(|e| e["extra"]["memo"] = json!(5))),
        ("memo bidi", Box::new(|e| e["extra"]["memo"] = json!("inv\u{202E}7"))),
        ("memo newline", Box::new(|e| e["extra"]["memo"] = json!("a\nb"))),
        ("memo alm", Box::new(|e| e["extra"]["memo"] = json!("a\u{061C}b"))),
        ("memo tag", Box::new(|e| e["extra"]["memo"] = json!("a\u{E0041}b"))),
        ("versions 1 only", Box::new(|e| e["extra"]["transactionVersions"] = json!([1]))),
        ("versions string", Box::new(|e| e["extra"]["transactionVersions"] = json!(["0"]))),
        ("versions not array", Box::new(|e| e["extra"]["transactionVersions"] = json!(0))),
        ("splits", Box::new(|e| e["extra"]["splits"] = json!([{"payTo":FEEPAYER,"amount":"1"}]))),
        ("ata creation", Box::new(|e| e["extra"]["ataCreationRequired"] = json!(true))),
        ("ata creation str", Box::new(|e| e["extra"]["ataCreationRequired"] = json!("false"))),
        ("unknown extra", Box::new(|e| e["extra"]["escrowAfter"] = json!(3600))),
        // 요구 최상위의 모르는 칸·유효 시간
        ("unknown top", Box::new(|e| e["refundTo"] = json!(FEEPAYER))),
        ("no timeout", Box::new(|e| { e.as_object_mut().unwrap().remove("maxTimeoutSeconds"); })),
        ("timeout 0", Box::new(|e| e["maxTimeoutSeconds"] = json!(0))),
        ("timeout neg", Box::new(|e| e["maxTimeoutSeconds"] = json!(-5))),
        ("timeout frac", Box::new(|e| e["maxTimeoutSeconds"] = json!(1.5))),
        ("timeout str", Box::new(|e| e["maxTimeoutSeconds"] = json!("60"))),
    ];
    for (name, f) in cases {
        let mut e = good();
        f(&mut e);
        let r = pick_requirement_for(&pr_of(vec![e]), &SOLANA_DEVNET);
        assert!(r.is_err(), "통과하면 안 된다: {name}");
        let msg = r.err().unwrap();
        assert!(msg.contains("Nothing was paid") || msg.contains("결제하지 않았습니다"), "{name}: {msg}");
    }
    // 64자 넘는 평범한 메모는 통과(256바이트까지)
    let mut e = good();
    e["extra"]["memo"] = json!("x".repeat(200));
    assert!(pick_requirement_for(&pr_of(vec![e]), &SOLANA_DEVNET).is_ok());
    // 빈 splits·false 플래그·null 은 «없음» 과 같다
    let mut e = good();
    e["extra"]["splits"] = json!([]);
    e["extra"]["ataCreationRequired"] = json!(false);
    assert!(pick_requirement_for(&pr_of(vec![e]), &SOLANA_DEVNET).is_ok());
    // 메인넷을 고른 지갑은 데브넷 요구를 거절
    assert!(pick_requirement_for(&pr_of(vec![good()]), &SOLANA_MAINNET).is_err());
}

/// 설명 칸(V1 시절 이름)은 결제와 상관없어 통과, 유효 시간은 그대로 옮겨진다.
#[test]
fn descriptive_top_fields_pass_and_timeout_is_kept() {
    let mut e = good();
    e["description"] = json!("파고");
    e["mimeType"] = json!("application/json");
    let r = pick_requirement_for(&pr_of(vec![e]), &SOLANA_DEVNET).unwrap();
    assert_eq!(r.max_timeout_secs, 60);
}

/// 다시 받은 402 대조: 힌트만 바뀌면 같은 요구(새 원본을 돌려줌), 금액·받는 곳·메모가 바뀌면 없음.
#[test]
fn same_offer_ignores_only_blockhash_hints() {
    let mut picked = good();
    picked["extra"]["recentBlockhash"] = json!("old");
    let mut fresh = good();
    fresh["extra"]["recentBlockhash"] = json!("new");
    fresh["extra"]["lastValidBlockHeight"] = json!(99);
    let found = find_same_offer(&pr_of(vec![fresh.clone()]), &picked).unwrap();
    assert_eq!(found, fresh);
    for f in [
        |e: &mut Value| e["amount"] = json!("20000"),
        |e: &mut Value| e["payTo"] = json!(FEEPAYER),
        |e: &mut Value| e["extra"]["memo"] = json!("other"),
        |e: &mut Value| e["extra"]["feePayer"] = json!(PAYTO),
        |e: &mut Value| e["maxTimeoutSeconds"] = json!(30),
    ] {
        let mut changed = good();
        f(&mut changed);
        assert!(find_same_offer(&pr_of(vec![changed]), &picked).is_none());
    }
    // V1 응답은 대조하지 않는다
    let v1 = parse_required(None, &json!({"x402Version":1,"accepts":[good()]}).to_string()).unwrap();
    assert!(find_same_offer(&v1, &good()).is_none());
}

#[test]
fn v1_is_refused() {
    let body = json!({"x402Version":1,"accepts":[good()]}).to_string();
    let pr = parse_required(None, &body).unwrap();
    assert!(pick_requirement_for(&pr, &SOLANA_DEVNET).err().unwrap().contains("V1"));
}

/// 제출 헤더 = V2 모양 — resource·accepted 원본 에코 + payload.transaction 하나.
#[test]
fn submission_shape() {
    let pr = pr_of(vec![good()]);
    let req = pick_requirement_for(&pr, &SOLANA_DEVNET).unwrap();
    let signed = SvmSigned {
        transaction: "AQIDBA==".into(),
        signature: "sig".into(),
        blockhash: "b".into(),
        last_valid_block_height: 1,
    };
    let sub = pr.build_submission(&req, &signed).unwrap();
    assert_eq!(sub.header_name, "PAYMENT-SIGNATURE");
    assert_eq!(sub.response_header, "PAYMENT-RESPONSE");
    let v: Value = serde_json::from_slice(&B64.decode(sub.value).unwrap()).unwrap();
    assert_eq!(v["x402Version"], 2);
    assert_eq!(v["resource"]["url"], "https://x/wave");
    assert_eq!(v["accepted"], good());
    assert_eq!(v["payload"], json!({"transaction":"AQIDBA=="}));
}

#[test]
fn header_takes_precedence_over_body() {
    let raw = json!({"x402Version":2,"accepts":[good()]}).to_string();
    let pr = parse_required(Some(&B64.encode(raw)), "not json").unwrap();
    assert_eq!(pr.version, 2);
}

#[test]
fn settlement_parse() {
    let sig = bs58::encode([7u8; 64]).into_string();
    let v2 = B64.encode(format!(r#"{{"success":true,"transaction":"{sig}","network":"solana:x"}}"#));
    assert_eq!(parse_settlement(&v2), Some((sig.clone(), true)));
    let failed = B64.encode(format!(r#"{{"success":false,"transaction":"{sig}"}}"#));
    assert_eq!(parse_settlement(&failed), Some((sig, false)));
    assert_eq!(parse_settlement(&B64.encode(r#"{"success":true}"#)), None);
    assert_eq!(parse_settlement("!!notb64!!"), None);
}

#[test]
fn base_units_format() {
    assert_eq!(base_units_to_usdc("10000").unwrap(), "0.01");
    assert_eq!(base_units_to_usdc("1000000").unwrap(), "1");
    assert_eq!(base_units_to_usdc("1500000").unwrap(), "1.5");
    assert_eq!(base_units_to_usdc("1").unwrap(), "0.000001");
}
