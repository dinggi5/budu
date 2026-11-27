use super::*;

/// 🔴 개발 78: 결제는 https 또는 이 컴퓨터 안의 http 만.
#[test]
fn payment_only_over_https_or_loopback() {
    let ok = |u: &str| payment_url_ok(&reqwest::Url::parse(u).unwrap());
    assert!(ok("https://api.exa.ai/search"));
    assert!(ok("http://localhost:4021/paid"));
    assert!(ok("http://127.0.0.1:8080/x"));
    assert!(ok("http://[::1]:8080/x"));
    assert!(ok("http://kadi.localhost/x"));
    assert!(!ok("http://api.example.com/paid"));
    assert!(!ok("http://10.0.0.5/paid"));
    assert!(!ok("http://localhost.evil.com/paid"));
    assert!(!ok("ftp://localhost/x"));
}
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};

/// 목 서버가 받은 요청 하나.
#[derive(Debug, Clone)]
struct Seen {
    method: String,
    path: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

impl Seen {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }
}

fn read_request(s: &mut TcpStream) -> Seen {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    let head_end = loop {
        let n = s.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break buf.len();
        }
        buf.extend_from_slice(&chunk[..n]);
        if let Some(i) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            break i + 4;
        }
    };
    let head = String::from_utf8_lossy(&buf[..head_end]).into_owned();
    let mut lines = head.lines();
    let mut first = lines.next().unwrap_or("").split_whitespace();
    let method = first.next().unwrap_or("").to_string();
    let path = first.next().unwrap_or("").to_string();
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(':'))
        .map(|(k, v)| (k.trim().to_string(), v.trim().to_string()))
        .collect();
    let len: usize = headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case("content-length"))
        .and_then(|(_, v)| v.parse().ok())
        .unwrap_or(0);
    let mut body = buf[head_end..].to_vec();
    while body.len() < len {
        let n = s.read(&mut chunk).unwrap_or(0);
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
    }
    Seen {
        method,
        path,
        headers,
        body,
    }
}

/// 요청마다 `answer` 가 정한 응답(상태 줄 + 헤더 + 본문)을 쓰고, 받은 요청을 채널로 돌려준다.
fn spawn_mock(answers: Vec<String>) -> (String, std::sync::mpsc::Receiver<Seen>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        for answer in answers {
            let Ok((mut s, _)) = listener.accept() else {
                return;
            };
            let seen = read_request(&mut s);
            let _ = s.write_all(answer.as_bytes());
            let _ = s.flush();
            let _ = tx.send(seen);
        }
    });
    (base, rx)
}

fn http(status: &str, extra: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n{body}",
        body.len()
    )
}

fn sample_request() -> FetchRequest {
    FetchRequest::build(
        Some("POST"),
        &[("X-Client".into(), "budu-test".into())],
        Some(&serde_json::json!({"query": "결제한 바로 그 질문", "numResults": 3})),
    )
    .unwrap()
}

fn sample_signed() -> crate::policy::SvmSigned {
    crate::policy::SvmSigned {
        transaction: "AQID".into(),
        signature: "5ourSig".into(),
        blockhash: "EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k".into(),
        last_valid_block_height: 1,
    }
}

fn submission() -> x402::Submission {
    x402::Submission {
        header_name: "PAYMENT-SIGNATURE",
        value: "c2lnbmVk".into(),
        response_header: "PAYMENT-RESPONSE",
        alt_header: None,
    }
}

/// 판매자 정산 tx 는 2xx·성공·해시 꼴일 때만 링크가 된다.
#[test]
fn seller_settlement_link_only_for_clean_success() {
    use base64::Engine;
    let enc = |v: serde_json::Value| base64::engine::general_purpose::STANDARD.encode(v.to_string());
    let h = bs58::encode([3u8; 64]).into_string();
    let ok = enc(serde_json::json!({"success": true, "transaction": h}));
    let p = ("https://explorer.solana.com/tx/", "?cluster=devnet");
    assert_eq!(
        seller_settlement_link(true, &ok, p),
        (h.clone(), format!("https://explorer.solana.com/tx/{h}?cluster=devnet"))
    );
    // EVM 해시 꼴은 이제 링크가 안 된다
    let evm = enc(serde_json::json!({"success": true, "transaction": format!("0x{}", "ab".repeat(32))}));
    assert_eq!(seller_settlement_link(true, &evm, p).0, "");
    // success 가 없으면 parse_settlement 처럼 성공으로 읽는다.
    let silent = enc(serde_json::json!({"txHash": h}));
    assert_eq!(seller_settlement_link(true, &silent, p).0, h);
    let none = (String::new(), String::new());
    // 비-2xx · 실패라 말함 · 해시 꼴 아님 · 못 읽는 헤더
    assert_eq!(seller_settlement_link(false, &ok, p), none);
    let failed = enc(serde_json::json!({"success": false, "transaction": h}));
    assert_eq!(seller_settlement_link(true, &failed, p), none);
    let odd = enc(serde_json::json!({"success": true, "transaction": "https://evil.example/"}));
    assert_eq!(seller_settlement_link(true, &odd, p), none);
    assert_eq!(seller_settlement_link(true, "!!", p), none);
}

/// 🔴 개발 77 — **결제 재요청은 첫 요청과 같은 method·headers·body 로 나간다.** 사람이 승인한 결제는 그 요청에 대한
/// 값이라, 재요청이 GET 으로 바뀌거나 body 가 빠지면 서명한 것과 다른 걸 산다. 첫 요청은 실제 `run_x402` 로,
/// 재요청은 실제 `deliver` 로 보내 목 서버가 받은 바이트를 맞대 본다.
#[tokio::test]
async fn paid_retry_repeats_method_headers_and_body() {
    let (base, rx) = spawn_mock(vec![
        // run_x402 의 첫 요청에 402 를 주면 GUI 승인으로 넘어가므로, 첫 요청은 200 으로 받아 무엇이 나갔는지만 본다.
        http("200 OK", "", r#"{"free":true}"#),
        http("200 OK", "", r#"{"results":[]}"#),
    ]);
    let url = format!("{base}/search");
    let request = sample_request();

    let first = run_x402(&url, &request, None, None).await.unwrap();
    assert!(matches!(
        first,
        X402Outcome::NotPaid {
            http_status: 200,
            ..
        }
    ));
    let probe = rx.recv().unwrap();

    let paid = deliver(Delivery {
        sub: submission(),
        url: &url,
        request: &request,
        signed: sample_signed(),
        explorer: ("https://x/tx/", ""),
        chain_id: 103,
        amount: "0.007".into(),
        pay_to: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".into(),
        resource: url.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(paid, X402Outcome::Paid { ok: true, .. }));
    let retry = rx.recv().unwrap();

    assert_eq!(probe.method, "POST");
    assert_eq!(retry.method, probe.method);
    assert_eq!(retry.path, probe.path);
    assert_eq!(retry.body, probe.body);
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&retry.body).unwrap(),
        serde_json::json!({"query": "결제한 바로 그 질문", "numResults": 3})
    );
    for h in ["x-client", "content-type"] {
        assert!(probe.header(h).is_some(), "첫 요청에 {h} 가 없다");
        assert_eq!(retry.header(h), probe.header(h), "{h} 가 갈렸다");
    }
    // 다른 건 결제 헤더 하나뿐이다.
    assert_eq!(probe.header("payment-signature"), None);
    assert_eq!(retry.header("payment-signature"), Some("c2lnbmVk"));
}

/// 🔴 결제 재요청은 리다이렉트를 따라가지 않는다(서명 유출 방지) — method·body 가 붙어도 그대로다.
#[tokio::test]
async fn paid_retry_does_not_follow_redirects() {
    let (elsewhere, other_rx) = spawn_mock(vec![http("200 OK", "", "{}")]);
    let (base, rx) = spawn_mock(vec![http(
        "302 Found",
        &format!("Location: {elsewhere}/steal\r\n"),
        "",
    )]);
    let url = format!("{base}/search");
    let out = deliver(Delivery {
        sub: submission(),
        url: &url,
        request: &sample_request(),
        signed: sample_signed(),
        explorer: ("https://x/tx/", ""),
        chain_id: 103,
        amount: "0.007".into(),
        pay_to: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".into(),
        resource: url.clone(),
    })
    .await
    .unwrap();
    assert!(matches!(
        out,
        X402Outcome::Paid {
            http_status: 302,
            ok: false,
            ..
        }
    ));
    assert_eq!(
        rx.recv().unwrap().header("payment-signature"),
        Some("c2lnbmVk")
    );
    assert!(
        other_rx
            .recv_timeout(std::time::Duration::from_millis(300))
            .is_err(),
        "리다이렉트를 따라가 결제 헤더가 다른 곳으로 갔다"
    );
}

/// 결제 뒤 응답이 중간에 끊기면 받은 만큼을 남기고 「다시 부르면 또 결제된다」를 싣는다.
#[tokio::test]
async fn paid_response_cut_short_is_not_plain_success() {
    // Content-Length 는 100 인데 10 바이트만 보내고 닫는다.
    let (base, _rx) = spawn_mock(vec![
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: 100\r\nConnection: close\r\n\r\n{\"part\":1,".to_string(),
    ]);
    let url = format!("{base}/gen");
    let out = deliver(Delivery {
        sub: submission(),
        url: &url,
        request: &sample_request(),
        signed: sample_signed(),
        explorer: ("https://x/tx/", ""),
        chain_id: 103,
        amount: "0.03".into(),
        pay_to: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".into(),
        resource: url.clone(),
    })
    .await
    .unwrap();
    let X402Outcome::Paid { notice, body, .. } = &out else {
        panic!("Paid 가 아니다");
    };
    assert!(body.incomplete);
    assert!(body.text.starts_with("{\"part\":1,"), "{}", body.text);
    assert!(!notice.is_empty());
    assert_eq!(out.to_json()["notice"], *notice);
}

/// 상대 경로 Location 은 요청 주소 기준으로 풀어서 알려 준다. 304 같은 3xx 는 리다이렉트로 보지 않는다.
#[tokio::test]
async fn probe_redirect_location_is_resolved() {
    let (base, _rx) = spawn_mock(vec![http(
        "308 Permanent Redirect",
        "Location: /v2/search\r\n",
        "",
    )]);
    let err = match run_x402(&format!("{base}/search"), &sample_request(), None, None).await {
        Err(e) => e,
        Ok(_) => panic!("리다이렉트를 따라갔다"),
    };
    assert!(err.contains(&format!("{base}/v2/search")), "{err}");

    let (base, _rx) = spawn_mock(vec![http("304 Not Modified", "", "")]);
    let out = run_x402(&format!("{base}/x"), &sample_request(), None, None)
        .await
        .unwrap();
    assert!(matches!(
        out,
        X402Outcome::NotPaid {
            http_status: 304,
            ..
        }
    ));
}

/// method·headers·body 가 있는 첫 요청도 리다이렉트를 따라가지 않는다 — 301/302 는 POST 를 GET 으로 바꾸고 body 를
/// 버리므로, 따라가면 402 를 낸 곳과 우리가 보낼 요청이 갈린다. 아무것도 결제하기 전에 멈춘다.
#[tokio::test]
async fn probe_with_body_stops_at_redirect() {
    let (elsewhere, other_rx) = spawn_mock(vec![http("200 OK", "", "{}")]);
    let (base, _rx) = spawn_mock(vec![http(
        "301 Moved Permanently",
        &format!("Location: {elsewhere}/new\r\n"),
        "",
    )]);
    let err = match run_x402(&format!("{base}/old"), &sample_request(), None, None).await {
        Err(e) => e,
        Ok(_) => panic!("리다이렉트를 따라갔다"),
    };
    assert!(err.contains(&format!("{elsewhere}/new")), "{err}");
    assert!(other_rx
        .recv_timeout(std::time::Duration::from_millis(300))
        .is_err());
}

fn no_content(reason: &str) -> X402Outcome {
    X402Outcome::PaidNoContent {
        tx: "0xTX".into(),
        explorer: String::new(),
        reason: reason.into(),
        notice: "n".into(),
    }
}

fn paid(ok: bool, tx: &str) -> X402Outcome {
    X402Outcome::Paid {
        notice: String::new(),
        tx: tx.into(),
        explorer: String::new(),
        http_status: if ok { 200 } else { 402 },
        ok,
        amount: "0.01".into(),
        pay_to: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".into(),
        resource: "https://ex.com/a".into(),
        settlement: String::new(),
        settlement_tx: String::new(),
        settlement_explorer: String::new(),
        body: FetchedBody::default(),
    }
}

/// 🔴 돈의 행방 세 갈래 — 표 하나로 전부 박는다. MCP·CLI 가 이 판정 하나를 쓴다.
#[test]
fn payment_state_table() {
    let cases: Vec<(X402Outcome, &str)> = vec![
        (
            X402Outcome::NotPaid {
                http_status: 200,
                body: FetchedBody::default(),
            },
            PAYMENT_NONE,
        ),
        (
            X402Outcome::Declined {
                status: "rejected".into(),
                detail: String::new(),
            },
            PAYMENT_NONE,
        ),
        (no_content("reverted"), PAYMENT_NONE), // 가스만 — 결제액은 그대로
        (no_content("undelivered"), PAYMENT_CONFIRMED), // 채굴 성공, 서버만 모름
        (no_content("pending"), PAYMENT_UNKNOWN),
        (no_content("unknown"), PAYMENT_UNKNOWN),
        (no_content("someday-new"), PAYMENT_UNKNOWN), // 모르는 값을 「안 나감」으로 접지 않는다
        (paid(true, ""), PAYMENT_CONFIRMED),          // 서명 갈래 정산 성공
        (paid(false, ""), PAYMENT_UNKNOWN),           // 서명 갈래 거절 — 정산 여부 모름(#7)
        (paid(false, "0xTX"), PAYMENT_CONFIRMED),     // 직접 제출은 채굴 성공 뒤에만 여기 온다
    ];
    for (o, want) in &cases {
        assert_eq!(o.payment_state(), *want, "{}", o.to_json());
        // 예전 `paid` 는 「안 나갔다고 단정할 수 없다」 — none 일 때만 false.
        assert_eq!(o.paid(), *want != PAYMENT_NONE);
        assert_eq!(o.to_json()["payment"], *want);
        assert_eq!(o.to_json()["paid"], *want != PAYMENT_NONE);
    }
}
