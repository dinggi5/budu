/// 승인 창 한도 상황: 남는 한도·무제한(빈 값)·넘으면 Err.
#[test]
fn room_for_shows_what_is_left() {
    let u = |n: u64| u128::from(n);
    let r = room_for(u(10_000), u(5_000_000), u(20_000_000), u(1_500_000), 6).unwrap();
    assert_eq!(
        r,
        LimitRoom {
            single: "5".into(),
            daily: "20".into(),
            spent: "1.5".into(),
            left_after: "18.49".into(),
        }
    );
    // 무제한(0)은 빈 값
    let r = room_for(u(10_000), 0, 0, 0, 6).unwrap();
    assert_eq!((r.single.as_str(), r.daily.as_str(), r.left_after.as_str()), ("", "", ""));
    assert_eq!(r.spent, "0");
    // 건당 초과·오늘 초과는 Err
    assert!(room_for(u(6_000_000), u(5_000_000), 0, 0, 6).is_err());
    assert!(room_for(u(2_000_000), 0, u(20_000_000), u(19_000_000), 6).is_err());
    // 딱 맞으면 통과, 남는 한도 0
    let r = room_for(u(1_000_000), 0, u(20_000_000), u(19_000_000), 6).unwrap();
    assert_eq!(r.left_after, "0");
}

/// 감기는 거대 금액은 거부, 경계 바로 아래는 통과.
#[test]
fn huge_amounts_are_refused_not_wrapped() {
    let wrap = "1809251394333065553493296640760748560207343510400633813116524750123642650625";
    assert!(parse_usdc_nonneg(wrap, 6).is_err());
    assert!(parse_usdc_nonneg(&format!("{wrap}.5"), 6).is_err());
    // 금액이 u128 — 정수+소수 38자리까지. 32 + 6 = 38 은 받고(넘치지 않는다), 한 자리 더는 거부.
    let edge = "9".repeat(32);
    assert_eq!(parse_usdc_nonneg(&edge, 6).unwrap().to_string(), format!("{edge}000000"));
    assert!(parse_usdc_nonneg(&format!("{edge}9"), 6).is_err());
    // 앞의 0 은 자리로 안 센다.
    assert_eq!(parse_usdc_nonneg(&format!("{}1.5", "0".repeat(90)), 6).unwrap(), u128::from(1_500_000u64));
}

use super::*;

// 숫자가 없는 글자는 0 = 무제한이 되지 않는다 — alloy 는 "." 를 0 으로 읽었다.
#[test]
fn digitless_input_is_not_zero() {
    for s in [".", "_", "-", "+", "", " . ", "1_000", "1e3", "1.2.3", "0x10"] {
        assert!(parse_usdc_nonneg(s, 6).is_err(), "usdc {s:?}");
    }
    assert!(parse_limit_usdc(".", 6).is_err());
    assert_eq!(parse_usdc_nonneg(".5", 6).unwrap(), u128::from(500_000u64));
    assert_eq!(parse_usdc_nonneg("1.", 6).unwrap(), u128::from(1_000_000u64));
    assert_eq!(parse_usdc_nonneg("0", 6).unwrap(), 0);
    assert!(parse_usdc_nonneg("-1", 6).is_err());
}

// alloy `parse_units`·`format_units` 를 손으로 바꾼 것 — 옛 동작과 같은지.
#[test]
fn hand_rolled_units_match_old_behavior() {
    assert_eq!(parse_usdc_nonneg(" +1.5 ", 6).unwrap(), 1_500_000);
    assert_eq!(parse_usdc_nonneg("0.000001", 6).unwrap(), 1);
    assert_eq!(parse_usdc_nonneg("0.0000010", 6).unwrap(), 1); // 끝 0 은 자리로 안 센다
    assert!(parse_usdc_nonneg("0.0000001", 6).is_err());
    assert_eq!(parse_usdc_nonneg("-0", 6).unwrap(), 0); // alloy 도 0 이었다
    assert_eq!(parse_usdc_nonneg("-0.000", 6).unwrap(), 0);
    assert!(parse_usdc_nonneg("-0.000001", 6).is_err());
    assert_eq!(parse_usdc_nonneg("7", 0).unwrap(), 7);
    assert!(parse_usdc_nonneg("7.1", 0).is_err());
    assert_eq!(format_units(1_500_000, 6), "1.500000");
    assert_eq!(format_units(0, 6), "0.000000");
    assert_eq!(format_units(10_000, 6), "0.010000");
    assert_eq!(format_units(7, 0), "7");
    assert_eq!(format_units(0, 18), "0.000000000000000000");
    assert_eq!(trim_units(&format_units(5_000_000, 6)), "5");
}

// 예약 장부 쓰기의 실패 갈래를 실제로 밟는다.
#[test]
fn reservation_commit_failures_leave_old_ledger() {
    use crate::store::fault::{next, Fail};
    let dir = std::env::temp_dir().join(format!("budu-resv-{}", std::process::id()));
    let path = dir.join("spend.json");
    crate::store::write_atomic(&path, b"before").unwrap();
    // 디렉터리 동기화만 실패 → 새 장부가 제자리에 갔어도 예약 전으로 되돌리고 거절.
    next(Fail::NotDurable);
    assert!(commit_reservation(&path, "before", "after").is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "before");
    // 아예 못 썼다 → 거절, 파일 그대로.
    next(Fail::NotWritten);
    assert!(commit_reservation(&path, "before", "after").is_err());
    assert_eq!(fs::read_to_string(&path).unwrap(), "before");
    // 주입은 한 번만 — 다음 쓰기는 성공한다.
    assert!(commit_reservation(&path, "before", "after").is_ok());
    assert_eq!(fs::read_to_string(&path).unwrap(), "after");
    let _ = fs::remove_dir_all(&dir);
}

// 🔴 개발 71: 체인 확인 환불은 기록 하나당 한 번 — 같은 열쇠는 두 번째에 아무것도 안 한다. 다른 날·빈 열쇠도.
#[test]
fn refund_once_per_record() {
    let mut s = Spend {
        day: 7,
        usdc: "5000000".into(),
        ..Default::default()
    };
    let one = u128::from(1_000_000u64);
    assert!(apply_refund_once(&mut s, "USDC", one, 7, "id-a"));
    assert!(!apply_refund_once(&mut s, "USDC", one, 7, "id-a"));
    assert_eq!(s.usdc, "4000000");
    assert!(!apply_refund_once(&mut s, "USDC", one, 6, "id-b")); // 어제 예약
    assert!(!apply_refund_once(&mut s, "USDC", one, 7, "")); // 열쇠 없음
    assert!(apply_refund_once(&mut s, "USDC", one, 7, "id-b"));
    assert_eq!(s.usdc, "3000000");
    // 새 날 장부는 목록도 비어 있다(직렬화에서도 빠진다).
    let fresh = judge_spend(None, None, 8).unwrap();
    assert!(fresh.refunded.is_empty());
    assert!(!serde_json::to_string(&fresh).unwrap().contains("refunded"));
}

// 🔴 개발 71: 오늘 고친 장부를 못 읽으면 예약을 거부하고, 어제 이전 파일이면 새 날로 간다.
#[test]
fn unreadable_ledger_blocks_only_today() {
    let today = 20_000;
    let ok = |day: u64| {
        Some(Ok(format!(
            r#"{{"day":{day},"usdc":"19000000","eth":"0"}}"#
        )))
    };
    assert_eq!(
        judge_spend(ok(today), Some(today), today).unwrap().usdc,
        "19000000"
    );
    assert_eq!(
        judge_spend(ok(today - 1), Some(today - 1), today)
            .unwrap()
            .usdc,
        "0"
    );
    assert_eq!(judge_spend(None, None, today).unwrap().usdc, "0");
    assert!(judge_spend(Some(Ok("{깨짐".into())), Some(today), today).is_none());
    assert!(judge_spend(Some(Err(())), Some(today), today).is_none());
    assert!(judge_spend(Some(Err(())), None, today).is_none());
    assert_eq!(
        judge_spend(Some(Ok("{깨짐".into())), Some(today - 1), today)
            .unwrap()
            .usdc,
        "0"
    );
}

// 🔴 개발 72: JSON 모양은 맞는데 금액 칸이 숫자가 아니면 「못 읽음」 — 0 부터 한도를 재면 안 된다.
#[test]
fn ledger_with_bad_amount_blocks() {
    let today = 20_000;
    let with = |usdc: &str, eth: &str| {
        Some(Ok(format!(
            r#"{{"day":{today},"usdc":"{usdc}","eth":"{eth}"}}"#
        )))
    };
    assert!(judge_spend(with("abc", "0"), Some(today), today).is_none());
    assert!(judge_spend(with("-1", "0"), Some(today), today).is_none());
    // 빈 칸도 못 읽음 — 장부를 쓰는 곳은 늘 숫자를 쓴다.
    assert!(judge_spend(with("", "0"), Some(today), today).is_none());
    // ETH 칸은 걷어 냈다 — 옛 장부에 남은 그 칸은 무엇이든 무시하고 USDC 만 본다.
    assert_eq!(judge_spend(with("7", "1.5"), Some(today), today).unwrap().usdc, "7");
    assert_eq!(
        judge_spend(Some(Ok(format!(r#"{{"day":{today},"usdc":"7"}}"#))), Some(today), today).unwrap().usdc,
        "7"
    );
    // 어제 파일이면 그래도 새 날로 간다(한 번 상한 장부가 영영 막지 않게).
    assert_eq!(
        judge_spend(with("abc", "0"), Some(today - 1), today)
            .unwrap()
            .usdc,
        "0"
    );
}

// 단일 한도 초과는 거부.
#[test]
fn enforce_caps_rejects_over_single() {
    let r = enforce_caps(
        u128::from(6_000_000u64), // 6 USDC
        u128::from(5_000_000u64), // 단일 5
        0,
        u128::from(20_000_000u64),
        "USDC",
        6,
    );
    assert!(r.is_err());
}

// 단일은 통과해도 오늘 누적이 일일 한도를 넘기면 거부.
#[test]
fn enforce_caps_rejects_over_daily() {
    // 단일 5 이하지만, 이미 18 썼고 4 더 보내면 22 > 20.
    let r = enforce_caps(
        u128::from(4_000_000u64),
        u128::from(5_000_000u64),
        u128::from(18_000_000u64),
        u128::from(20_000_000u64),
        "USDC",
        6,
    );
    assert!(r.is_err());
}

// 한도가 0이면 무제한 → 아무리 커도 통과.
#[test]
fn enforce_caps_zero_means_unlimited() {
    let r = enforce_caps(
        u128::from(999_000_000u64),
        0, // 단일 무제한
        u128::from(500_000_000u64),
        0, // 일일 무제한
        "USDC",
        6,
    );
    assert!(r.is_ok());
}

// 단일·일일 모두 안쪽이면 통과.
/// 🔴 개발 73: 자릿수를 넘는 소수는 잘리지 않고 거부 — 잘리면 양수 한도가 0(무제한)이 된다.
#[test]
fn excess_decimals_rejected_not_truncated() {
    assert!(parse_usdc_nonneg("0.0000001", 6).is_err());
    assert!(parse_usdc_nonneg("1.1234567", 6).is_err());
    assert_eq!(parse_usdc_nonneg("1.123456", 6).unwrap(), u128::from(1_123_456u64));
    assert_eq!(parse_usdc_nonneg("1.1000000", 6).unwrap(), u128::from(1_100_000u64));
    assert_eq!(parse_usdc_nonneg("5", 6).unwrap(), u128::from(5_000_000u64));
}

#[test]
fn enforce_caps_allows_within_limits() {
    let r = enforce_caps(
        u128::from(2_000_000u64),
        u128::from(5_000_000u64),
        u128::from(10_000_000u64),
        u128::from(20_000_000u64),
        "USDC",
        6,
    );
    assert!(r.is_ok());
}

// 음수 USDC 금액/한도는 거부돼야 한다 (옛 alloy `parse_units` 가 음수를 거대 U256 으로 둔갑시키던 함정).
#[test]
fn parse_usdc_rejects_negative() {
    assert!(parse_usdc_nonneg("-1", 6).is_err());
    assert!(parse_usdc_nonneg("-0.000001", 6).is_err());
    assert!(parse_usdc_nonneg("  -5  ", 6).is_err());
    // 양수·0 은 정상.
    assert_eq!(
        parse_usdc_nonneg("1.5", 6).unwrap(),
        u128::from(1_500_000u64)
    );
    assert_eq!(parse_usdc_nonneg("0", 6).unwrap(), 0);
}
