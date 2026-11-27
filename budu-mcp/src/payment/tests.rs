use super::*;

/// 어떤 실제 체인도 id 0 이 아니라 활성 체인이 무엇이든 어긋난다 — 파일을 안 건드리고 돌아온다.
#[test]
fn x402_request_refuses_when_chain_moved() {
    let err = write_request("USDC", "0xabc", "0.01", "", crate::policy::KIND_X402, "", Some(0), None, "")
        .expect_err("체인이 다르면 거절");
    assert!(err.contains("네트워크") || err.contains("network"), "{err}");
}

/// 결제 요청 JSON 왕복 — src-tauri가 읽는 형식과 호환돼야 한다.
#[test]
fn payment_request_roundtrip() {
    let r = PaymentRequest {
        id: "123".into(),
        token: "USDC".into(),
        to: "0xabc".into(),
        amount: "1.5".into(),
        memo: "데이터 API 호출".into(),
        created: 100,
        kind: crate::policy::KIND_X402.into(),
        resource: String::new(),
        chain_id: 84_532,
        account: 1,
        from: "0xOne".into(),
        lease: false,
        svm: Some(crate::policy::SvmTerms {
            network: "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1".into(),
            asset: "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU".into(),
            pay_to: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk".into(),
            amount: "1500000".into(),
            fee_payer: "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5".into(),
            memo: None,
        }),
        job: "오슬로호 입항 준비".into(),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: PaymentRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, "123");
    assert_eq!((back.account, back.from.as_str()), (1, "0xOne"));
    assert_eq!(back.token, "USDC");
    assert_eq!(back.svm, r.svm);
    assert_eq!(back.job, "오슬로호 입항 준비");
    // memo 가 없으면 칸째 안 쓴다(GUI 의 serde default 와 맞물림)
    assert!(!json.contains("\"memo\":null"));
    assert_eq!(back.memo, "데이터 API 호출");
    assert_eq!(back.kind, "x402-svm");
}

/// 🔴 옛 앱이 쓴 하트비트에는 `ui_ok` 가 없다 → **true 로 읽혀야** 한다.
/// 기본값을 빠뜨리면 serde 가 false 로 채워 **모든 결제가 「화면이 죽었다」로 거절**된다 —
/// 앱만 업데이트가 늦어도 지갑이 통째로 먹통이 되는 자리라 테스트로 못박는다.
#[test]
fn heartbeat_without_ui_ok_is_alive() {
    let h: Heartbeat = serde_json::from_str(r#"{"ts":100}"#).unwrap();
    assert!(h.ui_ok, "옛 하트비트는 「띄울 수 있다」로 읽혀야 한다");
    let h: Heartbeat = serde_json::from_str(r#"{"ts":100,"ui_ok":false}"#).unwrap();
    assert!(!h.ui_ok);
}

/// 결과 파싱 — GUI가 쓴 형식. x402 필드 없는 기존 결과도 파싱돼야 한다.
#[test]
fn payment_result_parses() {
    let json = r#"{"id":"1","status":"approved","tx_hash":"0xhash","detail":""}"#;
    let r: PaymentResult = serde_json::from_str(json).unwrap();
    assert_eq!(r.status, "approved");
    assert_eq!(r.tx_hash, "0xhash");
    assert!(r.x402.is_none());
}

/// x402 결과: GUI 가 부분 서명 트랜잭션을 함께 돌려준다.
#[test]
fn x402_result_carries_payment() {
    let json = r#"{"id":"1","status":"approved","tx_hash":"","detail":"",
      "x402":{"transaction":"AQID","signature":"5sig","blockhash":"bh","last_valid_block_height":7}}"#;
    let r: PaymentResult = serde_json::from_str(json).unwrap();
    let p = r.x402.expect("x402 페이로드");
    assert_eq!(p.transaction, "AQID");
    assert_eq!(p.signature, "5sig");
    assert_eq!(p.last_valid_block_height, 7);
    // 옛(EVM) 모양은 읽히지 않는다 — 결제 헤더로 조립될 수 없다
    let evm = r#"{"id":"1","status":"approved","tx_hash":"","detail":"",
      "x402":{"signature":"0xsig","authorization":{"from":"0xa"}}}"#;
    assert!(serde_json::from_str::<PaymentResult>(evm).is_err());
}

/// 🔴 하트비트에 `kinds` 가 없으면 **아무 방식도 지원 안 함**으로 읽는다(닫히는 쪽). 앱이 적어 주면 그대로.
#[test]
fn heartbeat_without_kinds_supports_nothing() {
    let old: Heartbeat = serde_json::from_str(r#"{"ts":1}"#).unwrap();
    assert!(old.kinds.is_empty());
    let new: Heartbeat = serde_json::from_str(r#"{"ts":1,"ui_ok":true,"kinds":["x402"]}"#).unwrap();
    assert_eq!(new.kinds, vec!["x402".to_string()]);
}

/// 하트비트 신선도: 10초 이내면 살아있음, 넘으면 죽음.
#[test]
fn heartbeat_freshness() {
    assert!(is_fresh(1000, 1000)); // 같은 순간
    assert!(is_fresh(1010, 1000)); // 10초 경과(경계)
    assert!(!is_fresh(1011, 1000)); // 11초 → 만료
    assert!(is_fresh(1000, 2000)); // 시계 역전도 살아있음으로(saturating)
}

/// 요청 id는 비어있지 않다.
#[test]
fn new_id_is_nonempty() {
    assert!(!new_id().is_empty());
}

/// single-flight 원자 획득: 첫 claim 은 성공, 파일이 남아 있는 동안 두 번째 claim 은 거절.
/// 첫 내용은 덮어쓰이지 않는다(둘 다 통과해 한쪽 유실되던 경합 방지).
#[test]
fn claim_request_is_single_flight() {
    let dir = std::env::temp_dir().join(format!("budu-mcp-claim-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("payment_request.json");
    let _ = fs::remove_file(&path);

    assert!(claim_request_file(&path, b"first").is_ok());
    assert!(claim_request_file(&path, b"second").is_err()); // 이미 대기 중 → 거절
    assert_eq!(fs::read_to_string(&path).unwrap(), "first"); // 첫 내용 보존

    // 처리 후(파일 제거) 다시 획득 가능.
    let _ = fs::remove_file(&path);
    assert!(claim_request_file(&path, b"third").is_ok());
    // 🔴 개발 73: 임시 파일이 남지 않는다(성공·거절 모두) — 자리엔 완성된 파일 하나뿐.
    assert!(claim_request_file(&path, b"fourth").is_err());
    let left: Vec<_> = fs::read_dir(&dir).unwrap().flatten().map(|e| e.file_name()).collect();
    assert_eq!(left.len(), 1, "{left:?}");
    let _ = fs::remove_dir_all(&dir);
}

/// 요청 하나를 그 경로에 써 둔다(테스트 도우미).
fn write_req(path: &Path, id: &str) {
    let r = PaymentRequest {
        id: id.into(),
        token: "USDC".into(),
        to: "0x0".into(),
        amount: "1".into(),
        memo: String::new(),
        created: 0,
        kind: crate::policy::KIND_X402.into(),
        resource: String::new(),
        chain_id: 0,
        account: 0,
        from: String::new(),
        lease: false,
        svm: None,
        job: String::new(),
    };
    fs::write(path, serde_json::to_string(&r).unwrap()).unwrap();
}

/// 🔴 거두기는 **내 요청일 때만** 지운다. 남의 요청을 지우면 그쪽의
/// single-flight 슬롯이 깨져 겹친 결제가 생긴다.
#[test]
fn cancel_only_removes_my_request() {
    let dir = std::env::temp_dir().join(format!("budu-mcp-cancel-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("payment_request.json");

    write_req(&path, "A");
    cancel_request_at(&path, "B"); // 남의 요청 → 안 건드린다
    assert!(path.exists(), "다른 id 의 요청은 남아야 한다");
    cancel_request_at(&path, "A"); // 내 요청 → 지운다
    assert!(!path.exists(), "내 요청은 거둬야 한다");

    cancel_request_at(&path, "A"); // 이미 없는 경우도 조용히 통과
    let _ = fs::remove_dir_all(&dir);
}

/// 🔴 **대기 중에 future 가 버려지면 요청을 거둔다**.
/// 클라이언트가 죽으면 런타임이 내려가며 이 future 가 버려지는데, 그때 요청 파일이 남으면
/// 「아무도 안 기다리는 승인 창」이 뜬다. 결과를 받았거나 시간 초과로 정상 반환한 경우엔
/// **거두면 안 된다**(그 뒷정리는 호출자 몫이고, 결과를 받은 요청은 이미 남의 것일 수 있다).
#[test]
fn drop_while_waiting_cancels_but_normal_return_does_not() {
    let dir = std::env::temp_dir().join(format!("budu-mcp-drop-{}", std::process::id()));
    let _ = fs::create_dir_all(&dir);
    let path = dir.join("payment_request.json");

    // 대기 중 버려짐 → 거둔다.
    write_req(&path, "A");
    {
        let _g = CancelOnDrop {
            id: "A",
            path: &path,
            armed: true,
        };
    }
    assert!(!path.exists(), "대기 중 버려지면 요청을 거둔다");

    // 정상 반환(결과 수신·시간 초과)에서 내려 둔 가드는 아무것도 안 한다.
    write_req(&path, "A");
    {
        let mut g = CancelOnDrop {
            id: "A",
            path: &path,
            armed: true,
        };
        g.armed = false;
    }
    assert!(path.exists(), "정상 반환이면 거두지 않는다");
    let _ = fs::remove_dir_all(&dir);
}

// ── 개발 66: 시간 초과 뒤의 결말 ─────────────────────────────────────────────────────────

fn tmp_dir(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("budu-mcp-late-{tag}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&d);
    fs::create_dir_all(&d).unwrap();
    d
}

/// GUI 가 쓰는 결제 시도 기록을 흉내 낸다(같은 경로 규칙 — shared/policy.rs).
fn write_attempt(dir: &Path, id: &str, state: &str, status: &str, tx: &str) {
    let rec = crate::policy::AttemptRecord {
        v: 1,
        id: id.into(),
        state: state.into(),
        kind: "transfer".into(),
        chain_id: 5_042_002,
        started: 1,
        updated: 1,
        status: status.into(),
        tx_hash: tx.into(),
        detail: String::new(),
        x402: None,
    };
    let path = crate::policy::attempt_path(dir, id).unwrap();
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, serde_json::to_string(&rec).unwrap()).unwrap();
}

/// 승인이 시작된 적 없으면 예전과 같다 — None(= 「응답 없음」)이고, 요청은 거둔다.
#[tokio::test]
async fn timeout_with_nothing_started_is_none_and_withdraws() {
    let dir = tmp_dir("none");
    let req = dir.join("payment_request.json");
    write_req(&req, "A");
    let gone = std::sync::atomic::AtomicBool::new(false);
    let r = await_result_in(&dir, "A", Duration::ZERO, Duration::from_secs(5), &|| true, Duration::ZERO, &gone).await;
    assert!(r.is_none());
    assert!(!gone.into_inner(), "시간 초과가 먼저면 «앱 꺼짐» 이 아니다");
    assert!(!req.exists(), "시간 초과면 요청을 거둔다");
    let _ = fs::remove_dir_all(&dir);
}

/// 앱이 죽으면 5분을 채우지 않고 거둔다 — 승인이 시작된 적 없으면 None, 요청도 치운다.
#[tokio::test]
async fn app_gone_withdraws_early() {
    let dir = tmp_dir("gone");
    let req = dir.join("payment_request.json");
    write_req(&req, "A");
    let t = std::time::Instant::now();
    let gone = std::sync::atomic::AtomicBool::new(false);
    let r = await_result_in(&dir, "A", Duration::from_secs(60), Duration::from_secs(5), &|| true, Duration::ZERO, &gone).await;
    assert!(r.is_none());
    assert!(gone.into_inner(), "거둔 이유 = 앱 꺼짐");
    assert!(!req.exists(), "앱이 죽어도 요청을 거둔다");
    assert!(t.elapsed() < Duration::from_secs(5), "5분(여기선 60초)을 기다리지 않는다");
    let _ = fs::remove_dir_all(&dir);
}

/// 앱이 **전송 중에** 죽었으면(`sending` 이 남음) 「안 나감」 이 아니라 늦은 대기 끝에 「불명」 이다.
#[tokio::test]
async fn app_gone_while_sending_is_unknown() {
    let dir = tmp_dir("gone-sending");
    write_req(&dir.join("payment_request.json"), "A");
    write_attempt(&dir, "A", crate::policy::ATTEMPT_SENDING, "pending", "");
    let gone = std::sync::atomic::AtomicBool::new(false);
    let r = await_result_in(&dir, "A", Duration::from_secs(60), Duration::from_millis(800), &|| true, Duration::ZERO, &gone)
        .await
        .expect("불명");
    assert_eq!(r.status, "unknown");
    assert!(gone.into_inner(), "앱이 꺼져 거뒀어도 결과가 있으면 그 결과(불명)가 이긴다");
    let _ = fs::remove_dir_all(&dir);
}

/// 확실한 실패(비번 오류 뒤 사람이 떠난 경우 등)도 「아무것도 안 나감」이다.
#[tokio::test]
async fn failed_record_means_nothing_left() {
    let dir = tmp_dir("failed");
    write_req(&dir.join("payment_request.json"), "A");
    write_attempt(&dir, "A", crate::policy::ATTEMPT_FAILED, "failed", "");
    let r = await_result_in(&dir, "A", Duration::ZERO, Duration::from_secs(5), &|| false, Duration::MAX, &Default::default()).await;
    assert!(r.is_none());
    let _ = fs::remove_dir_all(&dir);
}

/// 🔴 5분이 끝날 무렵 사람이 승인해 GUI 가 전송 중이면, 「응답 없음」이
/// 아니라 **전송이 끝나기를 기다렸다가 그 결말**을 돌려준다. 요청을 거둔 뒤라 GUI 는 결과 파일을
/// 안 쓴다(남의 결과를 덮지 않으려고) — 결말은 기록에서 온다.
#[tokio::test]
async fn timeout_while_gui_is_sending_returns_its_outcome() {
    let dir = tmp_dir("sending");
    let req = dir.join("payment_request.json");
    write_req(&req, "A");
    write_attempt(&dir, "A", crate::policy::ATTEMPT_SENDING, "", "");
    let d2 = dir.clone();
    let gui = tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(1200)).await;
        write_attempt(&d2, "A", crate::policy::ATTEMPT_DONE, "approved", "0xT");
    });
    let r = await_result_in(&dir, "A", Duration::ZERO, Duration::from_secs(10), &|| false, Duration::MAX, &Default::default())
        .await
        .expect("전송 중이던 승인은 None(응답 없음)이 아니다");
    assert_eq!((r.status.as_str(), r.tx_hash.as_str()), ("approved", "0xT"));
    assert!(
        !req.exists(),
        "요청은 먼저 거둔다 — 새 승인이 시작되지 않게"
    );
    gui.await.unwrap();
    let _ = fs::remove_dir_all(&dir);
}

/// 늦은 대기까지 넘겨도 GUI 가 끝내지 못했으면 **불명**이다 — 사람은 승인했고 돈이 나가는 중일 수 있다.
#[tokio::test]
async fn still_sending_after_the_late_wait_is_unknown() {
    let dir = tmp_dir("stuck");
    write_req(&dir.join("payment_request.json"), "A");
    write_attempt(&dir, "A", crate::policy::ATTEMPT_SENDING, "", "");
    let r = await_result_in(&dir, "A", Duration::ZERO, Duration::from_millis(800), &|| false, Duration::MAX, &Default::default())
        .await
        .expect("불명은 None 이 아니다");
    assert_eq!(r.status, "unknown");
    assert!(!r.detail.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

/// 🔴 개발 73: 시도 기록이 깨져 있으면 「아무것도 안 나감」이 아니라 불명 — 깨진 게 sending·done 이었을 수 있다.
#[tokio::test]
async fn broken_attempt_after_timeout_is_unknown() {
    let dir = tmp_dir("broken-attempt");
    write_req(&dir.join("payment_request.json"), "A");
    write_attempt(&dir, "A", crate::policy::ATTEMPT_SENDING, "", "");
    fs::write(crate::policy::attempt_path(&dir, "A").unwrap(), "{\"v\":1,").unwrap();
    let r = await_result_in(&dir, "A", Duration::ZERO, Duration::from_millis(800), &|| false, Duration::MAX, &Default::default())
        .await
        .expect("못 읽는 기록은 「안 나감」(None)이 아니다");
    assert_eq!(r.status, "unknown");
    let _ = fs::remove_dir_all(&dir);
}

/// GUI 가 이미 「불명」으로 끝냈으면 그 tx 해시가 그대로 넘어온다(MCP 가 영수증을 물어볼 수 있게).
#[tokio::test]
async fn unknown_record_carries_its_tx() {
    let dir = tmp_dir("unknown");
    write_req(&dir.join("payment_request.json"), "A");
    write_attempt(&dir, "A", crate::policy::ATTEMPT_UNKNOWN, "unknown", "0xU");
    let r = await_result_in(&dir, "A", Duration::ZERO, Duration::from_secs(5), &|| false, Duration::MAX, &Default::default())
        .await
        .unwrap();
    assert_eq!((r.status.as_str(), r.tx_hash.as_str()), ("unknown", "0xU"));
    let _ = fs::remove_dir_all(&dir);
}

/// 거두기 직전에 결과 파일이 들어왔으면 그걸 쓴다(기록보다 먼저 본다). 남의 id 결과는 안 가져간다.
#[tokio::test]
async fn result_that_raced_the_timeout_is_kept() {
    let dir = tmp_dir("raced");
    write_req(&dir.join("payment_request.json"), "A");
    let res = dir.join("payment_result.json");
    fs::write(
        &res,
        r#"{"id":"B","status":"approved","tx_hash":"0xB","detail":""}"#,
    )
    .unwrap();
    assert!(
        await_result_in(&dir, "A", Duration::ZERO, Duration::from_secs(1), &|| false, Duration::MAX, &Default::default())
            .await
            .is_none(),
        "남의 결과는 내 것이 아니다"
    );
    assert!(res.exists(), "남의 결과 파일은 건드리지 않는다");
    fs::write(
        &res,
        r#"{"id":"A","status":"approved","tx_hash":"0xA","detail":""}"#,
    )
    .unwrap();
    let r = await_result_in(&dir, "A", Duration::ZERO, Duration::from_secs(1), &|| false, Duration::MAX, &Default::default())
        .await
        .unwrap();
    assert_eq!(r.tx_hash, "0xA");
    let _ = fs::remove_dir_all(&dir);
}

/// 임대 갱신은 내 요청의 수정 시각만 바꾸고, 없는 파일은 만들지 않는다.
#[test]
fn touch_only_refreshes_my_existing_request() {
    let dir = tmp_dir("touch");
    let path = dir.join("payment_request.json");
    touch_if_mine(&path, "A");
    assert!(!path.exists(), "없는 요청을 되살리면 안 된다");
    write_req(&path, "A");
    let old = SystemTime::now() - Duration::from_secs(600);
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    touch_if_mine(&path, "B");
    let m = |p: &Path| fs::metadata(p).unwrap().modified().unwrap();
    assert!(
        m(&path) <= old + Duration::from_secs(1),
        "남의 id 로는 안 바뀐다"
    );
    touch_if_mine(&path, "A");
    assert!(m(&path) > old + Duration::from_secs(500));
    let _ = fs::remove_dir_all(&dir);
}
