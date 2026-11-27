use super::*;

// 개발 75 — 바꾼 칸만 얹고, 안 바꾼 칸은 다른 창이 저장한 값을 지킨다.
#[test]
fn edit_merge_keeps_untouched_fields() {
    // 다른 창이 자율 결제를 껐다.
    let cur = Settings { auto_approve_usdc: "0".into(), ..Default::default() };
    // 이 창은 옛 값(켜짐)을 들고 있고, 바꾼 건 일일 한도뿐.
    let form = Settings { auto_approve_usdc: "3".into(), daily_usdc: "7".into(), ..Default::default() };
    let out = merge_edit(&cur, &form, &["daily_usdc".into()], cur.chain_id).unwrap();
    assert_eq!(out.daily_usdc, "7");
    assert_eq!(out.auto_approve_usdc, "0", "손대지 않은 칸이 옛 값으로 되돌아가면 안 된다");
    // 모르는 키는 무시한다.
    assert!(merge_edit(&cur, &form, &["nope".into()], cur.chain_id).is_ok());
}

// 개발 75 — 네트워크를 안 바꿨는데 그 사이 체인이 바뀌었으면 거절, 바꿨으면 그 선택을 따른다.
#[test]
fn edit_merge_rejects_chain_drift() {
    let cur = Settings::default();
    let form = Settings::default();
    let other = cur.chain_id + 1;
    assert!(merge_edit(&cur, &form, &["daily_usdc".into()], other).is_err());
    let pick = Settings { chain_id: 101, ..Default::default() };
    let out = merge_edit(&cur, &pick, &["chain_id".into()], other).unwrap();
    assert_eq!(out.chain_id, 101);
}

// 기본 설정값 — 한도는 실돈 가드레일 축(USDC 5/20).
#[test]
fn default_settings_values() {
    let s = Settings::default();
    assert_eq!(s.single_usdc, "5");
    assert_eq!(s.daily_usdc, "20");
    // 쿠라 시절 ETH 칸은 저장 파일에서도 빠진다.
    let json = serde_json::to_string(&s).unwrap();
    assert!(!json.contains("_eth"), "{json}");
    // 자율 결제는 기본 꺼짐(0), 유휴 잠금 30분, RPC=공식, 자리비움잠금 꺼짐.
    assert_eq!(s.auto_approve_usdc, "0");
    assert_eq!(s.auto_lock_mins, "30");
    assert!(s.rpc_url.is_empty()); // 빈 값 = 활성 체인 공식 RPC 따라감
    assert!(!s.lock_on_blur);
    assert_eq!(s.chain_id, policy::NEW_INSTALL_CHAIN_ID); // 신규 기본 = 메인넷(개발 14)
}

// 🔴 개발 71: 다른 필드 하나가 깨져 통째로 못 읽는 파일에서도 사용자가 정한 한도는 살아남는다
// (기본 20 으로 올라가 더 나가지 않게). 자율 한도는 살리지 않는다 — 보수 기본이 「꺼짐」이다.
// JSON 자체가 깨졌으면 보수 기본값 그대로.
#[test]
fn broken_field_keeps_user_limits() {
    let f = SettingsFile::Text(
        r#"{"single_usdc":"0.5","daily_usdc":"1","single_eth":"0.001","daily_eth":"0.002",
           "auto_approve_usdc":"3","auto_lock_mins":42}"#
            .into(),
    );
    let s = settings_for_read(&f, true);
    assert_eq!(
        (s.single_usdc.as_str(), s.daily_usdc.as_str()),
        ("0.5", "1")
    );
    assert_eq!(s.auto_approve_usdc, "0");
    // 🔴 개발 73: 한도 칸을 못 살리면 「모름」 — 보수 기본 20 으로 올라가지 않는다.
    let junk = settings_for_read(&SettingsFile::Text("{ 깨짐".into()), true);
    assert_eq!(junk.daily_usdc, crate::limits::LIMIT_UNKNOWN);
    for broken in [
        r#"{"single_usdc":"0.5","daily_usdc":1,"single_eth":"0.001","daily_eth":"0.002"}"#,
        r#"{"single_usdc":"0.5","daily_usdc":null,"single_eth":"0.001","daily_eth":"0.002"}"#,
        r#"{"single_usdc":"0.5","single_eth":"0.001","daily_eth":"0.002"}"#,
    ] {
        let s = settings_for_read(&SettingsFile::Text(broken.into()), true);
        assert_eq!(s.daily_usdc, crate::limits::LIMIT_UNKNOWN, "{broken}");
        assert_eq!(s.single_usdc, "0.5", "{broken}");
    }
    let unreadable = settings_for_read(&SettingsFile::Unreadable, true);
    assert_eq!(unreadable.single_usdc, crate::limits::LIMIT_UNKNOWN);
    // 파일이 없던 시절의 기존 사용자(없음 + 지갑)는 그 시절 한도 그대로 — 「모름」이 아니다.
    assert_ne!(
        settings_for_read(&SettingsFile::Missing, true).daily_usdc,
        crate::limits::LIMIT_UNKNOWN
    );
}

// 🔴 신규(파일 없음)와 깨진 파일(있는데 못 읽음)은 다른 답이어야 한다.
// 신규 기본이 메인넷이 된 순간, "깨졌으면 기본값" 경로가 테스트넷 사용자를 조용히
// 실돈 체인으로 옮기는 문이 된다 — 깨진 파일은 돈 축을 테스트넷으로 접는다.
#[test]
fn read_fallbacks_split_new_vs_corrupt() {
    let text = |s: &str| SettingsFile::Text(s.to_string());
    // 파일 없음 + 지갑 없음 = 진짜 신규 → 메인넷 기본.
    assert_eq!(
        settings_for_read(&SettingsFile::Missing, false).chain_id,
        policy::NEW_INSTALL_CHAIN_ID
    );
    // 🔴 파일 없음 + 지갑 있음 = 개발 31 이전 설치(저장을 눌러야만 settings.json 이
    // 생겼다) → 테스트넷 보수. 여기가 메인넷이면 기존 지갑이 조용히 실돈 체인으로
    // 옮겨진다.
    let legacy = settings_for_read(&SettingsFile::Missing, true);
    assert_eq!(legacy.chain_id, policy::FALLBACK_CHAIN_ID);
    assert_eq!((legacy.single_usdc.as_str(), legacy.daily_usdc.as_str()), ("5", "20"));
    // 있는데 못 읽음(권한·홈 못 정함) → 보수적. 지갑 유무는 상관없다.
    assert_eq!(
        settings_for_read(&SettingsFile::Unreadable, false).chain_id,
        policy::FALLBACK_CHAIN_ID
    );
    // 깨진 JSON → 보수적(테스트넷). 한도는 「모름」, 나머지 값은 기본과 동일.
    let c = settings_for_read(&text("{ 이건 JSON 이 아니다"), true);
    assert_eq!(c.chain_id, policy::FALLBACK_CHAIN_ID);
    assert_eq!(c.single_usdc, crate::limits::LIMIT_UNKNOWN);
    // 정상 JSON 은 그대로.
    let ok = settings_for_read(
        &text(
            r#"{"single_usdc":"7","daily_usdc":"30",
        "single_eth":"0.1","daily_eth":"0.5","chain_id":101}"#,
        ),
        true,
    );
    assert_eq!(ok.chain_id, 101);
    assert_eq!(ok.single_usdc, "7");
}

// 🔴 프론트가 그리는 체인(read_settings → chain_id)과 돈이 나가는 체인(chain.rs → policy::chain_id_for)
// 이 **같은 파일에 같은 답**을 내야 한다 (실물 하네스에서 발견). 다른 필드 하나만 깨진
// 파일에서 둘이 갈리면 화면은 「연습용」인데 송금은 메인넷으로 나간다. 개발 56 부터 둘이 같은 함수라
// 구조적으로 못 갈리지만, 누가 Settings 파싱값을 다시 정본으로 삼으면 여기서 잡힌다.
#[test]
fn read_chain_matches_backend_selection() {
    let cases = [
        "{ 이건 JSON 이 아니다",
        "",
        r#"{"chain_id":101}"#, // 한도 필드 전부 없음(필수) → Settings 파싱 실패
        r#"{"single_usdc":5,"chain_id":101}"#, // 타입 틀림
        r#"{"daily_usdc":"20","single_eth":"0.01","daily_eth":"0.05","chain_id":101}"#, // 개발 51 하네스
        r#"{"single_usdc":"7","daily_usdc":"30","single_eth":"0.1","daily_eth":"0.5"}"#, // chain_id 없음(옛 파일)
        r#"{"single_usdc":"7","daily_usdc":"30","single_eth":"0.1","daily_eth":"0.5","chain_id":1}"#, // 미지원 id
    ];
    for text in cases {
        let file = SettingsFile::Text(text.to_string());
        let backend = crate::chain::chain_by_id(policy::chain_id_for(&file, || unreachable!()))
            .map(|c| c.chain_id)
            .unwrap_or(policy::FALLBACK_CHAIN_ID); // active_chain 과 같은 정규화
        let front = settings_for_read(&file, true).chain_id;
        assert_eq!(front, backend, "갈렸다: {text}");
    }
    // 개발 1 빌드가 쓴 EVM 8453 — 화면·저장 경로 모두 데브넷으로 접혀야 한도만 바꾼 저장이 통과한다.
    let evm = SettingsFile::Text(
        r#"{"single_usdc":"5","daily_usdc":"20","single_eth":"0.01","daily_eth":"0.05","chain_id":8453,"rpc_url":"https://base-mainnet.example"}"#.into(),
    );
    assert_eq!(settings_for_read(&evm, true).rpc_url, "");
    assert_eq!(settings_for_read(&evm, true).chain_id, policy::FALLBACK_CHAIN_ID);
    let upd = settings_for_update(&evm, true).expect("정상 JSON");
    assert_eq!(upd.chain_id, policy::FALLBACK_CHAIN_ID);
    assert_eq!(upd.rpc_url, "", "숨어 있던 옛 체인 RPC 가 저장 경로에서 되살아나면 안 된다");
    let form = Settings { daily_usdc: "3".into(), ..upd.clone() };
    let merged = merge_edit(&upd, &form, &["daily_usdc".into()], settings_for_read(&evm, true).chain_id).unwrap();
    assert!(crate::chain::chain_by_id(merged.chain_id).is_some());
    // 한도 필드가 빠져 Settings 로는 못 읽는 파일 — 체인만은 살아 있어야 한다.
    let f = settings_for_read(&SettingsFile::Text(r#"{"chain_id":101}"#.into()), true);
    assert_eq!(f.chain_id, policy::SOLANA_MAINNET_ID);
    // 못 살린 한도는 「모름」 — 보수 기본 20 으로 올라가지 않게.
    assert_eq!(f.single_usdc, crate::limits::LIMIT_UNKNOWN);
    // 파일 없음도 양쪽이 같은 함수를 타므로 같은 답이다.
    for wallet in [false, true] {
        assert_eq!(
            settings_for_read(&SettingsFile::Missing, wallet).chain_id,
            policy::chain_id_for(&SettingsFile::Missing, || wallet)
        );
    }
}

// 🔴 화면이 그리는 RPC(read_settings → rpc_url)와 잔액·송금이 붙는 RPC(effective_rpc →
// policy::rpc_url_for)가 **같은 파일에 같은 답**을 내야 한다. 개발 51 하네스·개발 56 대체
// 한도 필드가 깨진 파일에서 GUI 는 공식 RPC 로 접고 MCP 는 커스텀 RPC 를 봤다. 이제 셋이
// 같은 함수라 못 갈리지만, 누가 Settings 파싱값을 다시 정본으로 삼으면 여기서 잡힌다.
#[test]
fn read_rpc_matches_backend_selection() {
    let cases = [
        "{ 이건 JSON 이 아니다",
        "",
        r#"{"rpc_url":"https://x.invalid"}"#, // 한도 필드 전부 없음 → Settings 파싱 실패
        r#"{"single_usdc":5,"rpc_url":"https://x.invalid"}"#, // 타입 틀림
        r#"{"daily_usdc":"20","single_eth":"0.01","daily_eth":"0.05","chain_id":101,"rpc_url":"http://127.0.0.1:8545"}"#, // 개발 51 하네스
        r#"{"single_usdc":"7","daily_usdc":"30","single_eth":"0.1","daily_eth":"0.5"}"#, // rpc_url 없음(옛 파일)
        r#"{"single_usdc":"7","daily_usdc":"30","single_eth":"0.1","daily_eth":"0.5","rpc_url":" https://y.invalid "}"#, // 정상 + 공백
        r#"{"chain_id":103,"chain_id":101,"rpc_url":"https://x.invalid"}"#, // 키 중복
    ];
    for text in cases {
        let file = SettingsFile::Text(text.to_string());
        let backend = policy::rpc_url_for(&file);
        let front = settings_for_read(&file, true).rpc_url;
        assert_eq!(front, backend, "갈렸다: {text}");
    }
    // 개발 51 의 그 파일 — Settings 로는 못 읽지만 지정 RPC 와 체인은 살아 있어야 한다.
    let f = settings_for_read(
        &SettingsFile::Text(
            r#"{"daily_usdc":"20","single_eth":"0.01","daily_eth":"0.05","chain_id":101,"rpc_url":"http://127.0.0.1:8545"}"#.into(),
        ),
        true,
    );
    assert_eq!(f.rpc_url, "http://127.0.0.1:8545");
    assert_eq!(f.chain_id, policy::SOLANA_MAINNET_ID);
    // 못 살린 한도는 「모름」, 살린 칸은 그대로.
    assert_eq!(f.single_usdc, crate::limits::LIMIT_UNKNOWN);
    assert_eq!(f.daily_usdc, "20");
    // 파일 없음·못 읽음은 지정 RPC 없음(공식).
    assert!(settings_for_read(&SettingsFile::Missing, true)
        .rpc_url
        .is_empty());
    assert!(settings_for_read(&SettingsFile::Unreadable, true)
        .rpc_url
        .is_empty());
}

// 옛 settings.json(자율 결제 필드 없음)도 손실 없이 로드되고 새 필드는 기본값이 된다.
// (#[serde(default)] 가 없으면 파싱 실패 → 사용자의 기존 한도가 통째로 날아가는 버그)
#[test]
fn old_settings_without_auto_fields_loads() {
    let old = r#"{"single_usdc":"7","daily_usdc":"30","single_eth":"0.1","daily_eth":"0.5"}"#;
    let s: Settings = serde_json::from_str(old).unwrap();
    assert_eq!(s.single_usdc, "7"); // 기존 값 보존
    assert_eq!(s.daily_usdc, "30");
    assert_eq!(s.auto_approve_usdc, "0"); // 새 필드는 기본값
    assert_eq!(s.auto_lock_mins, "30");
    assert!(s.rpc_url.is_empty()); // 옛 파일엔 RPC 없음 → 빈 값(=공식 폴백)
    assert!(!s.lock_on_blur);
    assert!(s.notify_auto); // 알림은 기본 켜짐 (끄는 쪽이 명시적 선택)
    assert!(!s.notify_hide_amount); // 금액 숨기기는 기본 꺼짐
    assert!(s.auto_trusted_only); // 신뢰 주소 가드도 기본 켜짐 (안전 쪽 디폴트)
    assert_eq!(s.chain_id, policy::FALLBACK_CHAIN_ID); // 옛 파일엔 chain_id 없음 → 테스트넷
                                                   // 개발 31 필드도 옛 파일에서 안전하게 온다. autostart 기본은 **None**(false 아님) —
                                                   // false 면 자동 시작을 켜 둔 기존 사용자가 "끔을 원했다"로 기록돼 복구 대상에서 빠진다.
    assert_eq!(s.autostart, None);
}

// 🔴 해석 안 되는 설정 파일 위에 기본값을 쓰면 안 된다.
//
// read_settings 는 어떤 실패든 기본값으로 삼키는데, 개발 31 부터 autostart::reconcile 이
// **시작할 때마다** 읽고-쓰기를 한다. 그 경로가 read_settings 를 쓰면, 파일이 한 번 깨진
// 순간 사용자의 한도·커스텀 RPC·chain_id 가 통째로 기본값으로 덮인다 —
// 메인넷(101)을 쓰던 사람이 아무 말 없이 데브넷으로 돌아간다.
#[test]
fn corrupt_settings_are_never_overwritten() {
    let text = |s: &str| SettingsFile::Text(s.to_string());
    // 파일 없음 + 지갑 없음 = 진짜 신규 → 메인넷 기본에서 시작해 만들어도 안전.
    assert_eq!(
        settings_for_update(&SettingsFile::Missing, false)
            .expect("신규인데 None")
            .chain_id,
        policy::NEW_INSTALL_CHAIN_ID
    );
    // 🔴 파일 없음 + 지갑 있음 = 설정 파일이 없던 시절의 기존 사용자 → 테스트넷
    // 보수 기본을 저장해 못박는다. 메인넷을 저장하면 기존 지갑이 조용히 실돈
    // 체인으로 옮겨진다.
    assert_eq!(
        settings_for_update(&SettingsFile::Missing, true)
            .expect("레거시인데 None")
            .chain_id,
        policy::FALLBACK_CHAIN_ID
    );
    // 있는데 못 읽음(권한·홈 못 정함) → None. 덮어쓰지 않는다.
    assert!(settings_for_update(&SettingsFile::Unreadable, false).is_none());

    // 정상 JSON → 그대로 읽힌다.
    let ok = settings_for_update(
        &text(
            r#"{"single_usdc":"7","daily_usdc":"30",
        "single_eth":"0.1","daily_eth":"0.5","chain_id":101}"#,
        ),
        true,
    );
    assert_eq!(ok.expect("정상 JSON 인데 None").chain_id, 101);

    // 🔴 깨진 JSON·타입이 어긋난 값 → None. 호출부는 여기서 손을 떼야 한다.
    assert!(settings_for_update(&text("{ 이건 JSON 이 아니다"), true).is_none());
    assert!(settings_for_update(&text(r#"{"single_usdc": 5}"#), true).is_none()); // 문자열이어야 함
    assert!(settings_for_update(&text(""), true).is_none());
    // 필수 필드(serde default 없음) 하나만 빠져도 통째로 못 읽는다 — 개발 51 하네스에서
    // 손으로 쓴 파일이 이렇게 접혀 커스텀 RPC 가 무시됐다. 이 경우가 화면 알림의 대상이다.
    assert!(settings_for_update(
        &text(r#"{"daily_usdc":"20","single_eth":"0.01","daily_eth":"0.05","chain_id":101,"rpc_url":"https://example.invalid"}"#),
        true
    )
    .is_none());
}

// 🔴 폼 저장이 앱 관리 필드를 지우면 안 된다.
// 프론트 Settings 타입엔 autostart 가 없어서, 설정 화면의 "저장하고 닫기"는 늘 기본값(None)을
// 실어 보낸다. 그대로 쓰면 저장 한 번에 자동 시작 희망값이 날아간다.
#[test]
fn form_save_does_not_wipe_app_managed_fields() {
    let on_disk = Settings {
        autostart: Some(true),
        menubar: false,
        ..Default::default()
    };
    // 프론트가 보낸 것 = 폼 필드만 있고 관리 필드는 기본값인 상태.
    let mut from_form = Settings {
        single_usdc: "9".into(),
        ..Default::default()
    };
    assert_eq!(from_form.autostart, None); // 전제 확인

    preserve_managed(&mut from_form, &on_disk);

    assert_eq!(from_form.autostart, Some(true)); // 희망값 보존
    assert!(!from_form.menubar); // 꺼 둔 메뉴바도 보존
    assert_eq!(from_form.single_usdc, "9"); // 폼 값은 그대로 반영
}

// 음수 한도는 저장을 거부해야 한다 (거대 U256=무제한 둔갑 차단). 양수·0 은 통과.
#[test]
fn set_settings_rejects_negative_limits() {
    let neg_single = Settings {
        single_usdc: "-1".into(),
        ..Default::default()
    };
    assert!(set_settings(neg_single).is_err());

    let neg_auto = Settings {
        auto_approve_usdc: "-0.01".into(),
        ..Default::default()
    };
    assert!(set_settings(neg_auto).is_err());
}

// 알 수 없는 체인 ID는 저장 거부 (지원 체인=테스트넷/메인넷만). chain_id:1(이더리움 L1)=미지원.
// (Err 경로만 검사 — 유효 입력으로 set_settings 를 부르면 실제 ~/.budu/settings.json 을 덮어써서
//  테스트가 사용자 설정·체인을 바꿔버린다. 음수 한도 테스트와 같은 이유로 거부 케이스만.)
#[test]
fn set_settings_rejects_unknown_chain() {
    let bad = Settings {
        chain_id: 1,
        ..Default::default()
    };
    let err = set_settings(bad).unwrap_err();
    assert!(
        err.contains("지원하지 않는 체인"),
        "체인 검증 메시지가 아님: {err}"
    );
}

// redact_urls 테스트는 policy::tests 가 본다(정본이 그쪽으로 갔다).

// 개발 14 — 한도는 네트워크별. 옛 파일(한도가 평평한 칸뿐)은 그 값을 **두 네트워크에 복사**한다(누구의 한도도 풀리지 않게).
#[test]
fn legacy_limits_copy_to_every_network() {
    let f = SettingsFile::Text(
        r#"{"single_usdc":"1","daily_usdc":"2","auto_approve_usdc":"0.5","auto_lock_mins":"30","chain_id":103}"#.into(),
    );
    let s = settings_for_read(&f, true);
    for id in policy::SUPPORTED_CHAIN_IDS {
        let l = &s.limits[&id.to_string()];
        assert_eq!((l.single_usdc.as_str(), l.daily_usdc.as_str(), l.auto_approve_usdc.as_str()), ("1", "2", "0.5"), "{id}");
    }
}

// 활성 체인의 한도가 평평한 칸(결제·세션·홈이 읽는 곳)에 비친다.
#[test]
fn flat_fields_follow_active_network() {
    let f = SettingsFile::Text(
        r#"{"single_usdc":"9","daily_usdc":"9","auto_approve_usdc":"9","auto_lock_mins":"30","chain_id":101,
            "limits":{"101":{"single_usdc":"1","daily_usdc":"3","auto_approve_usdc":"0"},
                      "103":{"single_usdc":"50","daily_usdc":"100","auto_approve_usdc":"5"}}}"#
            .into(),
    );
    let s = settings_for_read(&f, true);
    assert_eq!((s.single_usdc.as_str(), s.daily_usdc.as_str(), s.auto_approve_usdc.as_str()), ("1", "3", "0"));
    let dev = SettingsFile::Text(
        r#"{"single_usdc":"9","daily_usdc":"9","auto_lock_mins":"30","chain_id":103,
            "limits":{"101":{"single_usdc":"1","daily_usdc":"3","auto_approve_usdc":"0"},
                      "103":{"single_usdc":"50","daily_usdc":"100","auto_approve_usdc":"5"}}}"#
            .into(),
    );
    assert_eq!(settings_for_read(&dev, true).daily_usdc, "100");
}

// 깨진 파일: 체인별 한도도 있는 칸만 살리고, 못 살린 칸은 「모름」, 자율 금액은 꺼짐.
#[test]
fn broken_file_salvages_network_limits() {
    let f = SettingsFile::Text(
        r#"{"single_usdc":"9","daily_usdc":"9","auto_lock_mins":42,"chain_id":101,
            "limits":{"101":{"single_usdc":"1","auto_approve_usdc":"7"}}}"#
            .into(),
    );
    let s = settings_for_read(&f, true);
    assert_eq!(s.single_usdc, "1");
    assert_eq!(s.daily_usdc, crate::limits::LIMIT_UNKNOWN);
    assert_eq!(s.auto_approve_usdc, "0");
}

// 저장 합치기: `limits.<id>.<칸>` 은 그 칸만 — 다른 창이 고친 다른 네트워크 한도를 되돌리지 않는다.
#[test]
fn edit_merge_limits_per_field() {
    let mut cur = Settings::default().normalize();
    cur.limits.get_mut("103").unwrap().daily_usdc = "77".into(); // 다른 창이 데브넷 하루 한도를 바꿨다
    let mut form = Settings::default().normalize();
    form.limits.get_mut("101").unwrap().single_usdc = "2".into();
    let out = merge_edit(&cur, &form, &["limits.101.single_usdc".into(), "limits.101.bogus".into()], cur.chain_id)
        .unwrap()
        .normalize();
    assert_eq!(out.limits["101"].single_usdc, "2");
    assert_eq!(out.limits["103"].daily_usdc, "77");
    assert_eq!(out.single_usdc, "2"); // 활성 체인(메인넷) 값이 평평한 칸에
}

// 자동 잠금: 목록 밖 값은 짧은 쪽으로, 0·목록 값·10분 미만은 그대로.
#[test]
fn lock_minutes_snap_down() {
    assert_eq!(snap_lock_mins("45"), "30");
    assert_eq!(snap_lock_mins("120"), "60");
    assert_eq!(snap_lock_mins("99999"), "10080");
    assert_eq!(snap_lock_mins("0"), "0");
    assert_eq!(snap_lock_mins("5"), "5");
    assert_eq!(snap_lock_mins("240"), "240");
    assert_eq!(snap_lock_mins("abc"), "abc");
}

// 신규 설치는 메인넷 + 두 네트워크 모두 기본 한도, 자동 학습 꺼짐.
#[test]
fn new_install_is_mainnet_with_limits() {
    let s = settings_for_read(&SettingsFile::Missing, false);
    assert_eq!(s.chain_id, policy::SOLANA_MAINNET_ID);
    assert_eq!(s.limits.len(), 2);
    assert!(!s.auto_learn_trusted);
    // 지갑만 있는 옛 설치는 여전히 데브넷.
    assert_eq!(settings_for_read(&SettingsFile::Missing, true).chain_id, policy::FALLBACK_CHAIN_ID);
}
