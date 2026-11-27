use super::*;

const SIG: &str = "5b7M6aadukZHvymevqxJFHyHFTEMTuLTHq1vUMXMf9FMsfchfirhnti9YrnfuA1mocTpQ9RVdrf3rZ2pqwpueXNj";

fn signed_row(status: &str) -> HistoryEntry {
    HistoryEntry {
        status: status.into(),
        detail: SIG.into(),
        valid_until: 100,
        amount: "0.01".into(),
        ..Default::default()
    }
}

/// 「내용 못 받음」 표시만 하는 정산 기록은 상태를 안 바꾸고 표시만 붙인다. 정상 정산에 실린 표시도 남는다.
#[test]
fn undelivered_marker_only_marks() {
    let mut list = vec![signed_row("signed")];
    let mark = Settlement { nonce: SIG.into(), tx: String::new(), success: false, undelivered: true, delivered: false };
    assert!(apply_settlement(&mut list, &mark));
    assert_eq!(list[0].status, "signed");
    assert!(list[0].undelivered);
    // 이미 만료로 닫힌 기록에도 표시는 붙지만 상태는 그대로.
    let mut done = vec![signed_row("expired")];
    assert!(apply_settlement(&mut done, &mark));
    assert_eq!(done[0].status, "expired");
    let mut list = vec![signed_row("signed")];
    let ok = Settlement { nonce: SIG.into(), tx: "TX".into(), success: true, undelivered: true, delivered: false };
    assert!(apply_settlement(&mut list, &ok));
    assert_eq!((list[0].status.as_str(), list[0].undelivered), ("settled", true));
}

/// 체인 결말은 확인할 기록에만 한 번 적힌다 — 확인한 기록·높이 없는 옛 기록·차단 기록은 그대로.
#[test]
fn chain_verdict_applies_once_to_candidates() {
    use crate::confirm::Verdict;
    let mut list = vec![signed_row("settled")];
    let got = apply_verdict(&mut list, SIG, &Verdict::Settled { tx: "REAL".into() }).unwrap();
    assert_eq!((got.status.as_str(), got.settle_tx.as_str(), got.checked), ("settled", "REAL", true));
    assert!(apply_verdict(&mut list, SIG, &Verdict::Expired).is_none(), "확인한 기록은 다시 안 바꾼다");
    let mut list = vec![signed_row("signed")];
    assert_eq!(apply_verdict(&mut list, SIG, &Verdict::Expired).unwrap().status, "expired");
    let mut list = vec![signed_row("settle_failed")];
    assert_eq!(apply_verdict(&mut list, SIG, &Verdict::Reverted { tx: "T".into() }).unwrap().status, "reverted");
    let mut old = signed_row("signed");
    old.valid_until = 0;
    assert!(!needs_chain_check(&old), "높이 없는 옛 기록은 판정 못 한다");
    let mut blocked = signed_row("blocked");
    blocked.detail = "사유".into();
    assert!(!needs_chain_check(&blocked));
}

/// 🔴 정산 가져오기 — 가져간 뒤 새로 쓰인 정산은 다음 번에 잡히고, 같은 순간에
/// 두 번 가져가도 겹치지 않으며, 못 읽은 묶음은 지우지 않는다.
#[test]
fn settlements_are_claimed_without_loss() {
    let dir = std::env::temp_dir().join(format!("budu-settle-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let path = dir.join("x402_settlements.json");
    let one = |n: &str| format!(r#"[{{"nonce":"{n}","tx":"0x1","success":true}}]"#);

    fs::write(&path, one("A")).unwrap();
    let (s1, c1) = claim_settlements(&path);
    assert_eq!(s1.len(), 1);
    // 같은 초에 새 정산 B 가 쓰이고 또 가져간다 — A 의 묶음을 덮지 않는다(아직 안 지웠어도).
    fs::write(&path, one("B")).unwrap();
    let (s2, _) = claim_settlements(&path);
    let nonces: Vec<_> = s2.iter().map(|s| s.nonce.as_str()).collect();
    assert!(nonces.contains(&"A") && nonces.contains(&"B"), "{nonces:?}");
    for p in c1 {
        let _ = fs::remove_file(p);
    }
    // 못 읽는 묶음(여기선 같은 접두어의 디렉터리)은 소비 목록에 안 들어간다 = 지우지 않는다.
    let stuck = dir.join(".x402_settlements.json.claimed.0.stuck");
    fs::create_dir_all(&stuck).unwrap();
    let (_, c3) = claim_settlements(&path);
    assert!(!c3.contains(&stuck));
    let _ = fs::remove_dir_all(&dir);
}

fn entry(tag: &str) -> HistoryEntry {
    HistoryEntry {
        ts: 0,
        token: "USDC".into(),
        to: "0x0".into(),
        amount: "1".into(),
        status: "sent".into(),
        detail: tag.into(),
        settle_tx: String::new(),
        ..Default::default()
    }
}

fn temp_hot(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("budu-hist-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir.join("history-8453.json")
}

fn tags(list: &[HistoryEntry]) -> Vec<String> {
    list.iter().map(|e| e.detail.clone()).collect()
}

/// durable 기록의 두 실패 — 못 썼으면 본 파일 그대로(`NotWritten`), 디렉터리 동기화만 실패면 줄은 제자리(`InPlace`).
/// 호출자(먼저 적기)는 앞쪽이면 「줄 없음 → 지금 환불」, 뒤쪽이면 「그 id 로 마무리」를 고른다 — 그 판단의 전제다.
#[test]
fn durable_record_failures_say_where_the_row_is() {
    use crate::store::fault::{next, Fail};
    let hot = temp_hot("durable");
    record_at(&hot, entry("old"), 10).unwrap();
    next(Fail::NotWritten);
    assert!(matches!(record_at_with(&hot, entry("a"), 10, true), Err(RecordError::NotWritten(_))));
    assert_eq!(tags(&read_history_at(&hot)), ["old"]);
    next(Fail::NotDurable);
    assert!(matches!(record_at_with(&hot, entry("b"), 10, true), Err(RecordError::InPlace(_))));
    assert_eq!(tags(&read_history_at(&hot)), ["b", "old"]);
    // 보통 쓰기(durable 아님)는 주입을 안 쓴다 — 다음 durable 쓰기에 남는다.
    next(Fail::NotWritten);
    record_at(&hot, entry("c"), 10).unwrap();
    assert!(record_at_with(&hot, entry("d"), 10, true).is_err());
    let _ = fs::remove_dir_all(hot.parent().unwrap());
}

/// 먼저 적기를 못 한 결제의 메모리 표식은 빠지고, 뒤에 적는 기록 실패는 남긴다.
#[test]
fn forget_recent_drops_only_that_id() {
    let mut a = entry("a");
    a.id = "budu80-forget-a".into();
    a.ts = now_secs();
    let mut b = a.clone();
    b.id = "budu80-forget-b".into();
    remember_recent(&a);
    remember_recent(&b);
    forget_recent(&a.id);
    let ids: Vec<_> = recent_in_memory().into_iter().map(|e| e.id).collect();
    assert!(!ids.contains(&a.id) && ids.contains(&b.id), "{ids:?}");
    forget_recent(&b.id);
}

/// 🔴 개발 70: 200건을 넘친 기록은 지워지지 않고 보관 파일로 간다 — 본 파일은 최신 cap 건, 나머지는 보관 파일에
/// 오래된 순으로, 둘을 이어 읽으면 빠짐없이 최신순.
#[test]
fn overflow_moves_to_archive_without_loss() {
    let hot = temp_hot("overflow");
    for i in 0..7 {
        record_at(&hot, entry(&i.to_string()), 3).unwrap();
    }
    assert_eq!(tags(&read_history_at(&hot)), ["6", "5", "4"]);
    let raw = fs::read_to_string(crate::policy::history_archive_path(&hot)).unwrap();
    assert_eq!(raw.lines().count(), 4, "{raw}");
    assert_eq!(
        tags(&crate::policy::read_sent_history(&hot, 100, "")),
        ["6", "5", "4", "3", "2", "1", "0"]
    );
    // 본 파일로 충분하면 딱 그만큼, 모자라면 보관 파일에서 이어서.
    assert_eq!(
        tags(&crate::policy::read_sent_history(&hot, 2, "")),
        ["6", "5"]
    );
    assert_eq!(
        tags(&crate::policy::read_sent_history(&hot, 5, "")),
        ["6", "5", "4", "3", "2"]
    );
    let _ = fs::remove_dir_all(hot.parent().unwrap());
}

/// 덧붙인 뒤 본 파일을 쓰기 전에 죽은 경우 — 같은 기록이 본 파일 끝과 보관 파일 끝에 다 있다. 다음 번에 또
/// 밀려나도 두 번 적히지 않는다. 쓰다 죽은 반쪽 줄은 건너뛰고, 그 뒤에 붙인 줄은 살아 있다.
#[test]
fn crash_between_append_and_write_does_not_duplicate() {
    let hot = temp_hot("crash");
    for i in 0..4 {
        record_at(&hot, entry(&i.to_string()), 3).unwrap();
    }
    // 본 파일 = [3,2,1], 보관 = [0]. 「1」 을 보관 파일에 덧붙이고 죽었다고 치자.
    let archive = crate::policy::history_archive_path(&hot);
    let mut raw = fs::read_to_string(&archive).unwrap();
    raw.push_str(&serde_json::to_string(&entry("1")).unwrap());
    raw.push('\n');
    fs::write(&archive, &raw).unwrap();
    record_at(&hot, entry("4"), 3).unwrap();
    assert_eq!(
        tags(&crate::policy::read_sent_history(&hot, 100, "")),
        ["4", "3", "2", "1", "0"]
    );
    // 반쪽 줄(개행 없이 끝남) 뒤에 덧붙여도 새 줄은 멀쩡하다.
    let mut raw = fs::read_to_string(&archive).unwrap();
    raw.push_str(r#"{"ts":0,"tok"#);
    fs::write(&archive, &raw).unwrap();
    record_at(&hot, entry("5"), 3).unwrap();
    assert_eq!(
        tags(&crate::policy::read_sent_history(&hot, 100, "")),
        ["5", "4", "3", "2", "1", "0"]
    );
    let _ = fs::remove_dir_all(hot.parent().unwrap());
}

/// 🔴 여러 건을 한꺼번에 옮기다 죽어도(본 파일이 cap 을 넘겨 있던 경우) 겹친 구간 전체를
/// 알아보고 새 것만 붙인다. 한글 사유가 있어 끝 읽기가 글자 중간에서 잘려도 같다.
#[test]
fn multi_evict_crash_does_not_duplicate() {
    // 덧대는 바이트를 1·2·3 으로 — 끝 읽기의 자른 자리가 3바이트 글자의 세 위상을 다 밟는다(적어도 한 번은 글자 중간).
    for pad in ["x", "xx", "xxx"] {
        let hot = temp_hot(&format!("multi{}", pad.len()));
        let e = |t: &str| entry(&format!("{t} 한글 사유"));
        // 본 파일이 cap(2)을 넘겨 5건 = 보관에 실패하던 뒤.
        let list: Vec<HistoryEntry> = ["4", "3", "2", "1", "0"].iter().map(|t| e(t)).collect();
        write_json(hot.clone(), &list).unwrap();
        // 맨 앞에 256KB 넘는 한글 기록(뒤에 덧대 JSON 이 아니게 = 읽을 땐 걸러진다), 그 뒤
        // 「0·1·2」 를 보관 파일에 덧붙이고 본 파일을 쓰기 전에 죽었다.
        let archive = crate::policy::history_archive_path(&hot);
        let mut raw =
            serde_json::to_string(&entry(&format!("f {}", "가".repeat(90_000)))).unwrap();
        raw.push_str(pad);
        raw.push('\n');
        for t in ["0", "1", "2"] {
            raw.push_str(&serde_json::to_string(&e(t)).unwrap());
            raw.push('\n');
        }
        fs::write(&archive, raw).unwrap();
        record_at(&hot, e("5"), 2).unwrap();
        let got: Vec<String> = crate::policy::read_sent_history(&hot, 100, "")
            .iter()
            .map(|x| x.detail.split(' ').next().unwrap().to_string())
            .collect();
        assert_eq!(got, ["5", "4", "3", "2", "1", "0"], "pad {pad}");
        let _ = fs::remove_dir_all(hot.parent().unwrap());
    }
}

/// 🔴 개발 72: 보관 파일 끝부분만 읽었으면, 그 가장 오래된 기록이 10분 창 밖이어야 「창을 다 봤다」.
#[test]
fn archive_tail_must_cover_window() {
    let at = |ts: u64| HistoryEntry {
        ts,
        ..Default::default()
    };
    let now = 10_000;
    // 파일 전체를 읽었으면 기록이 없어도 덮는다.
    assert!(archive_covers_window(&[], true, now, 600));
    // 끝부분만 — 가장 오래된 게 창 안이면 앞쪽에 창 안 기록이 더 있을 수 있다.
    assert!(!archive_covers_window(&[at(now - 100), at(now - 5)], false, now, 600));
    assert!(!archive_covers_window(&[], false, now, 600));
    // 정확히 창 길이 전은 아직 창 안(`last_same_payment` 와 같은 경계) — 앞쪽에 같은 초가 더 있을 수 있다.
    assert!(!archive_covers_window(&[at(now - 600), at(now - 5)], false, now, 600));
    assert!(archive_covers_window(&[at(now - 601), at(now - 5)], false, now, 600));
}

/// 🔴 개발 72: 보관 파일은 「없음」과 「있는데 못 읽음」을 가른다.
#[test]
fn archive_tail_checked_separates_missing_from_unreadable() {
    let dir = std::env::temp_dir().join(format!("budu-archive-{}-{}", std::process::id(), now_secs()));
    fs::create_dir_all(&dir).unwrap();
    // 없음 → 빈 목록, 전체를 본 셈.
    let (v, whole) = archive_tail_checked(&dir.join("none.jsonl")).unwrap();
    assert!(v.is_empty() && whole);
    // 그 자리에 디렉터리 → 있는데 못 읽음.
    let blocked = dir.join("blocked.jsonl");
    fs::create_dir_all(&blocked).unwrap();
    assert!(archive_tail_checked(&blocked).is_err());
    // 작은 파일 → 전체.
    let small = dir.join("small.jsonl");
    let one = HistoryEntry {
        ts: 1,
        status: "sent".into(),
        ..Default::default()
    };
    fs::write(&small, format!("{}\n", serde_json::to_string(&one).unwrap())).unwrap();
    let (v, whole) = archive_tail_checked(&small).unwrap();
    assert_eq!(v.len(), 1);
    assert!(whole);
    // 🔴 개발 73: 깨진 줄이 하나라도 있으면 못 읽음 — 관대한 쪽(`archive_tail`)만 건너뛴다.
    let line = serde_json::to_string(&one).unwrap();
    let broken = dir.join("broken.jsonl");
    fs::write(&broken, format!("{line}\n{{\"ts\":2,\"sta\n{line}\n")).unwrap();
    assert!(archive_tail_checked(&broken).is_err());
    assert_eq!(archive_tail(&broken).len(), 2);
    // 전체를 읽었으면 첫 줄이 깨져도 못 읽음.
    fs::write(&broken, format!("{{\"ts\"\n{line}\n")).unwrap();
    assert!(archive_tail_checked(&broken).is_err());
    // 256KB 로 자른 자리의 첫 줄(원래 반쪽)은 봐준다 — 나머지가 온전하면 읽힌다.
    let big = dir.join("big.jsonl");
    let mut body = String::new();
    while body.len() < 300 * 1024 {
        body.push_str(&line);
        body.push('\n');
    }
    fs::write(&big, &body).unwrap();
    let (v, whole) = archive_tail_checked(&big).unwrap();
    assert!(!whole && !v.is_empty());
    let _ = fs::remove_dir_all(&dir);
}

/// 체인 확인이 정산 tx 까지 먼저 채운 뒤 MCP 정산이 늦게 오면 — 「내용 못 받음」 은 남고 «받음» 은 안 생긴다.
/// 반대로 늦게 온 정산이 «받음» 이면 그것만 얹는다(상태·tx 는 체인이 정한 그대로).
#[test]
fn late_settlement_after_chain_check_keeps_delivery_facts() {
    let chain_settled = || HistoryEntry {
        ts: 1,
        token: "USDC".into(),
        to: "X".into(),
        amount: "0.01".into(),
        status: "settled".into(),
        detail: "SIG".into(),
        settle_tx: "CHAINTX".into(),
        checked: true,
        ..Default::default()
    };
    let mut list = vec![chain_settled()];
    let late_lost = Settlement { nonce: "SIG".into(), tx: "SELLERTX".into(), success: true, undelivered: true, delivered: false };
    assert!(apply_settlement(&mut list, &late_lost));
    assert!(list[0].undelivered && !list[0].delivered);
    assert_eq!((list[0].status.as_str(), list[0].settle_tx.as_str()), ("settled", "CHAINTX"));

    let mut list = vec![chain_settled()];
    let late_ok = Settlement { nonce: "SIG".into(), tx: "SELLERTX".into(), success: true, undelivered: false, delivered: true };
    assert!(apply_settlement(&mut list, &late_ok));
    assert!(list[0].delivered && !list[0].undelivered);

    // 「signed」 에 성공+받음 → 정산됨·받음. 받음과 못 받음이 같이 오면 못 받음이 이긴다.
    let mut signed = chain_settled();
    signed.status = "signed".into();
    signed.settle_tx.clear();
    let mut list = vec![signed.clone()];
    assert!(apply_settlement(&mut list, &late_ok));
    assert!(list[0].delivered);
    let mut list = vec![signed];
    let both = Settlement { nonce: "SIG".into(), tx: "T".into(), success: true, undelivered: true, delivered: true };
    assert!(apply_settlement(&mut list, &both));
    assert!(!list[0].delivered && list[0].undelivered);
}

/// 🔴 개발 71: 같은 결제를 10분 안에 두 번 — 나간(또는 나갔을 수 있는) 기록만, 같은 토큰·받는 곳(정확히 — base58)·금액(숫자로)만.
#[test]
fn same_payment_within_window() {
    let parse = |a: &str| crate::limits::parse_usdc_nonneg(a, 6).ok();
    let rec = |ts: u64, status: &str, to: &str, amount: &str| HistoryEntry {
        ts,
        token: "USDC".into(),
        to: to.into(),
        amount: amount.into(),
        status: status.into(),
        ..Default::default()
    };
    let now = 10_000;
    let hit = |list: &[HistoryEntry], amount: &str| {
        last_same_payment(list, "USDC", "0xabc", amount, now, 600, None, parse)
    };
    // 결제 대상 URL 을 주면 그 URL 까지 같은 기록만 같은 결제다(작업 예산 — /wave 와 /tide 는 값이 같아도 다른 결제).
    let mut wave = rec(now - 30, "settled", "0xabc", "0.01");
    wave.resource = "http://localhost:4021/wave".into();
    wave.delivered = true;
    let by = |r: &str| last_same_payment(std::slice::from_ref(&wave), "USDC", "0xabc", "0.01", now, 600, Some(r), parse);
    // 작업 예산은 앞의 같은 결제가 «정산됨 + 내용 받음» 이면 새로 사는 것으로 본다(통과).
    assert_eq!(by("http://localhost:4021/wave"), None);
    assert_eq!(by("http://localhost:4021/tide"), None);
    // 내용을 못 받았거나 결말이 아직이면(정산 대기·정산 실패·결과 모름) 재시도일 수 있다 → 사람에게.
    let job_hit = |e: &HistoryEntry| {
        last_same_payment(std::slice::from_ref(e), "USDC", "0xabc", "0.01", now, 600, Some("http://localhost:4021/wave"), parse)
    };
    let mut lost = wave.clone();
    lost.undelivered = true;
    assert_eq!(job_hit(&lost), Some(30));
    // «받음» 신호가 없으면(옛 MCP·본문을 읽다 죽음·체인 확인만 있음) 받았다고 치지 않는다.
    let mut silent = wave.clone();
    silent.delivered = false;
    assert_eq!(job_hit(&silent), Some(30));
    for s in ["signed", "settle_failed", "unknown", "sent"] {
        let mut w = wave.clone();
        w.status = s.into();
        assert_eq!(job_hit(&w), Some(30), "{s}");
    }
    // 자율 결제(URL 없음)는 정산·내용 받음이어도 지금처럼 사람에게.
    assert_eq!(
        last_same_payment(std::slice::from_ref(&wave), "USDC", "0xabc", "0.01", now, 600, None, parse),
        Some(30)
    );
    assert_eq!(
        hit(&[rec(now - 30, "sent", " 0xabc ", "0.01")], "0.010"),
        Some(30)
    );
    // 대소문자만 다른 받는 곳은 다른 주소다(base58).
    assert_eq!(hit(&[rec(now - 30, "sent", "0xAbC", "0.01")], "0.01"), None);
    for s in ["signed", "settled", "unknown", "settle_failed"] {
        assert_eq!(
            hit(&[rec(now - 5, s, "0xabc", "0.01")], "0.01"),
            Some(5),
            "{s}"
        );
    }
    // 안 나간 것·창 밖·다른 금액·다른 곳·다른 토큰은 아니다.
    for s in [
        "blocked",
        "failed",
        "reverted",
        "expired",
        "received",
    ] {
        assert_eq!(
            hit(&[rec(now - 5, s, "0xabc", "0.01")], "0.01"),
            None,
            "{s}"
        );
    }
    assert_eq!(
        hit(&[rec(now - 601, "sent", "0xabc", "0.01")], "0.01"),
        None
    );
    assert_eq!(
        hit(&[rec(now - 600, "sent", "0xabc", "0.01")], "0.01"),
        Some(600)
    );
    assert_eq!(hit(&[rec(now - 5, "sent", "0xabc", "0.02")], "0.01"), None);
    // 🔴 개발 73: 기록 금액이 깨졌으면 같은 것으로 본다(모르면 사람에게).
    assert_eq!(hit(&[rec(now - 5, "sent", "0xabc", "abc")], "0.01"), Some(5));
    assert_eq!(hit(&[rec(now - 5, "sent", "0xabd", "0.01")], "0.01"), None);
    let mut eth = rec(now - 5, "sent", "0xabc", "0.01");
    eth.token = "ETH".into();
    assert_eq!(hit(&[eth], "0.01"), None);
    // 여러 건이면 가장 최근.
    assert_eq!(
        hit(
            &[
                rec(now - 400, "sent", "0xabc", "0.01"),
                rec(now - 90, "unknown", "0xabc", "0.01")
            ],
            "0.01"
        ),
        Some(90)
    );
}

/// 🔴 개발 71: 모든 필드가 같은 기록(같은 초·금액·사유)이 연달아 밀려나도, 고유 id 가
/// 달라 보관 파일의 끝과 「겹쳤다」고 보지 않는다 — 진짜 기록을 건너뛰지 않는다.
#[test]
fn identical_records_with_ids_are_not_skipped() {
    let hot = temp_hot("same");
    let same = || {
        let mut e = entry("같은 사유");
        e.id = new_record_id();
        e
    };
    for _ in 0..6 {
        record_at(&hot, same(), 2).unwrap();
    }
    assert_eq!(crate::policy::read_sent_history(&hot, 100, "").len(), 6);
    // 대조군: id 가 없는 옛 기록끼리는 여전히 하나로 보인다(그래서 id 가 필요했다).
    let old = temp_hot("same-old");
    for _ in 0..6 {
        record_at(&old, entry("같은 사유"), 2).unwrap();
    }
    assert!(crate::policy::read_sent_history(&old, 100, "").len() < 6);
    let _ = fs::remove_dir_all(hot.parent().unwrap());
    let _ = fs::remove_dir_all(old.parent().unwrap());
}

/// 🔴 개발 71: 주인이 다른 기록은 안 보인다(본 파일·보관 파일 둘 다). 주인이 비어 있는
/// 옛 기록은 보인다(누구 것인지 모른다 — 예전과 같다). 🔴 대소문자는 **가린다**(base58).
#[test]
fn other_owners_records_are_hidden() {
    let hot = temp_hot("owner");
    let by = |tag: &str, from: &str| {
        let mut e = entry(tag);
        e.from = from.into();
        e.id = new_record_id();
        e
    };
    record_at(&hot, by("old-seed", "0xAAAA"), 2).unwrap();
    record_at(&hot, by("legacy", ""), 2).unwrap();
    record_at(&hot, by("mine-1", "0xBbBb"), 2).unwrap();
    record_at(&hot, by("case-twin", "0xBBBB"), 2).unwrap();
    record_at(&hot, by("mine-2", "0xBbBb"), 2).unwrap();
    assert_eq!(
        tags(&crate::policy::read_sent_history(&hot, 100, "0xBbBb")),
        ["mine-2", "mine-1", "legacy"]
    );
    assert_eq!(crate::policy::read_sent_history(&hot, 100, "").len(), 5);
    let _ = fs::remove_dir_all(hot.parent().unwrap());
}

/// 🔴 개발 70: 깨진 본 파일을 빈 목록으로 읽고 덮어쓰지 않는다 — 옆으로 치우고 새로 시작.
/// 🔴 개발 73: 깨진 내역을 10분 안에 치웠으면 중복 검사는 증명 못 한다 — 치운 파일에 방금 결제가 있었을 수 있다.
#[test]
fn recently_set_aside_history_needs_a_human() {
    let hot = temp_hot("aside");
    let dir = hot.parent().unwrap().to_path_buf();
    let stem = hot.file_stem().unwrap().to_string_lossy().into_owned();
    let now = 100_000;
    assert!(!set_aside_recently(&hot, now, 600));
    fs::write(dir.join(format!("{stem}.broken.{}.json", now - 700)), "x").unwrap();
    assert!(!set_aside_recently(&hot, now, 600)); // 창 밖
    fs::write(dir.join(format!("{stem}.broken.{}.json", now - 30)), "x").unwrap();
    assert!(set_aside_recently(&hot, now, 600));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn broken_history_is_set_aside_not_overwritten() {
    let hot = temp_hot("broken");
    fs::write(&hot, "[{\"ts\":1,").unwrap();
    record_at(&hot, entry("new"), 3).unwrap();
    assert_eq!(tags(&read_history_at(&hot)), ["new"]);
    let aside: Vec<_> = fs::read_dir(hot.parent().unwrap())
        .unwrap()
        .flatten()
        .filter(|e| e.file_name().to_string_lossy().contains(".broken."))
        .collect();
    assert_eq!(aside.len(), 1);
    assert_eq!(fs::read_to_string(aside[0].path()).unwrap(), "[{\"ts\":1,");
    let _ = fs::remove_dir_all(hot.parent().unwrap());
}

/// 🔴 개발 70: 정산이 오기 전에 「signed」 가 보관 파일로 밀려났어도 정산이 반영된다.
#[test]
fn settlement_reaches_archived_signed_entry() {
    let hot = temp_hot("settle-archive");
    let mut signed = entry("0xnonce");
    signed.status = "signed".into();
    record_at(&hot, signed, 2).unwrap();
    for i in 0..3 {
        record_at(&hot, entry(&i.to_string()), 2).unwrap();
    }
    let archive = crate::policy::history_archive_path(&hot);
    let s = Settlement {
        nonce: "0xnonce".into(),
        tx: "0xsettle".into(),
        success: true,
        undelivered: false, delivered: false,
    };
    let mut pending = vec![&s];
    assert_eq!(settle_in_archive(&archive, &mut pending).unwrap(), 1);
    assert!(pending.is_empty());
    let all = crate::policy::read_sent_history(&hot, 100, "");
    let got = all.iter().find(|e| e.detail == "0xnonce").unwrap();
    assert_eq!(
        (got.status.as_str(), got.settle_tx.as_str()),
        ("settled", "0xsettle")
    );
    assert_eq!(all.len(), 4);
    // 없는 보관 파일은 0, 에러 아님.
    let none = hot.with_file_name("nope.archive.jsonl");
    assert_eq!(settle_in_archive(&none, &mut vec![&s]).unwrap(), 0);
    let _ = fs::remove_dir_all(hot.parent().unwrap());
}

/// 보관 파일에 못 쓰면 자르지 않는다 — 본 파일이 cap 을 넘길 뿐 잃지 않는다.
#[test]
fn archive_failure_keeps_everything_in_hot() {
    let hot = temp_hot("nofail");
    // 보관 파일 자리에 디렉터리 = 열 수 없다.
    fs::create_dir_all(crate::policy::history_archive_path(&hot)).unwrap();
    for i in 0..5 {
        record_at(&hot, entry(&i.to_string()), 3).unwrap();
    }
    assert_eq!(tags(&read_history_at(&hot)), ["4", "3", "2", "1", "0"]);
    let _ = fs::remove_dir_all(hot.parent().unwrap());
}

// 거래 내역 항목 JSON 왕복 (한글 사유 포함).
#[test]
fn history_entry_roundtrip() {
    let e = HistoryEntry {
        ts: 123,
        token: "ETH".into(),
        to: "0xabc".into(),
        amount: "0.01".into(),
        status: "blocked".into(),
        detail: "긴급 잠금".into(),
        settle_tx: String::new(),
        ..Default::default()
    };
    let json = serde_json::to_string(&e).unwrap();
    let back: HistoryEntry = serde_json::from_str(&json).unwrap();
    assert_eq!(back.ts, 123);
    assert_eq!(back.status, "blocked");
    assert_eq!(back.detail, "긴급 잠금");
}

// 옛 history.json(settle_tx 필드 없음)도 무손실 로드.
#[test]
fn old_history_without_settle_tx_loads() {
    let old = r#"{"ts":1,"token":"USDC","to":"0xabc","amount":"0.01","status":"signed","detail":"0xnonce"}"#;
    let e: HistoryEntry = serde_json::from_str(old).unwrap();
    assert_eq!(e.status, "signed");
    assert_eq!(e.settle_tx, ""); // 새 필드 기본값
}

// x402 정산 적용: 매칭되는 "signed" 항목만 "settled"+tx 로 바뀐다.
#[test]
fn apply_settlement_updates_matching_signed_entry() {
    let mut list = vec![
        HistoryEntry {
            ts: 1,
            token: "USDC".into(),
            to: "0xpay".into(),
            amount: "0.01".into(),
            status: "signed".into(),
            detail: "0xNONCE".into(),
            settle_tx: String::new(),
            ..Default::default()
        },
        HistoryEntry {
            ts: 2,
            token: "USDC".into(),
            to: "0xother".into(),
            amount: "0.5".into(),
            status: "sent".into(),
            detail: "0xtxhash".into(),
            settle_tx: String::new(),
            ..Default::default()
        },
    ];
    // 매칭 성공 → settled
    let ok = Settlement {
        nonce: "0xNONCE".into(),
        tx: "0xSETTLE".into(),
        success: true,
        undelivered: false, delivered: false,
    };
    assert!(apply_settlement(&mut list, &ok));
    assert_eq!(list[0].status, "settled");
    assert_eq!(list[0].settle_tx, "0xSETTLE");
    assert_eq!(list[1].status, "sent"); // 무관한 항목 불변

    // 같은 nonce 재적용 → 바뀌는 게 없어 false(중복 방지)
    assert!(!apply_settlement(&mut list, &ok));

    // 매칭 없는 nonce → false
    let miss = Settlement {
        nonce: "0xZZZ".into(),
        tx: "0xT".into(),
        success: true,
        undelivered: false, delivered: false,
    };
    assert!(!apply_settlement(&mut list, &miss));

    // 정산 실패 → settle_failed
    let mut list2 = vec![HistoryEntry {
        ts: 1,
        token: "USDC".into(),
        to: "0xpay".into(),
        amount: "0.01".into(),
        status: "signed".into(),
        detail: "0xN2".into(),
        settle_tx: String::new(),
        ..Default::default()
    }];
    let fail = Settlement {
        nonce: "0xN2".into(),
        tx: "0xT2".into(),
        success: false,
        undelivered: false, delivered: false,
    };
    assert!(apply_settlement(&mut list2, &fail));
    assert_eq!(list2[0].status, "settle_failed");
}

// 없는 내역 파일은 「못 읽음」이 아니다 — 깨진 파일만(정산 묶음을 남길지 가른다).
#[test]
fn history_unreadable_only_when_present_and_broken() {
    let dir = std::env::temp_dir().join(format!("budu-hist-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let p = dir.join("history.json");
    assert!(!history_unreadable(&p));
    fs::write(&p, "[]").unwrap();
    assert!(!history_unreadable(&p));
    fs::write(&p, "[{ 반쪽").unwrap();
    assert!(history_unreadable(&p));
    let _ = fs::remove_dir_all(&dir);
}

// 과거에 기록된 비redact detail(RPC URL·키)은 출력 시점에 가려진다.
#[test]
fn get_history_redacts_leaked_url_in_detail() {
    let list = vec![HistoryEntry {
        ts: 1,
        token: "ETH".into(),
        to: "0xabc".into(),
        amount: "0".into(),
        status: "failed".into(),
        detail: "RPC 연결 실패: https://base.alchemy.com/v2/LEAKEDKEY".into(),
        settle_tx: String::new(),
        ..Default::default()
    }];
    let out = redact_details(list);
    assert!(
        !out[0].detail.contains("LEAKEDKEY"),
        "키가 남음: {}",
        out[0].detail
    );
    assert_eq!(out[0].detail, "RPC 연결 실패: [RPC]");
}
