use super::*;

// 하드햇 기본 니모닉(test×11 junk)의 솔라나 0번 계정 m/44'/501'/0'/0' (keys.rs 와 같은 파생).
// 팬텀 호환 자체는 keys::tests::phantom_compatible_first_account 가 알려진 주소로 고정한다.
#[test]
fn derive_known_vector() {
    let phrase = "test test test test test test test test test test test junk";
    let addr = derive_address(phrase).expect("파생 성공");
    assert_eq!(addr, "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96");
}

// 새로 생성한 12단어 니모닉 → 유효한 솔라나 주소(base58 32바이트, 곡선 위).
#[test]
fn generate_and_derive_roundtrip() {
    let phrase = keys::generate_phrase();
    assert_eq!(phrase.split_whitespace().count(), 12);
    let addr = derive_address(&phrase).expect("파생 성공");
    let b = crate::solana::parse_pubkey(&addr).expect("base58 32바이트");
    assert!(crate::solana::is_on_curve(&b));
}

// 암호화 → 올바른 비번으로 복호화하면 원본 니모닉이 그대로 나온다.
#[test]
fn encrypt_decrypt_roundtrip() {
    let phrase = "test test test test test test test test test test test junk";
    let addr = derive_address(phrase).unwrap();
    let w = encrypt_wallet(phrase, &addr, "비밀번호123").expect("암호화");
    let out = decrypt_wallet(&w, "비밀번호123").expect("복호화");
    assert_eq!(out.as_str(), phrase);
}

// 틀린 비번이면 복호화가 실패해야 한다 (GCM 인증 태그).
#[test]
fn wrong_password_fails() {
    let phrase = "test test test test test test test test test test test junk";
    let addr = derive_address(phrase).unwrap();
    let w = encrypt_wallet(phrase, &addr, "맞는비번").expect("암호화");
    assert!(decrypt_wallet(&w, "틀린비번").is_err());
}

// 복호화한 니모닉을 12단어로 쪼개면 reveal_mnemonic 이 돌려줄 결과가 된다.
#[test]
fn reveal_splits_into_twelve_words() {
    let phrase = "test test test test test test test test test test test junk";
    let addr = derive_address(phrase).unwrap();
    let w = encrypt_wallet(phrase, &addr, "pw").unwrap();
    let words: Vec<String> = decrypt_wallet(&w, "pw")
        .unwrap()
        .split_whitespace()
        .map(String::from)
        .collect();
    assert_eq!(words.len(), 12);
    assert_eq!(words[0], "test");
    assert_eq!(words[11], "junk");
}

// 새로 암호화한 지갑은 아직 백업 안 된 상태여야 한다.
#[test]
fn new_wallet_is_not_backed_up() {
    let w = encrypt_wallet("a b c", "0x0", "pw").unwrap();
    assert!(!w.backed_up);
}

// backed_up/kdf 필드가 없는 옛 v2 파일도 읽혀야 한다 (#[serde(default)]).
// kdf 기본값은 반드시 v2 시절 파라미터(19 MiB/t=2/p=1) — 아니면 기존 지갑이 안 열린다.
#[test]
fn legacy_v2_without_backed_up_field_loads() {
    let json =
        r#"{"version":2,"address":"0xabc","salt":"AA==","nonce":"AA==","ciphertext":"AA=="}"#;
    let w: EncryptedWallet = serde_json::from_str(json).expect("파싱 성공");
    assert!(!w.backed_up);
    assert_eq!((w.kdf_m, w.kdf_t, w.kdf_p), (KDF_M_V2, KDF_T_V2, KDF_P_V2));
}

// 새로 암호화하면 v3 + 강화 KDF 파라미터여야 한다.
#[test]
fn new_wallet_uses_strengthened_kdf() {
    let w = encrypt_wallet("a b c", "0x0", "pw").unwrap();
    assert_eq!(w.version, 3);
    assert_eq!((w.kdf_m, w.kdf_t, w.kdf_p), (KDF_M_V3, KDF_T_V3, KDF_P_V3));
}

// 실제 v2 파일 재현: kdf 필드를 JSON에서 지워도(옛 파일 그대로) 복호화돼야 한다.
#[test]
fn real_v2_file_without_kdf_fields_decrypts() {
    let phrase = "test test test test test test test test test test test junk";
    let v2 = encrypt_with(phrase, "0x0", "pw", 2, KDF_M_V2, KDF_T_V2, KDF_P_V2).unwrap();
    let mut json: serde_json::Value = serde_json::to_value(&v2).unwrap();
    let obj = json.as_object_mut().unwrap();
    obj.remove("kdf_m");
    obj.remove("kdf_t");
    obj.remove("kdf_p"); // 옛 파일엔 이 필드들이 없다
    let w: EncryptedWallet = serde_json::from_value(json).unwrap();
    assert_eq!(decrypt_wallet(&w, "pw").unwrap().as_str(), phrase);
}

// lazy 업그레이드: v2 → v3 사본(같은 비번으로 복호화 가능, backed_up 보존), v3 → None.
#[test]
fn upgrade_v2_to_v3_preserves_contents() {
    let phrase = "test test test test test test test test test test test junk";
    let mut v2 = encrypt_with(phrase, "0xAddr", "pw", 2, KDF_M_V2, KDF_T_V2, KDF_P_V2).unwrap();
    v2.backed_up = true;

    let v3 = upgraded_wallet(&v2, phrase, "pw").expect("v2는 업그레이드 대상");
    assert_eq!(v3.version, 3);
    assert_eq!(
        (v3.kdf_m, v3.kdf_t, v3.kdf_p),
        (KDF_M_V3, KDF_T_V3, KDF_P_V3)
    );
    assert_eq!(v3.address, "0xAddr");
    assert!(v3.backed_up); // 백업 표시 보존
    assert_eq!(decrypt_wallet(&v3, "pw").unwrap().as_str(), phrase);

    // 이미 v3면 재암호화하지 않는다.
    assert!(upgraded_wallet(&v3, phrase, "pw").is_none());
}

/// 🔴 개발 70: 업그레이드가 KDF 를 도는 사이 계정이 추가·전환됐으면 그 최신 목록을 싣고,
/// 그새 다른 지갑을 가져왔거나 이미 업그레이드됐으면 쓰지 않는다.
#[test]
fn upgrade_carries_latest_metadata_or_skips() {
    let phrase = "test test test test test test test test test test test junk";
    let v2 = encrypt_with(phrase, "0xAddr", "pw", 2, KDF_M_V2, KDF_T_V2, KDF_P_V2).unwrap();
    let nw = upgraded_wallet(&v2, phrase, "pw").unwrap();

    let mut latest = v2.clone();
    latest.accounts = vec![
        Account {
            index: 0,
            address: "0xAddr".into(),
            label: String::new(),
        },
        Account {
            index: 1,
            address: "0xB".into(),
            label: String::new(),
        },
    ];
    latest.active = 1;
    latest.backed_up = true;
    let out = carry_metadata(nw.clone(), &v2, &latest).expect("같은 지갑이면 쓴다");
    assert_eq!(out.version, 3);
    assert_eq!(
        (out.accounts.len(), out.active, out.backed_up),
        (2, 1, true)
    );

    let other = encrypt_with(phrase, "0xAddr", "pw", 2, KDF_M_V2, KDF_T_V2, KDF_P_V2).unwrap();
    assert!(
        carry_metadata(nw.clone(), &v2, &other).is_none(),
        "암호문이 다르면 안 쓴다"
    );
    assert!(
        carry_metadata(nw.clone(), &v2, &nw).is_none(),
        "이미 v3 면 안 쓴다"
    );

    // 그새 `0x…` 주소를 고쳐 썼으면 업그레이드가 옛 주소로 되돌리지 않는다.
    let mut repaired = v2.clone();
    repaired.address = "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96".into();
    let out = carry_metadata(nw.clone(), &v2, &repaired).unwrap();
    assert_eq!(out.address, "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96");
}

// `0x…`·빈 주소만 「옛 형식」, 솔라나 꼴은(키와 맞든 안 맞든) 아니다.
#[test]
fn legacy_address_detection() {
    assert!(is_legacy_address("0x036CbD53842c5426634e7929541eC2318f3dCF7e"));
    assert!(is_legacy_address(""));
    assert!(!is_legacy_address("oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96"));
    assert!(!is_legacy_address("AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD"));
}

// 정규화: 줄바꿈·중복 공백·대문자를 흡수해 표준 12단어로 정리한다.
#[test]
fn normalize_collapses_whitespace_and_lowercases() {
    let out = normalize_mnemonic(
        "  TEST   test\ntest\ttest test test test test test test test JUNK ",
    );
    assert_eq!(out.split_whitespace().count(), 12);
    assert_eq!(
        &*out,
        "test test test test test test test test test test test junk"
    );
}

// 가져오기: 지저분한 입력(대문자·여분 공백)도 정규화 후 알려진 벡터로 파생되고,
// v3 강화 KDF + backed_up=true(시드 직접 보유)로 암호화된다. 복호화하면 정규화된 문구가 나온다.
#[test]
fn import_builds_backed_up_v3_from_messy_phrase() {
    let w = build_imported_wallet(
        "  TEST test\ntest test test test test test test test test junk ",
        "pw",
    )
    .expect("정상 문구");
    assert_eq!(w.version, 3);
    assert_eq!((w.kdf_m, w.kdf_t, w.kdf_p), (KDF_M_V3, KDF_T_V3, KDF_P_V3));
    assert!(w.backed_up); // 사용자가 시드를 직접 입력 = 이미 보유
    assert_eq!(w.address, "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96");
    assert_eq!(
        decrypt_wallet(&w, "pw").unwrap().as_str(),
        "test test test test test test test test test test test junk"
    );
}

// 체크섬이 안 맞는 12단어(abandon×12)는 거부돼야 한다 — 오타·잘못된 문구 방어.
#[test]
fn import_rejects_bad_checksum() {
    let bad = "abandon abandon abandon abandon abandon abandon \
               abandon abandon abandon abandon abandon abandon";
    assert!(build_imported_wallet(bad, "pw").is_err());
}

// 참고: abandon×11 + about 은 체크섬이 맞는 표준 벡터라 가져오기가 성공해야 한다.
#[test]
fn import_accepts_valid_all_zero_vector() {
    let ok = "abandon abandon abandon abandon abandon abandon \
              abandon abandon abandon abandon abandon about";
    let w = build_imported_wallet(ok, "pw").expect("유효한 표준 벡터");
    assert_eq!(w.address, "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk");
}

// 단어 수가 12/15/18/21/24 가 아니면 거부 (사람 말 메시지).
#[test]
fn import_rejects_wrong_word_count() {
    assert!(build_imported_wallet("test test test", "pw").is_err());
    assert!(build_imported_wallet("", "pw").is_err());
}

// HD 파생 인덱스: 1·2번 계정 = m/44'/501'/1'/0' · m/44'/501'/2'/0' (팬텀의 계정 순서).
// 여기가 어긋나면 «이 12단어를 다른 지갑에 넣어도 같은 계정이 나온다»는 약속이 깨진다.
#[test]
fn derive_hardhat_accounts_by_index() {
    let phrase = "test test test test test test test test test test test junk";
    let a1 = signer_for_account(phrase, 1).unwrap().address();
    let a2 = signer_for_account(phrase, 2).unwrap().address();
    assert_eq!(a1, "AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD");
    assert_eq!(a2, "CqMbRgMuEhQi9BUS8xP44Wk5nENm48FqJnfjEi4eNb1k");
    // 0 번은 예전 함수와 같다.
    assert_eq!(
        signer_for_account(phrase, 0).unwrap().address(),
        signer_from_phrase(phrase).unwrap().address()
    );
}

// 옛 파일(accounts/active 없음) → 계정 0 하나, 주소는 address 필드, 활성 = 0.
// (규칙 자체는 policy::tests 가 본다 — 여기는 EncryptedWallet 역직렬화가 그 함수에 제대로 물리는지.)
#[test]
fn legacy_file_normalizes_to_single_account() {
    let json =
        r#"{"version":3,"address":"0xAbc","salt":"AA==","nonce":"AA==","ciphertext":"AA=="}"#;
    let w: EncryptedWallet = serde_json::from_str(json).unwrap();
    assert_eq!(
        w.accounts(),
        vec![Account {
            index: 0,
            address: "0xAbc".into(),
            label: String::new()
        }]
    );
    assert_eq!(w.active_account().index, 0);
    assert_eq!(w.active_account().address, "0xAbc");
}

// 계정 목록: 인덱스 순 정렬, 계정 0 의 주소는 address 필드가 이긴다(라벨은 목록 것),
// 활성이 목록에 없으면 계정 0 으로 접는다.
#[test]
fn accounts_sorted_zero_from_address_and_active_fallback() {
    let json = r#"{"version":3,"address":"0xZero","salt":"AA==","nonce":"AA==","ciphertext":"AA==",
        "accounts":[{"index":2,"address":"0xTwo","label":"AI"},{"index":0,"address":"0xStale","label":"나"},{"index":1,"address":"0xOne"}],
        "active":2}"#;
    let w: EncryptedWallet = serde_json::from_str(json).unwrap();
    let list = w.accounts();
    assert_eq!(
        list.iter().map(|a| a.index).collect::<Vec<_>>(),
        vec![0, 1, 2]
    );
    assert_eq!(list[0].address, "0xZero"); // 정본은 address 필드
    assert_eq!(list[0].label, "나");
    assert_eq!(list[1].label, ""); // label 없는 항목도 읽힌다
    assert_eq!(w.active_account().address, "0xTwo");

    let mut w2 = w;
    w2.active = 9; // 없는 계정 → 0
    assert_eq!(w2.active_account().index, 0);
}

// 계정별 파일 이름 규칙(0 은 그대로, 그 외 -a{n})은 policy::tests 가 본다.

// 작업 안에서 고정한 계정이 파일보다 우선한다 (디스크를 안 본다).
#[tokio::test]
async fn pinned_account_overrides_file() {
    let got = with_pinned_account(5, async { active_account_index() }).await;
    assert_eq!(got, 5);
    // 안쪽에서 다시 감싸면 안쪽 값.
    let inner =
        with_pinned_account(5, with_pinned_account(6, async { active_account_index() })).await;
    assert_eq!(inner, 6);
}

// 라벨 정리: 공백·제어 문자 제거, 24자 상한.
#[test]
fn clean_label_trims_and_caps() {
    assert_eq!(
        clean_label(
            "  AI 전용
 "
        ),
        "AI 전용"
    );
    assert_eq!(clean_label(""), "");
    let long = "가".repeat(40);
    assert_eq!(clean_label(&long).chars().count(), LABEL_MAX_CHARS);
}

// v2→v3 업그레이드가 계정 목록·활성 계정을 잃지 않는다.
#[test]
fn upgrade_preserves_accounts() {
    let phrase = "test test test test test test test test test test test junk";
    let mut v2 = encrypt_with(phrase, "0xAddr", "pw", 2, KDF_M_V2, KDF_T_V2, KDF_P_V2).unwrap();
    v2.accounts = vec![Account {
        index: 1,
        address: "0xOne".into(),
        label: "둘째".into(),
    }];
    v2.active = 1;
    let v3 = upgraded_wallet(&v2, phrase, "pw").unwrap();
    assert_eq!(v3.accounts, v2.accounts);
    assert_eq!(v3.active_account().address, "0xOne");
}

/// 🔴 **실제 파일 경로를 타는 계정 흐름 하네스**. `HOME` 을 임시 폴더로 바꿔
/// 이 프로세스의 `~/.budu` 을 격리한 뒤, 가져오기 → 계정 추가 → 전환 → 요청 계정 대조 →
/// 계정별 내역 파일 → 정산 교차 반영 → 대기 중 전환 차단을 실물로 돈다.
///
/// `#[ignore]` 인 이유: HOME 을 바꾸는 건 프로세스 전역이라 다른 테스트와 같이 돌면 안 된다.
/// 반드시 혼자 돌린다:  `cargo test -- --ignored account_flow_in_temp_home --test-threads=1`
/// `BUDU_HARNESS_HOME` 을 주면 그 폴더를 HOME 으로 써서(지우지 않음) CLI 하네스가 이어 쓴다.
#[tokio::test]
#[ignore = "HOME 을 바꾼다 — 혼자 돌릴 것 (--test-threads=1)"]
async fn account_flow_in_temp_home() {
    let home = std::env::var("BUDU_HARNESS_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            std::env::temp_dir().join(format!("budu-accounts-{}", std::process::id()))
        });
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    std::env::set_var("HOME", &home);
    assert_eq!(budu_dir().unwrap(), home.join(".budu"));

    // ① 하드햇 니모닉 가져오기 → 계정 0 하나, 활성 0.
    let phrase = "test test test test test test test test test test test junk";
    import_wallet("pw".into(), phrase.into()).unwrap();
    let st = get_wallet_status().unwrap();
    assert_eq!(st.accounts.len(), 1);
    assert_eq!(st.active, 0);
    assert_eq!(
        st.address.as_deref(),
        Some("oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96")
    );

    // ② 틀린 비번으로는 계정을 못 만든다.
    assert!(add_account("wrong".into()).is_err());

    // ③ 계정 추가 → 인덱스 1(하드햇 1번 주소), 곧 활성. 한 번 더 → 2.
    let st = add_account("pw".into()).unwrap();
    assert_eq!(st.active, 1);
    assert_eq!(
        st.address.as_deref(),
        Some("AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD")
    );
    let st = add_account("pw".into()).unwrap();
    assert_eq!(st.active, 2);
    assert_eq!(st.accounts.len(), 3);
    assert_eq!(active_account_index(), 2);

    // ④ 이름 바꾸기(공백 정리), 없는 계정은 거부.
    let st = rename_account(1, "  AI 전용 \n".into()).unwrap();
    assert_eq!(st.accounts[1].label, "AI 전용");
    assert!(rename_account(9, "x".into()).is_err());

    // ⑤ 전환: 없는 계정 거부, 0 으로 → 활성 주소가 원래 주소.
    assert!(switch_account(7).is_err());
    let st = switch_account(0).unwrap();
    assert_eq!(st.active, 0);
    assert_eq!(active_account().unwrap().index, 0);

    // ⑥ 비번 잠금 해제는 활성 계정(0)의 키, 고정하면 그 계정의 키.
    assert_eq!(
        unlock_signer("pw").unwrap().address(),
        "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96"
    );
    let pinned = with_pinned_account(1, async {
        unlock_signer("pw").unwrap().address()
    })
    .await;
    assert_eq!(pinned, "AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD");

    // ⑥-1 파일의 평문 주소가 **솔라나 꼴인데** 키와 다르면(변조·다른 빌드) 서명자를 안 만들고 고쳐 쓰지도 않는다
    //     — 세션 경로도 같다.
    let good = read_encrypted().unwrap();
    let mut bad = read_encrypted().unwrap();
    bad.address = "AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD".into(); // 계정 1 의 주소를 계정 0 자리에
    write_encrypted(&bad).unwrap();
    assert!(unlock_signer("pw").is_err());
    assert!(session_signer_for(phrase).is_err());
    assert_eq!(read_encrypted().unwrap().address, "AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD");
    assert!(!get_wallet_status().unwrap().legacy_address);
    write_encrypted(&good).unwrap();
    assert!(session_signer_for(phrase).is_ok());

    // ⑥-2 EVM 시절 `0x…` 주소는 비번 없는 길에선 그대로 막히고(상태에 legacy_address),
    //     비번을 받은 길(결제 승인)에서 같은 니모닉으로 다시 계산해 고쳐 쓴다 — 암호문·라벨·활성은 그대로.
    let mut evm = read_encrypted().unwrap();
    evm.address = "0x036CbD53842c5426634e7929541eC2318f3dCF7e".into();
    if let Some(a) = evm.accounts.iter_mut().find(|a| a.index == 1) {
        a.address = "0xAbCdEf0000000000000000000000000000000001".into();
    }
    write_encrypted(&evm).unwrap();
    assert!(session_signer_for(phrase).is_err());
    assert!(get_wallet_status().unwrap().legacy_address);
    assert!(unlock_signer("wrong").is_err()); // 틀린 비번은 아무것도 안 고친다
    assert!(read_encrypted().unwrap().address.starts_with("0x"));
    assert_eq!(
        unlock_signer("pw").unwrap().address(),
        "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96"
    );
    let fixed = read_encrypted().unwrap();
    assert_eq!(fixed.address, "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96");
    assert_eq!(fixed.ciphertext, evm.ciphertext);
    assert_eq!(fixed.active, evm.active);
    let acc1 = fixed.accounts().into_iter().find(|a| a.index == 1).unwrap();
    assert_eq!(acc1.address, "AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD");
    assert!(!get_wallet_status().unwrap().legacy_address);
    assert!(session_signer_for(phrase).is_ok());

    // ⑥-3 「옛 형식」 화면의 비번 칸 — 같은 고치기, 상태를 돌려준다. 고칠 게 없으면 그대로.
    write_encrypted(&evm).unwrap();
    assert!(repair_wallet_address("wrong".into()).is_err());
    let st = repair_wallet_address("pw".into()).unwrap();
    assert!(!st.legacy_address);
    assert_eq!(read_encrypted().unwrap().address, "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96");
    let st = repair_wallet_address("pw".into()).unwrap();
    assert!(!st.legacy_address);
    write_encrypted(&good).unwrap();

    // ⑦ 요청 계정 대조: 계정 1 로 각인된 요청은 활성 0 에서 거부, 1 로 바꾸면 통과.
    //    옛 요청(from 빈 값)은 계정 0 의 요청 — 활성 0 에서만 통과. 주소가 맞아도 인덱스가 다르면 거부.
    let mut req = crate::ipc::PaymentRequest {
        id: "1".into(),
        token: "USDC".into(),
        to: "0x0".into(),
        amount: "1".into(),
        memo: String::new(),
        created: crate::store::now_secs(),
        kind: "x402".into(),
        resource: String::new(),
        chain_id: 0,
        account: 1,
        from: "AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD".into(),
        lease: false,
        svm: None,
        job: String::new(),
    };
    assert!(crate::ipc::ensure_request_account(&req).is_err());
    switch_account(1).unwrap();
    assert!(crate::ipc::ensure_request_account(&req).is_ok());
    req.account = 2; // 주소는 1 인데 인덱스 2 → 거부
    assert!(crate::ipc::ensure_request_account(&req).is_err());
    req.account = 1;
    req.from = String::new(); // 옛 요청(각인 없음) = 계정 0 의 요청
    switch_account(0).unwrap();
    assert!(crate::ipc::ensure_request_account(&req).is_ok());
    assert_eq!(crate::ipc::request_account_index(&req), 0);
    switch_account(2).unwrap(); // 활성이 0 이 아니면 옛 요청은 거절
    assert!(crate::ipc::ensure_request_account(&req).is_err());
    switch_account(0).unwrap();

    // ⑧ 내역은 계정별 파일: 고정한 계정 1 의 시도는 history-a1.json 에, 활성 0 의 것은 history.json 에.
    //    (settings.json 이 없고 지갑이 있으니 체인은 Sepolia = 접미 없음.)
    with_pinned_account(1, async {
        crate::history::log_attempt("USDC", "0xto", "1", "signed", "0xnonce1");
    })
    .await;
    crate::history::log_attempt("USDC", "0xto", "2", "sent", "0xhash0");
    let dir = home.join(".budu");
    assert!(dir.join("history-a1.json").exists());
    assert!(dir.join("history.json").exists());
    assert_eq!(crate::history::get_history(None).len(), 1); // 활성 0 의 눈엔 자기 것 하나
    let seen = with_pinned_account(1, async { crate::history::get_history(None).len() }).await;
    assert_eq!(seen, 1);

    // ⑨ 정산 교차 반영: 활성은 0 인데 정산은 계정 1 이 서명한 nonce → 1 의 파일이 갱신된다.
    fs::write(
        dir.join("x402_settlements.json"),
        r#"[{"nonce":"0xnonce1","tx":"0xsettle","success":true}]"#,
    )
    .unwrap();
    assert_eq!(crate::history::apply_x402_settlements(), 1);
    let a1 = fs::read_to_string(dir.join("history-a1.json")).unwrap();
    assert!(
        a1.contains("\"settled\"") && a1.contains("0xsettle"),
        "{a1}"
    );
    assert!(!dir.join("x402_settlements.json").exists());

    // ⑩ 승인 대기 중엔 전환·추가를 막는다 — 단 **요청이 각인한 계정으로 돌아가는 전환**은
    //    허용(전환과 요청 생성이 경합해 옛 계정으로 각인된 요청의 탈출구).
    //    활성 1 인 채로 계정 0 각인 요청이 놓인 상황을 만든다.
    switch_account(1).unwrap();
    req.from = "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96".into();
    req.account = 0;
    fs::write(
        dir.join("payment_request.json"),
        serde_json::to_string(&req).unwrap(),
    )
    .unwrap();
    assert!(switch_account(2).is_err()); // 딴 계정으로는 못 간다
    assert!(add_account("pw".into()).is_err());
    assert!(switch_account(0).is_ok()); // 각인 계정으로는 간다 → 이제 승인이 된다
    assert!(crate::ipc::ensure_request_account(&req).is_ok());
    assert!(switch_account(1).is_err()); // 되돌아가는 건 다시 막힌다
    fs::remove_file(dir.join("payment_request.json")).unwrap();
    assert!(switch_account(1).is_ok());

    // ⑪ 파일 실물: accounts 3개·active 1·주소 평문, 니모닉은 여전히 암호문뿐.
    let raw = fs::read_to_string(dir.join("wallet.enc")).unwrap();
    assert!(raw.contains("\"active\": 1"), "{raw}");
    assert!(raw.contains("CqMbRgMuEhQi9BUS8xP44Wk5nENm48FqJnfjEi4eNb1k"));
    assert!(!raw.contains("junk"));

    if std::env::var("BUDU_HARNESS_HOME").is_err() {
        let _ = fs::remove_dir_all(&home);
    }
}

// 같은 입력이라도 매번 솔트/논스가 달라 암호문이 달라야 한다.
// ── wallet.enc 저장 실패 갈래 ──────────────

const HARDHAT: &str = "test test test test test test test test test test test junk";

fn enc_bytes() -> Vec<u8> {
    fs::read(enc_path().unwrap()).unwrap()
}

/// 고쳐 쓰기(`write_encrypted`): 새 내용이 제자리에 못 갔으면 오류이고 **옛 파일 그대로**, 디렉터리 동기화만 실패했으면
/// 성공(새 내용은 제자리 — 실패로 돌리면 「했는데 실패」가 된다).
#[test]
fn wallet_enc_rewrite_failures() {
    let _d = crate::store::test_dir::use_temp("wallet-rewrite");
    import_wallet("pw".into(), HARDHAT.into()).unwrap();
    let before = enc_bytes();

    crate::store::fault::next(crate::store::fault::Fail::NotWritten);
    assert!(rename_account(0, "새 이름".into()).is_err());
    assert_eq!(enc_bytes(), before, "못 썼으면 옛 지갑 그대로");

    crate::store::fault::next(crate::store::fault::Fail::NotDurable);
    let st = rename_account(0, "새 이름".into()).expect("동기화만 실패 → 성공");
    assert_eq!(st.accounts[0].label, "새 이름");
    assert_eq!(read_encrypted().unwrap().accounts()[0].label, "새 이름", "제자리에 갔다");
    // 시드는 그대로 풀린다.
    assert_eq!(decrypt_wallet(&read_encrypted().unwrap(), "pw").unwrap().as_str(), HARDHAT);
}

/// 🔴 개발 80 — 새 지갑은 있는 wallet.enc 를 덮지 않는다. 「없나 보고 쓰기」 사이에 다른 앱이 먼저 만든 경우도
/// (`create_encrypted` 를 바로 불러) 막힌다. 새로 만들 때 동기화만 실패하면 성공으로 친다.
#[test]
fn new_wallet_never_overwrites_wallet_enc() {
    let _d = crate::store::test_dir::use_temp("wallet-create");
    crate::store::fault::next(crate::store::fault::Fail::NotDurable);
    import_wallet("pw".into(), HARDHAT.into()).expect("동기화만 실패 → 성공");
    let first = enc_bytes();

    assert!(create_wallet("other".into()).is_err(), "이미 지갑이 있다");
    assert!(import_wallet("other".into(), HARDHAT.into()).is_err());
    let other = build_imported_wallet(HARDHAT, "other").unwrap();
    let e = create_encrypted(&other).expect_err("확인 뒤 끼어든 경우 — 파일 단계에서 막힌다");
    assert!(e.contains("이미 파일") || e.contains("already exists"), "{e}");
    assert_eq!(enc_bytes(), first, "첫 지갑 그대로");
    assert!(decrypt_wallet(&read_encrypted().unwrap(), "pw").is_ok());
}

/// 🔴 개발 79 — 평문 wallet.json 은 유일한 사본일 수 있다. 암호화본이 **디스크까지** 내려가고 다시 풀린 뒤에만 지운다.
#[test]
fn migrate_keeps_plaintext_unless_encrypted_copy_is_durable() {
    let legacy = format!(
        r#"{{"mnemonic":"{HARDHAT}","address":"oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96"}}"#
    );
    for (fail, enc_left) in [
        (crate::store::fault::Fail::NotWritten, false),
        (crate::store::fault::Fail::NotDurable, true),
    ] {
        let _d = crate::store::test_dir::use_temp("wallet-migrate");
        fs::write(legacy_path().unwrap(), &legacy).unwrap();
        crate::store::fault::next(fail);
        assert!(migrate_wallet("pw".into()).is_err(), "{fail:?}");
        assert!(legacy_path().unwrap().exists(), "{fail:?}: 평문 원본을 남긴다");
        assert_eq!(enc_path().unwrap().exists(), enc_left, "{fail:?}");
    }
    let _d = crate::store::test_dir::use_temp("wallet-migrate");
    fs::write(legacy_path().unwrap(), &legacy).unwrap();
    let info = migrate_wallet("pw".into()).unwrap();
    assert_eq!(info.address, "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96");
    assert!(!legacy_path().unwrap().exists(), "확인 뒤에만 지운다");
    assert_eq!(decrypt_wallet(&read_encrypted().unwrap(), "pw").unwrap().as_str(), HARDHAT);
}

#[test]
fn ciphertext_is_randomized() {
    let phrase = "test test test test test test test test test test test junk";
    let a = encrypt_wallet(phrase, "0x0", "pw").unwrap();
    let b = encrypt_wallet(phrase, "0x0", "pw").unwrap();
    assert_ne!(a.ciphertext, b.ciphertext);
    assert_ne!(a.salt, b.salt);
}

/// 🔴 개발 78: 파일에 적힌 KDF 비용이 터무니없으면 할당하지 않고 거절한다.
#[test]
fn absurd_kdf_cost_is_refused_before_allocating() {
    assert!(derive_key("pw", &[0u8; 16], u32::MAX, 3, 1).is_err());
    assert!(derive_key("pw", &[0u8; 16], KDF_M_V3, u32::MAX, 1).is_err());
    assert!(derive_key("pw", &[0u8; 16], KDF_M_V3, 3, 64).is_err());
    assert!(derive_key("pw", &[0u8; 16], KDF_M_V2, KDF_T_V2, KDF_P_V2).is_ok());
}
