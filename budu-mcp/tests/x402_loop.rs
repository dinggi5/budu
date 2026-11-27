// x402 HTTP 루프 통합 테스트 — 솔라나 V2.
//
// 로컬 목 x402 서버를 띄우고 실제 reqwest 로 끝과 끝을 본다:
//   요청 → 402(`PAYMENT-REQUIRED` 헤더: EVM 요구 + 솔라나 데브넷 요구) → 우리 고르기(솔라나만) →
//   [지갑이 부분 서명한 트랜잭션] → `PAYMENT-SIGNATURE` 로 재요청 → 목 서버가 봉투를 검사 → 200 + `PAYMENT-RESPONSE`.
//
// GUI/IPC(사람 승인·서명)만 빠진 형태다. 「지갑이 서명한 트랜잭션」 자리에는 정답지(`src-tauri/src/svm_x402_golden.json`
// — 공식 클라이언트가 만든 실제 부분 서명 tx)를 넣는다. 서명 코어 자체는 src-tauri 의 정답지 테스트가 바이트 단위로 고정한다.

use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

use budu_mcp::chain::SOLANA_DEVNET;
use budu_mcp::policy::SvmSigned;
use budu_mcp::x402;

const PAY_TO: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";
const FEE_PAYER: &str = "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5";
const PAID_BODY: &str = "{\"wave_m\":1.2,\"source\":\"sample\"}";

fn golden_tx() -> String {
    let v: Value = serde_json::from_str(include_str!("../../src-tauri/src/svm_x402_golden.json")).unwrap();
    v["cases"][0]["transaction_base64"].as_str().unwrap().to_string()
}

fn svm_requirement() -> Value {
    json!({"scheme":"exact","network":SOLANA_DEVNET.x402_network_caip2,"amount":"10000",
        "asset":SOLANA_DEVNET.usdc_mint,"payTo":PAY_TO,"maxTimeoutSeconds":60,
        "extra":{"feePayer":FEE_PAYER,"memo":"budu-golden-1"}})
}

fn payment_required() -> Value {
    json!({"x402Version":2,"error":"Payment required",
        "resource":{"url":"http://127.0.0.1/wave","description":"부산 인근 파고","mimeType":"application/json"},
        "accepts":[
            {"scheme":"exact","network":"eip155:84532","amount":"10000",
             "asset":"0x036CbD53842c5426634e7929541eC2318f3dCF7e",
             "payTo":"0x209693Bc6afc0C5328bA36FaF03C514EF312287C","extra":{"name":"USDC","version":"2"}},
            svm_requirement()
        ]})
}

fn read_http_request(s: &mut TcpStream) -> String {
    let mut buf = vec![0u8; 65536];
    let mut out = Vec::new();
    loop {
        let n = s.read(&mut buf).unwrap();
        if n == 0 {
            break;
        }
        out.extend_from_slice(&buf[..n]);
        if out.windows(4).any(|w| w == b"\r\n\r\n") {
            break;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn header_value(req: &str, name: &str) -> Option<String> {
    req.lines().find_map(|l| {
        let (k, v) = l.split_once(':')?;
        k.trim().eq_ignore_ascii_case(name).then(|| v.trim().to_string())
    })
}

/// 결제 봉투 검사 — 판매자 미들웨어가 페이실리테이터에 넘기기 전에 보는 것.
fn verify_envelope(header: &str) -> Result<(), String> {
    let v: Value = serde_json::from_slice(&B64.decode(header).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    if v["x402Version"] != 2 {
        return Err("version".into());
    }
    if v["accepted"] != svm_requirement() {
        return Err(format!("accepted 가 제시한 솔라나 요구와 다름: {}", v["accepted"]));
    }
    if v["resource"]["description"] != "부산 인근 파고" {
        return Err("resource 에코 안 됨".into());
    }
    let tx = v["payload"]["transaction"].as_str().ok_or("payload.transaction 없음")?;
    let bytes = B64.decode(tx).map_err(|e| e.to_string())?;
    // 서명 2칸(compact 2) · 0번(feePayer) 자리는 비어 있음 · v0 메시지
    if bytes[0] != 2 || bytes[1..65].iter().any(|&b| b != 0) || bytes[129] != 0x80 {
        return Err("부분 서명 v0 트랜잭션 모양이 아님".into());
    }
    if v["payload"].as_object().map(|o| o.len()) != Some(1) {
        return Err("payload 에 transaction 말고 다른 칸".into());
    }
    Ok(())
}

fn spawn_mock() -> (String, std::sync::mpsc::Receiver<Result<(), String>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/wave", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for n in 0..2 {
            let (mut s, _) = listener.accept().unwrap();
            let req = read_http_request(&mut s);
            if n == 0 {
                let h = B64.encode(payment_required().to_string());
                let resp = format!(
                    "HTTP/1.1 402 Payment Required\r\nPAYMENT-REQUIRED: {h}\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}"
                );
                s.write_all(resp.as_bytes()).unwrap();
            } else {
                let res = header_value(&req, "payment-signature")
                    .ok_or("PAYMENT-SIGNATURE 없음".to_string())
                    .and_then(|h| verify_envelope(&h));
                let ok = res.is_ok();
                tx.send(res).unwrap();
                let settle = B64.encode(
                    json!({"success":ok,"transaction":bs58_sig(),"network":SOLANA_DEVNET.x402_network_caip2,"payer":"x"})
                        .to_string(),
                );
                let (status, body) = if ok { ("200 OK", PAID_BODY) } else { ("402 Payment Required", "{}") };
                let resp = format!(
                    "HTTP/1.1 {status}\r\nPAYMENT-RESPONSE: {settle}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                s.write_all(resp.as_bytes()).unwrap();
            }
        }
    });
    (url, rx)
}

/// 64바이트 서명 base58 (정산 tx id 자리).
fn bs58_sig() -> String {
    // '1' 은 base58 의 0 바이트 — 64개면 64바이트 0. 꼴 검사(is_tx_hash)만 통과하면 된다.
    "1".repeat(64)
}

/// 🔴 솔라나 V2 루프 — EVM 요구는 건너뛰고 솔라나 데브넷 요구를 골라, 봉투를 판매자가 받는 모양 그대로 보낸다.
#[tokio::test]
async fn x402_svm_v2_loop_picks_solana_and_submits_envelope() {
    let (url, rx) = spawn_mock();
    let client = reqwest::Client::new();
    let r = client.get(&url).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 402);
    let pr_header = r.headers().get("payment-required").unwrap().to_str().unwrap().to_string();
    let body = r.text().await.unwrap();
    let required = x402::parse_required(Some(&pr_header), &body).unwrap();
    let req = x402::pick_requirement_for(&required, &SOLANA_DEVNET).expect("솔라나 요구를 골라야 한다");
    assert_eq!(req.terms.pay_to, PAY_TO);
    assert_eq!(req.terms.fee_payer, FEE_PAYER);
    assert_eq!(req.terms.memo.as_deref(), Some("budu-golden-1"));
    assert_eq!(x402::base_units_to_usdc(&req.amount).unwrap(), "0.01");

    // GUI 가 돌려줬을 부분 서명 트랜잭션(정답지).
    let signed = SvmSigned {
        transaction: golden_tx(),
        signature: "x".into(),
        blockhash: "EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k".into(),
        last_valid_block_height: 1,
    };
    let sub = required.build_submission(&req, &signed).unwrap();
    let paid = client.get(&url).header(sub.header_name, &sub.value).send().await.unwrap();
    rx.recv().unwrap().expect("목 판매자가 봉투를 받아야 한다");
    assert_eq!(paid.status().as_u16(), 200);
    let settle = paid.headers().get(sub.response_header).unwrap().to_str().unwrap().to_string();
    let (tx, ok) = x402::parse_settlement(&settle).unwrap();
    assert!(ok);
    assert!(x402::is_tx_hash(&tx));
    assert_eq!(paid.text().await.unwrap(), PAID_BODY);
}

/// 라이브 — 실제 데모 판매자(budu-port)의 402 를 우리 고르기가 받는지. `cd budu-port && npm start` 뒤
/// `cargo test --test x402_loop live_budu_port_is_picked -- --ignored`.
#[tokio::test]
#[ignore]
async fn live_budu_port_is_picked() {
    let url = std::env::var("BUDU_PORT_URL").unwrap_or("http://localhost:4021/wave".into());
    let r = reqwest::get(&url).await.unwrap();
    assert_eq!(r.status().as_u16(), 402);
    let h = r.headers().get("payment-required").unwrap().to_str().unwrap().to_string();
    let required = x402::parse_required(Some(&h), "").unwrap();
    let req = x402::pick_requirement_for(&required, &SOLANA_DEVNET).unwrap();
    assert_eq!(req.terms.amount, "10000");
    assert!(budu_mcp::solana::token_account_exists(
        &reqwest::Client::new(),
        SOLANA_DEVNET.default_rpc,
        SOLANA_DEVNET.genesis_hash,
        &req.terms.pay_to,
        SOLANA_DEVNET.usdc_mint,
        6,
    )
    .await
    .unwrap());
}
