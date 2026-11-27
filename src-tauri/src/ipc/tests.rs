use super::*;
use super::watchdog::*;

/// 🔴 개발 73: 못 읽는 요청 파일은 늙었을 때만 치운다 — 읽히는 것·새것은 그대로.
#[test]
fn stale_unreadable_request_is_set_aside() {
    let dir = std::env::temp_dir().join(format!("budu-req-aside-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join("payment_request.json");
    fs::write(&p, "{\"id\":").unwrap();
    set_aside_if_stale_unreadable(&p);
    assert!(p.exists(), "새것은 아직 쓰는 중일 수 있다");
    let old = std::time::SystemTime::now()
        - std::time::Duration::from_secs(APPROVAL_WINDOW_SECS + STALE_GRACE_SECS + 5);
    fs::File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
    set_aside_if_stale_unreadable(&p);
    assert!(!p.exists(), "늙은 반쪽은 치운다");
    assert_eq!(fs::read_dir(&dir).unwrap().count(), 1, "지우지 않고 옆에 남긴다");
    // 깨진 UTF-8 도 같다.
    fs::write(&p, [0xff, 0xfe, b'{']).unwrap();
    fs::File::options().write(true).open(&p).unwrap().set_modified(old).unwrap();
    set_aside_if_stale_unreadable(&p);
    assert!(!p.exists(), "깨진 UTF-8 도 치운다");
    let _ = fs::remove_dir_all(&dir);
}

fn tick_in(now: u64, front_alive: bool, live: bool) -> TickIn {
    TickIn {
        now,
        front_alive,
        live,
        dismissal: crate::appwin::Dismissal::No,
        can_beat: true,
    }
}

/// 🔴 개발 71: 감시 한 차례를 시간 순으로 밟는다 — 프론트가 깨어 있으면 아무것도 안 하고,
/// 잠든 채 요청이 오면 곧바로 한 번·그 뒤 15초마다 깨우고, 세 번 깨워도 안 돌아오면 마지막 깨움 15초 뒤에야
/// 「죽었다」로 본다. 요청이 사라지면 고정을 한 번 풀고 그 뒤론 가만히 있다.
#[test]
fn watch_tick_timeline() {
    let mut st = WatchState::default();
    // 프론트가 깨어 있다 — 하트비트만.
    let o = watch_tick(&mut st, tick_in(100, true, true));
    assert_eq!(
        o,
        TickOut {
            beat: Some(true),
            ..Default::default()
        }
    );
    // 하트비트는 2초마다.
    assert_eq!(watch_tick(&mut st, tick_in(101, true, true)).beat, None);
    // 잠든 프론트에 요청 — 즉시 깨우고 고정.
    let o = watch_tick(&mut st, tick_in(103, false, true));
    assert_eq!(
        (o.set_pinned, o.wake, o.resolve_dead),
        (Some(true), true, false)
    );
    // 15초 안엔 다시 깨우지 않지만 고정은 계속 맞춘다.
    let o = watch_tick(&mut st, tick_in(110, false, true));
    assert_eq!((o.set_pinned, o.wake), (Some(true), false));
    // 두 번째·세 번째 깨움.
    assert!(watch_tick(&mut st, tick_in(118, false, true)).wake);
    assert!(watch_tick(&mut st, tick_in(133, false, true)).wake);
    assert_eq!(st.wakes_without_poll, 3);
    // 세 번째 깨움 뒤 15초가 안 됐다 — 아직 살아 있다고 본다.
    let o = watch_tick(&mut st, tick_in(140, false, true));
    assert!(!o.resolve_dead);
    // 15초가 지났다 — 죽었다. 하트비트도 ui_ok=false.
    let o = watch_tick(&mut st, tick_in(148, false, true));
    assert!(o.resolve_dead);
    assert_eq!(o.beat, Some(false));
    // 폴링이 한 번 돌아오면 카운터가 풀린다.
    watch_tick(&mut st, tick_in(149, true, true));
    assert_eq!((st.wakes_without_poll, st.last_pinned), (0, None));

    // 요청이 사라진 뒤: 우리가 고정했으면 한 번 풀고, 그 뒤론 아무것도 안 한다.
    let mut st = WatchState::default();
    watch_tick(&mut st, tick_in(200, false, true));
    let o = watch_tick(&mut st, tick_in(201, false, false));
    assert_eq!((o.set_pinned, o.wake), (Some(false), false));
    let o = watch_tick(&mut st, tick_in(202, false, false));
    assert_eq!(o.set_pinned, None);
}

/// 닫아 둔 요청(Hold)은 깨우지 않고 사망 카운터도 안 올린다. 만료 직전(Remind)엔 프론트 생사와 무관하게 한 번 띄운다.
/// 지갑이 없으면 하트비트를 안 찍는다.
#[test]
fn watch_tick_respects_dismissal_and_setup() {
    let mut st = WatchState::default();
    let mut i = tick_in(300, false, true);
    i.dismissal = crate::appwin::Dismissal::Hold;
    let o = watch_tick(&mut st, i);
    assert!(!o.wake && !o.remind);
    assert_ne!(o.set_pinned, Some(true), "닫아 둔 요청의 숨은 창에 고정을 걸지 않는다(개발 7)");
    assert_eq!(st.wakes_without_poll, 0);
    let mut i = tick_in(301, true, true);
    i.dismissal = crate::appwin::Dismissal::Remind;
    assert!(watch_tick(&mut st, i).remind);
    let mut i = tick_in(400, true, false);
    i.can_beat = false;
    assert_eq!(watch_tick(&mut st, i).beat, None);
}

/// 개발 7 2차 리뷰 P3 두 건 — 프론트가 잠든 채 Remind 가 오면 (1) 우리가 걸어 둔 고정을 풀지 않고(`show` 가 다시
/// 건다), (2) 되살린 직후 차례(표식이 지워져 No)에 곧바로 또 깨우지 않는다 — 깨우면 사망 카운터가 한 칸 일찍 오른다.
#[test]
fn remind_keeps_pin_and_wake_backoff() {
    let mut st = WatchState::default();
    // 잠든 프론트에 요청 — 깨우고 고정(last_pinned = Some(true)).
    assert!(watch_tick(&mut st, tick_in(500, false, true)).wake);
    let before = st.wakes_without_poll;
    let mut i = tick_in(510, false, true);
    i.dismissal = crate::appwin::Dismissal::Remind;
    let o = watch_tick(&mut st, i);
    assert!(o.remind);
    assert_ne!(o.set_pinned, Some(false), "Remind 차례에 show 가 건 고정을 풀면 안 된다");
    assert!(!o.wake);
    // 다음 차례 — 표식은 지워졌다(No). 15초가 안 지났으니 깨우지 않는다.
    let o = watch_tick(&mut st, tick_in(511, false, true));
    assert!(!o.wake, "되살린 직후 다시 깨우면 안 된다");
    assert_eq!(o.set_pinned, Some(true));
    assert_eq!(st.wakes_without_poll, before);
    // 15초 뒤엔 다시 깨운다.
    assert!(watch_tick(&mut st, tick_in(525, false, true)).wake);
}

/// 🔴 개발 64 — 하트비트가 **실제 처리할 수 있는 방식 전부**를 적어야 한다. MCP 는 이 목록에
/// 없는 방식이면 요청을 아예 안 쓴다 → 여기서 빠뜨리면 새 결제 방식이 조용히 죽고, 반대로
/// 여기에만 적고 `approve_payment` 의 팔을 안 만들면 MCP 가 보낸 요청이 거절된다.
/// 두 곳이 같이 움직이는지 사람이 기억하는 대신 이 검사가 문다.
#[test]
fn heartbeat_advertises_every_kind_we_handle() {
    let kinds = supported_kinds();
    assert!(kinds.contains(&crate::policy::KIND_X402.to_string()), "x402-svm 이 하트비트에 없다");
    // 하트비트 JSON 에 실제로 실리는지(직렬화 누락 방지).
    let json = serde_json::to_string(&Heartbeat {
        ts: 1,
        ui_ok: true,
        kinds: supported_kinds(),
    })
    .unwrap();
    assert!(json.contains("x402"), "{json}");
}


// ── 지갑 폴더를 읽고 쓰는 갈래 ─────────────────────────────

fn rec(id: &str, state: &str, status: &str, tx: &str) -> crate::policy::AttemptRecord {
    crate::policy::AttemptRecord {
        v: 1,
        id: id.into(),
        state: state.into(),
        kind: "transfer".into(),
        chain_id: 8453,
        started: 1,
        updated: 1,
        status: status.into(),
        tx_hash: tx.into(),
        detail: String::new(),
        x402: None,
    }
}

/// 요청 파일을 내건다 — MCP 가 쓰는 것과 같은 자리.
fn put_request(id: &str) {
    let mut r = req_created(now_secs());
    r.id = id.into();
    write_json(request_path().unwrap(), &r).unwrap();
}

/// 🔴 승인 전 한도 검사: 건당 한도(기본 5)를 넘는 x402 요청은 창 없이 「failed + 이유」 로 끝나고 요청 파일이
/// 치워진다. 한도 안의 요청은 손대지 않는다. 다른 kind 는 판정하지 않는다(승인 단계가 거절한다).
#[test]
fn over_limit_request_is_refused_before_the_prompt() {
    let _g = globals();
    let _d = crate::store::test_dir::use_temp("precheck");
    let put = |id: &str, amount: &str, kind: &str| {
        let mut r = req_created(now_secs());
        r.id = id.into();
        r.amount = amount.into();
        r.kind = kind.into();
        write_json(request_path().unwrap(), &r).unwrap();
    };
    put("pc-1", "50", crate::policy::KIND_X402);
    refuse_over_limit();
    let r = written_result().expect("결과가 써져야 한다");
    assert_eq!((r.id.as_str(), r.status.as_str()), ("pc-1", "failed"));
    assert!(r.detail.contains("5 USDC"), "{}", r.detail);
    assert!(read_request().is_none(), "요청 파일이 치워져야 한다");

    let _ = fs::remove_file(result_path().unwrap());
    put("pc-2", "1", crate::policy::KIND_X402);
    refuse_over_limit();
    assert!(written_result().is_none());
    assert_eq!(read_request().map(|r| r.id).as_deref(), Some("pc-2"));

    put("pc-3", "50", "x402");
    refuse_over_limit();
    assert!(written_result().is_none());

    // 옛 형식 지갑(`0x…`)이면 한도 안의 결제도 창 전에 진짜 이유와 함께 돌려보낸다.
    let enc = crate::store::budu_dir().unwrap().join("wallet.enc");
    fs::write(
        &enc,
        r#"{"version":3,"address":"0x036CbD53842c5426634e7929541eC2318f3dCF7e","salt":"AA==","nonce":"AA==","ciphertext":"AA=="}"#,
    )
    .unwrap();
    put("pc-4", "1", crate::policy::KIND_X402);
    refuse_over_limit();
    let r = written_result().expect("옛 형식이면 거절");
    assert_eq!((r.id.as_str(), r.status.as_str()), ("pc-4", "failed"));
    assert!(r.detail.contains("옛 형식") || r.detail.contains("old format"), "{}", r.detail);
    let _ = fs::remove_file(&enc);
}

fn written_result() -> Option<PaymentResult> {
    fs::read_to_string(result_path().unwrap())
        .ok()
        .map(|s| serde_json::from_str(&s).unwrap())
}

/// 🔴 개발 80 — 결과 파일만 못 쓴 승인 뒤 남은 요청에 무엇을 덮어도 되나. 기록 없음·실패·넘김만 덮는다.
/// 끝난 결제(done·unknown)는 그 결과를 돌려주고, 나가는 중(sending)이나 못 읽는 기록은 `Err` — 아무것도 쓰지 말라.
#[test]
fn settled_attempt_result_reads_the_record() {
    let _d = crate::store::test_dir::use_temp("settled");
    assert!(matches!(settled_attempt_result("11"), Ok(None)), "기록 없음");
    for st in [crate::policy::ATTEMPT_FAILED, crate::policy::ATTEMPT_HANDED] {
        write_attempt(&rec("11", st, "", "")).unwrap();
        assert!(matches!(settled_attempt_result("11"), Ok(None)), "{st} 는 다시 승인·거부해도 된다");
    }
    write_attempt(&rec("11", crate::policy::ATTEMPT_SENDING, "", "")).unwrap();
    assert!(matches!(settled_attempt_result("11"), Err(())), "나가는 중");
    write_attempt(&rec("11", crate::policy::ATTEMPT_DONE, "approved", "0xA")).unwrap();
    let r = settled_attempt_result("11").unwrap().expect("끝난 결제");
    assert_eq!((r.status.as_str(), r.tx_hash.as_str()), ("approved", "0xA"));
    write_attempt(&rec("11", crate::policy::ATTEMPT_UNKNOWN, "unknown", "0xB")).unwrap();
    let r = settled_attempt_result("11").unwrap().expect("나갔을 수 있는 결제");
    assert_eq!((r.status.as_str(), r.tx_hash.as_str()), ("unknown", "0xB"));
    // 못 읽는 기록 — 「없음」으로 접으면 나간 돈을 「안 나감」으로 덮는다.
    let p = crate::policy::attempt_path(&budu_dir().unwrap(), "11").unwrap();
    fs::write(&p, "{\"v\":").unwrap();
    assert!(matches!(settled_attempt_result("11"), Err(())), "깨진 기록");
}

/// 🔴 거부 갈래. 끝난 결제의 요청을 사람이 거부하면 「거부됨」이 아니라 진짜 결과를 다시 전하고
/// 거부는 실패로 알린다. 나가는 중이면 결과 칸도 요청도 그대로. 아무것도 시작 안 한 요청만 「거부됨」.
#[test]
fn reject_never_overwrites_a_settled_payment() {
    let _g = globals();
    let _d = crate::store::test_dir::use_temp("reject");

    put_request("21");
    write_attempt(&rec("21", crate::policy::ATTEMPT_DONE, "approved", "0xA")).unwrap();
    let e = reject_payment("21".into(), None).unwrap_err();
    assert!(e.contains("이미 처리") || e.contains("already processed"), "{e}");
    let r = written_result().expect("진짜 결과를 다시 전한다");
    assert_eq!((r.id.as_str(), r.status.as_str(), r.tx_hash.as_str()), ("21", "approved", "0xA"));
    assert!(read_request().is_none(), "결과를 전했으니 요청은 치운다");

    let _ = fs::remove_file(result_path().unwrap());
    put_request("22");
    write_attempt(&rec("22", crate::policy::ATTEMPT_SENDING, "", "")).unwrap();
    let e = reject_payment("22".into(), None).unwrap_err();
    assert!(e.contains("확인하지 못한") || e.contains("couldn't be confirmed"), "{e}");
    assert!(written_result().is_none(), "나가는 중이면 결과 칸을 안 쓴다");
    assert_eq!(read_request().map(|r| r.id).as_deref(), Some("22"), "요청도 그대로");

    put_request("23");
    reject_payment("23".into(), Some("싫어요".into())).unwrap();
    let r = written_result().expect("거부됨");
    assert_eq!((r.id.as_str(), r.status.as_str(), r.detail.as_str()), ("23", "rejected", "싫어요"));
    assert!(read_request().is_none());

    // 진행 중인 승인이 있으면 끝난 기록이 없어도 거부를 안 받는다 — 같은 문.
    put_request("24");
    let _ = fs::remove_file(result_path().unwrap());
    {
        let _a = ApprovalGuard::acquire();
        assert!(reject_payment("24".into(), None).is_err());
    }
    assert!(written_result().is_none());
}

/// 🔴 감시 갈래 — 화면이 죽어 승인 창을 못 띄울 때. 끝난 결제는 결과를 다시 전하고(알림 없음), 나가는 중이면 손대지 않고,
/// 아무것도 시작 안 한 요청만 「실패」(알림).
#[test]
fn dead_window_keeps_settled_and_sending_payments() {
    let _d = crate::store::test_dir::use_temp("dead");
    let (r, notify) = dead_window_result("31".into()).expect("기록 없음 → 실패");
    assert_eq!((r.status.as_str(), notify), ("failed", true));
    write_attempt(&rec("32", crate::policy::ATTEMPT_UNKNOWN, "unknown", "0xC")).unwrap();
    let (r, notify) = dead_window_result("32".into()).expect("불명 → 그 결과");
    assert_eq!((r.status.as_str(), r.tx_hash.as_str(), notify), ("unknown", "0xC", false));
    write_attempt(&rec("33", crate::policy::ATTEMPT_SENDING, "", "")).unwrap();
    assert!(dead_window_result("33".into()).is_none(), "나가는 중 → 손대지 않는다");
}

pub(crate) fn req_created(created: u64) -> PaymentRequest {
    PaymentRequest {
        id: "1".into(),
        token: "USDC".into(),
        to: "0x0".into(),
        amount: "1".into(),
        memo: String::new(),
        created,
        kind: "x402".into(),
        resource: String::new(),
        chain_id: 0,
        account: 0,
        from: String::new(),
        lease: false,
        svm: None,
        job: String::new(),
    }
}

/// 🔴 늦게 끝난 승인은 **결과 칸도 요청 파일도** 건드리면 안 된다
///. 남의 요청을 치우면 single-flight 가 깨지고,
/// 남의 결과를 덮으면 그쪽은 돈이 나갔는데도 시간 초과로 보고 재시도한다.
#[test]
fn resolve_only_touches_its_own_request() {
    assert!(owns_pending(Some("A"), "A"), "같은 요청이면 기록한다");
    assert!(!owns_pending(Some("B"), "A"), "남의 요청은 건드리지 않는다");
    assert!(
        !owns_pending(None, "A"),
        "이미 거둬간 요청이면 남길 것도 없다"
    );
}

/// 승인 카운터는 프로세스 전역이다 — 그걸 보는 테스트끼리는 한 줄로 선다.
static GLOBALS: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn globals() -> std::sync::MutexGuard<'static, ()> {
    GLOBALS.lock().unwrap_or_else(|e| e.into_inner())
}

#[test]
fn approval_guard_marks_in_flight() {
    let _g = globals();
    assert!(!approval_in_flight());
    {
        let _g = ApprovalGuard::acquire();
        assert!(approval_in_flight());
        {
            // 카운터 자체는 겹침을 견딘다 — 겹침을 **거절**하는 건 begin_approval(admit_approval)
            // 몫이고, 여기선 Drop 이 하나씩 정확히 되돌리는지만 본다.
            let _g2 = ApprovalGuard::acquire();
            assert!(approval_in_flight());
        }
        assert!(approval_in_flight(), "안쪽 하나가 끝나도 바깥은 진행 중");
    }
    assert!(!approval_in_flight());
}

/// 🔴 승인 처리는 한 번에 하나다. 진행 중인 승인이 있으면 **같은 요청이든 다른
/// 요청이든** 두 번째 승인을 시작하지 않는다 — 같은 요청이면 같은 돈이 두 번 나가고, 다른
/// 요청(A 전송 중 요청 파일이 거둬져 B 가 생긴 경우)이면 사람 없이 두 건이 나간다.
/// 요청이 사라졌거나 다른 것으로 바뀐 경우는 그 전에 걸린다(감시 스레드·상대의 거두기).
#[test]
fn admit_approval_refuses_overlap() {
    assert!(
        admit_approval(Some("A"), "A", false).is_ok(),
        "대기 중이고 아무도 처리 안 함"
    );
    let same = admit_approval(Some("A"), "A", true).unwrap_err();
    assert!(
        same.contains("처리하는 중") || same.contains("still being processed"),
        "{same}"
    );
    let other = admit_approval(Some("B"), "B", true).unwrap_err();
    assert_eq!(other, same, "다른 요청이어도 진행 중이면 같은 이유로 거절");
    let gone = admit_approval(None, "A", false).unwrap_err();
    assert!(
        gone.contains("취소됐거나") || gone.contains("cancelled"),
        "{gone}"
    );
    let swapped = admit_approval(Some("B"), "A", false).unwrap_err();
    assert_eq!(
        swapped, gone,
        "남의 요청이 대기 중이면 내 요청은 이미 없는 것"
    );
    // 순서: 「요청이 없다」가 「처리 중」보다 먼저 — 진행 중인 것이 남의 것이라도 내 요청은 이미 없다.
    assert_eq!(admit_approval(None, "A", true).unwrap_err(), gone);
}

/// 🔴 죽은 요청 파일 청소의 조건. 하나라도 어긋나면 **안 지운다** —
/// 살아 있는 요청을 지우면 사람이 승인할 수 있는 결제가 사라지고, 승인이 진행 중일 때
/// 지우면 성공한 결제의 결과가 버려진다(`resolve_request` 가 자기 요청을 못 찾는다).
#[test]
fn clears_only_dead_requests_when_nothing_in_flight() {
    let dead = req_created(now_secs() - (APPROVAL_WINDOW_SECS + STALE_GRACE_SECS + 10));
    let alive = req_created(now_secs());
    assert!(
        should_clear_request(Some(&dead), false, false),
        "죽은 요청은 치운다"
    );
    assert!(
        !should_clear_request(Some(&dead), true, false),
        "승인 진행 중이면 죽은 요청도 안 건드린다"
    );
    assert!(
        !should_clear_request(Some(&alive), false, false),
        "살아 있는 요청은 사람이 승인할 것이다"
    );
    assert!(!should_clear_request(None, false, false), "없으면 치울 것도 없다");
    // 🔴 미래 시각(시계가 뒤로 밀림)은 `is_stale` 이 만료로 치지만 **지우지는 않는다** —
    // 방금 만들어져 MCP 가 아직 기다리는 중일 수 있다.
    let future = req_created(now_secs() + STALE_GRACE_SECS + 10);
    assert!(is_stale(&future), "미래 시각은 화면에서 감춘다");
    assert!(
        !should_clear_request(Some(&future), false, false),
        "미래 시각 요청은 늙은 게 아니라 안 지운다"
    );
    // 임대가 오래 끊긴 요청(기다리던 MCP 가 죽음)은 5분을 안 채워도 치운다 — 단 승인 진행 중이면 안 건드린다.
    assert!(should_clear_request(Some(&alive), false, true), "임대가 죽은 요청은 치운다");
    assert!(!should_clear_request(Some(&alive), true, true), "승인 진행 중이면 임대가 죽어도 둔다");
}

// 방금 만들어진 요청은 살아 있다 = 팝오버를 붙잡아야 한다.
#[test]
fn fresh_request_is_not_stale() {
    assert!(!is_stale(&req_created(now_secs())));
}

// 승인 시간(5분) 안이면 아직 살아 있다.
#[test]
fn request_within_window_is_not_stale() {
    assert!(!is_stale(&req_created(
        now_secs() - (APPROVAL_WINDOW_SECS - 10)
    )));
}

// 유예분까지 지나면 죽은 요청 — MCP 가 죽어 남긴 파일이 팝오버를 영영 붙잡지 못하게.
// 남은 승인 시간 — 승인 창 카운트다운과 같은 5분 기준. 유예 구간(살아 있는 요청이지만
// MCP 는 이미 거둬 갔을 수 있는 60초)에선 0 이라 되살리기(appwin::dismissal)가 안 걸린다.
#[test]
fn secs_left_counts_down_from_five_minutes() {
    let r = req_created(1_000);
    assert_eq!(secs_left(&r, 1_000), 300);
    assert_eq!(secs_left(&r, 1_240), 60);
    assert_eq!(secs_left(&r, 1_300), 0);
    assert_eq!(secs_left(&r, 1_350), 0);
    // created 가 미래(시계 오차)면 아직 안 흘렀다고 본다.
    assert_eq!(secs_left(&r, 900), 300);
}

#[test]
fn long_expired_request_is_stale() {
    let old = now_secs() - (APPROVAL_WINDOW_SECS + STALE_GRACE_SECS + 10);
    assert!(is_stale(&req_created(old)));
}

// 살짝 미래(시계 오차 수준)는 그대로 살아 있는 요청으로 본다.
#[test]
fn slightly_future_created_is_not_stale() {
    assert!(!is_stale(&req_created(now_secs() + STALE_GRACE_SECS / 2)));
}

// 유예를 크게 넘는 미래 시각은 만료로 본다 — 경과가 늘 0이라 영원히 안 늙는
// 요청이 팝오버를 영구히 붙잡는 구멍을 막는다(시계가 뒤로 밀린 경우).
#[test]
fn far_future_created_is_stale() {
    assert!(is_stale(&req_created(now_secs() + 10_000)));
}

// 결제 요청 JSON 왕복 — MCP가 쓰는 형식과 호환돼야 한다 (memo 한글 포함).
#[test]
fn payment_request_roundtrip() {
    let r = PaymentRequest {
        id: "1780623842000000000".into(),
        token: "USDC".into(),
        to: "0xabc".into(),
        amount: "1.5".into(),
        memo: "데이터 API 1회 호출".into(),
        created: 1780623842,
        kind: "x402".into(),
        resource: String::new(),
        chain_id: 84_532,
        account: 2,
        from: "0xF39f".into(),
        lease: false,
        svm: None,
        job: String::new(),
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: PaymentRequest = serde_json::from_str(&json).unwrap();
    assert_eq!(back.id, "1780623842000000000");
    assert_eq!((back.account, back.from.as_str()), (2, "0xF39f"));
    assert_eq!(back.token, "USDC");
    assert_eq!(back.memo, "데이터 API 1회 호출");
    assert_eq!(back.created, 1780623842);
    assert_eq!(back.kind, "x402");
}

// kind/resource 가 없는 요청 파일도 읽힌다 — kind 는 빈 값(= 승인 경로가 모르는 방식으로 거절).
#[test]
fn legacy_payment_request_defaults() {
    let json = r#"{"id":"1","token":"USDC","to":"0xabc","amount":"1","memo":"","created":1}"#;
    let r: PaymentRequest = serde_json::from_str(json).unwrap();
    assert_eq!(r.kind, "");
    assert!(r.resource.is_empty());
    assert_eq!(r.chain_id, 0); // 옛 요청엔 chain_id 없음 → 0(=체인 검사 건너뜀, 후방호환)
    assert!(r.from.is_empty()); // 옛 요청엔 계정 각인 없음 → 계정 검사 건너뜀
    assert_eq!(r.account, 0);
}

/// 각인된 요청은 그 계정, 각인 없는(옛) 요청은 계정 0.
#[tokio::test]
async fn request_account_index_stamp_or_zero() {
    let mut r = req_created(1);
    r.account = 3;
    r.from = "0xabc".into();
    assert_eq!(request_account_index(&r), 3);
    // 각인 없는(옛) 요청은 계정 0 — 이미 고정된 값이 있어도 바뀌지 않는다
    // (옛 사이드카는 계정 0 만 안다).
    let legacy = req_created(1);
    assert_eq!(request_account_index(&legacy), 0);
    let got =
        crate::wallet::with_pinned_account(7, async { request_account_index(&legacy) }).await;
    assert_eq!(got, 0);
}

// 결제 결과 JSON 왕복 — GUI가 쓰고 MCP가 읽는 형식.
#[test]
fn payment_result_roundtrip() {
    let r = PaymentResult {
        id: "1".into(),
        status: "approved".into(),
        tx_hash: "0xdeadbeef".into(),
        detail: String::new(),
        x402: None,
    };
    let json = serde_json::to_string(&r).unwrap();
    let back: PaymentResult = serde_json::from_str(&json).unwrap();
    assert_eq!(back.status, "approved");
    assert_eq!(back.tx_hash, "0xdeadbeef");
    assert!(back.detail.is_empty());
    // transfer 결과는 x402 필드를 직렬화하지 않는다(skip_serializing_if).
    assert!(!json.contains("x402"));
}

// x402 결과 JSON 왕복 — GUI가 서명 페이로드를 함께 쓴다.
#[test]
fn x402_payment_result_roundtrip() {
    let r = PaymentResult {
        id: "1".into(),
        status: "approved".into(),
        tx_hash: String::new(),
        detail: String::new(),
        x402: Some(SvmSigned {
            transaction: "AQID".into(),
            signature: "5sig".into(),
            blockhash: "EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k".into(),
            last_valid_block_height: 291_470_237,
        }),
    };
    let json = serde_json::to_string(&r).unwrap();
    assert!(json.contains("\"x402\""));
    let back: PaymentResult = serde_json::from_str(&json).unwrap();
    let p = back.x402.expect("x402 페이로드");
    assert_eq!(p.signature, "5sig");
    assert_eq!(p.transaction, "AQID");
    assert_eq!(p.last_valid_block_height, 291_470_237);
}

// MCP 하트비트 파싱 — client 필드 있/없 둘 다 (옛 형식 호환).
#[test]
fn mcp_alive_parses() {
    let a: McpAlive =
        serde_json::from_str(r#"{"ts":1780775675,"client":"claude-code"}"#).unwrap();
    assert_eq!(a.ts, 1780775675);
    assert_eq!(a.client, "claude-code");
    // client 없으면 기본 빈 문자열.
    let b: McpAlive = serde_json::from_str(r#"{"ts":1}"#).unwrap();
    assert!(b.client.is_empty());
}
