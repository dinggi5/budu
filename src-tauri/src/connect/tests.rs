use super::*;

// 확장 감지: manifest 의 name 이 정확히 "budu" 일 때만 우리 것으로 본다.
#[test]
fn manifest_detection() {
    assert!(manifest_is_budu(r#"{"name":"budu","version":"0.1.2"}"#));
    assert!(!manifest_is_budu(r#"{"name":"budu-fake"}"#));
    assert!(!manifest_is_budu(r#"{"version":"1.0"}"#)); // name 없음
    assert!(!manifest_is_budu("not json"));
}

// CLI 등록 감지: 사용자 범위 mcpServers 의 command 를 돌려준다(이름만 보지 않는다 —
// 옛 경로를 가리키는 등록을 "등록됨"으로 속이지 않기 위해.).
#[test]
fn claude_json_detection() {
    assert_eq!(
        claude_json_budu_command(r#"{"mcpServers":{"budu":{"command":"/a/budu-mcp"}}}"#),
        Some("/a/budu-mcp".to_string())
    );
    assert_eq!(
        claude_json_budu_command(r#"{"mcpServers":{"playwright":{}}}"#),
        None
    );
    // command 없는 항목(HTTP 서버 등)은 경로 대조가 불가 → None.
    assert_eq!(
        claude_json_budu_command(r#"{"mcpServers":{"budu":{}}}"#),
        None
    );
    // 프로젝트 범위에만 있는 건 등록으로 안 친다.
    assert_eq!(
        claude_json_budu_command(
            r#"{"projects":{"/a":{"mcpServers":{"budu":{"command":"x"}}}}}"#
        ),
        None
    );
    assert_eq!(claude_json_budu_command("{}"), None);
    assert_eq!(claude_json_budu_command("broken"), None);
}

// 원복용 항목 스냅숏: command 가 없어도 항목이 있으면 통째로 돌려준다(HTTP 서버
// 등록도 지웠으면 되돌려야 한다). null·부재는 "항목 없음".
#[test]
fn claude_json_entry_snapshot() {
    let json = r#"{"mcpServers":{"budu":{"type":"stdio","command":"/a/budu-mcp","args":[]}}}"#;
    let entry = claude_json_budu_entry(json).expect("항목이 있어야 한다");
    assert_eq!(entry["command"], "/a/budu-mcp");
    assert_eq!(entry["type"], "stdio");
    assert!(claude_json_budu_entry(r#"{"mcpServers":{"budu":{}}}"#).is_some());
    assert_eq!(
        claude_json_budu_entry(r#"{"mcpServers":{"budu":null}}"#),
        None
    );
    assert_eq!(claude_json_budu_entry(r#"{"mcpServers":{}}"#), None);
    assert_eq!(claude_json_budu_entry("{}"), None);
}

// 임시 실행 위치 감지: Translocation 은 무조건 임시,
// /Volumes 는 읽기 전용(DMG)일 때만 임시 — 외장 SSD 설치(쓰기 가능)는 정상 취급.
#[test]
fn temp_location_detection() {
    use std::path::Path;
    let ro = |_: &Path| true; // DMG 처럼 읽기 전용 볼륨
    let rw = |_: &Path| false; // 외장 SSD 처럼 쓰기 가능 볼륨
    assert!(classify_temp_location(
        Path::new(
            "/private/var/folders/ab/xyz/T/AppTranslocation/UUID/d/Budu.app/Contents/MacOS/budu"
        ),
        rw, // Translocation 은 볼륨 상태와 무관하게 임시
    ));
    assert!(classify_temp_location(
        Path::new("/var/folders/ab/xyz/T/AppTranslocation/UUID/d/Budu.app/Contents/MacOS/budu"),
        rw,
    ));
    assert!(classify_temp_location(
        Path::new("/Volumes/Budu 0.1.2/Budu.app/Contents/MacOS/budu"),
        ro,
    ));
    assert!(!classify_temp_location(
        Path::new("/Volumes/외장SSD/Applications/Budu.app/Contents/MacOS/budu"),
        rw,
    ));
    assert!(!classify_temp_location(
        Path::new("/Applications/Budu.app/Contents/MacOS/budu"),
        ro, // 프리픽스가 아니면 볼륨 상태는 보지도 않는다
    ));
    assert!(!classify_temp_location(
        Path::new("/Users/a/Applications/Budu.app/Contents/MacOS/budu"),
        rw,
    ));
    assert!(!classify_temp_location(
        Path::new("/Users/a/프로젝트/지갑지갑/src-tauri/target/debug/budu"),
        rw,
    ));
}

// 임시 실행 중 설치본 버전 대조. 등록될 budu-mcp 는 설치본
// 안의 것이라, 그게 지금 화면을 그리는 이 앱과 다른 버전이면 사용자에게 알려야 한다.
#[test]
fn installed_version_mismatch_detection() {
    use std::path::{Path, PathBuf};
    let mcp: PathBuf = "/Applications/Budu.app/Contents/MacOS/budu-mcp".into();
    let reads = |v: &'static str| move |_: &Path| Some(v.to_string());

    // 임시 실행이 아니면 볼 것도 없다 — mcp_path 는 이 앱 자신의 사이드카다.
    assert_eq!(
        classify_installed_mismatch(false, Some(&mcp), "0.2.1", reads("0.2.0")),
        None
    );
    // 임시 실행 + 설치본 버전이 다름 → 그 버전을 알린다.
    assert_eq!(
        classify_installed_mismatch(true, Some(&mcp), "0.2.1", reads("0.2.0")),
        Some("0.2.0".to_string())
    );
    // 같은 버전이면 조용히.
    assert_eq!(
        classify_installed_mismatch(true, Some(&mcp), "0.2.1", reads("0.2.1")),
        None
    );
    // 설치본이 아예 없어 등록할 경로가 없는 상태(프론트는 "옮기기" 안내를 띄운다).
    assert_eq!(
        classify_installed_mismatch(true, None, "0.2.1", reads("0.2.0")),
        None
    );
    // 버전을 못 읽으면 막지 않는다 — 모르는 걸 경고로 바꾸지 않는다.
    assert_eq!(
        classify_installed_mismatch(true, Some(&mcp), "0.2.1", |_: &Path| None),
        None
    );
    // 조상 셋을 못 올라가는 짧은 경로 → None (패닉이 아니라).
    assert_eq!(
        classify_installed_mismatch(true, Some(Path::new("/budu-mcp")), "0.2.1", reads("0.2.0")),
        None
    );
}

// 실물 plutil 스모크: 이 맥에 깔린 Budu.app 이 있으면 버전 문자열이 나와야 하고,
// 없는 번들은 None 이어야 한다(못 읽는다고 패닉하지 않는다).
#[test]
fn bundle_version_smoke() {
    use std::path::Path;
    assert_eq!(bundle_short_version(Path::new("/no/such/App.app")), None);
    let installed = Path::new("/Applications/Budu.app");
    if installed.is_dir() {
        let v = bundle_short_version(installed).expect("설치된 앱은 버전이 읽혀야 한다");
        assert!(
            v.chars().next().is_some_and(|c| c.is_ascii_digit()),
            "버전 같지 않은 값: {v}"
        );
    }
}

// 확장 꺼 둠 감지: isEnabled 가 **명시적으로 false** 일 때만. 모르면 꺼졌다고 단정하지 않는다.
#[test]
fn ext_disabled_detection() {
    assert!(ext_settings_disabled(r#"{"isEnabled": false}"#));
    assert!(!ext_settings_disabled(r#"{"isEnabled": true}"#)); // 실측 모양
    assert!(!ext_settings_disabled(r#"{}"#));
    assert!(!ext_settings_disabled(r#"{"isEnabled": "false"}"#));
    assert!(!ext_settings_disabled(""));
    assert!(!ext_settings_disabled("broken"));
}

// 자가 복구 대상: 우리 것이었고 그대로 두면 죽거나 옛 사본을 띄우는 등록만 고친다.
#[test]
fn heal_only_dead_temp_or_other_bundle() {
    use std::path::Path;
    let ours = Path::new("/Applications/Budu.app/Contents/MacOS/budu-mcp");
    let alive = |_: &Path| true;
    let dead = |_: &Path| false;
    let not_temp = |_: &Path| false;
    let temp = |_: &Path| true;

    // 이미 이 앱 → 할 일 없음.
    assert!(!should_heal(ours.to_str().unwrap(), ours, alive, not_temp));
    // 앱을 옮기거나 지워 죽은 등록 → 고친다(번들 밖 경로라도 — 죽었으면 누구에게도 쓸모없다).
    assert!(should_heal("/Users/a/Desktop/Budu.app/Contents/MacOS/budu-mcp", ours, dead, not_temp));
    assert!(should_heal("/Users/a/old/target/release/budu-mcp", ours, dead, not_temp));
    // 디스크 이미지에서 등록해 둔 것 → 고친다.
    assert!(should_heal("/Volumes/Budu 0.4.3/Budu.app/Contents/MacOS/budu-mcp", ours, alive, temp));
    // 살아 있는 다른 Budu.app 사본 → 지금 쓰는 이 앱으로.
    assert!(should_heal("/Users/a/Applications/Budu.app/Contents/MacOS/budu-mcp", ours, alive, not_temp));
    // 살아 있는 dev 빌드 → 개발자가 일부러 고른 것. 안 건드린다.
    assert!(!should_heal("/Users/a/지갑지갑/target/debug/budu-mcp", ours, alive, not_temp));
    // 이름이 budu-mcp 가 아니면(래퍼 등) 우리 것인지 모른다 — 죽었어도 안 건드린다.
    assert!(!should_heal("/usr/local/bin/my-budu-wrapper", ours, dead, not_temp));
    assert!(!should_heal("", ours, dead, not_temp));
}

// 실물 claude CLI 로 자가 복구의 add-json 경로를 돈다. 사용자의 ~/.claude.json 대신 임시
// CLAUDE_CONFIG_DIR 을 쓴다 — 환경 변수를 바꾸므로 혼자 돌린다:
//   cargo test reregister_json_real -- --ignored --test-threads=1
// 보는 것: command 만 바뀌고 사용자가 붙인 env·args 는 그대로 남는다.
#[test]
#[ignore]
fn reregister_json_real() {
    let Some(cli) = find_claude_cli() else {
        eprintln!("claude CLI 없음 — 건너뜀");
        return;
    };
    let dir = std::env::temp_dir().join(format!("budu-heal-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::env::set_var("CLAUDE_CONFIG_DIR", &dir);
    let old = r#"{"type":"stdio","command":"/gone/Budu.app/Contents/MacOS/budu-mcp","args":["--x"],"env":{"BUDU_X":"1"}}"#;
    let seeded = Command::new(&cli)
        .args(["mcp", "add-json", "--scope", "user", "budu", old])
        .output()
        .unwrap();
    assert!(seeded.status.success(), "{}", String::from_utf8_lossy(&seeded.stderr));
    let entry = registered_cli_entry().expect("심은 항목이 읽혀야 한다");
    let mut next = entry.clone();
    next["command"] = serde_json::Value::String("/Applications/Budu.app/Contents/MacOS/budu-mcp".into());
    if let Err(e) = reregister(&cli, AddWith::Json(next.to_string())) {
        panic!("{}", e.message);
    }
    let after = registered_cli_entry().expect("다시 등록된 항목");
    assert_eq!(after["command"], "/Applications/Budu.app/Contents/MacOS/budu-mcp");
    assert_eq!(after["env"]["BUDU_X"], "1");
    assert_eq!(after["args"][0], "--x");
    let _ = std::fs::remove_dir_all(&dir);
}

// 실물 스모크: 이 맥에 budu 확장이 깔려 있으면 그 폴더를 찾고, 설정 파일 이름(<폴더명>.json)으로
// 켬/끔을 읽을 수 있어야 한다. 안 깔린 맥에선 둘 다 「없음·안 꺼짐」.
#[test]
fn desktop_ext_smoke() {
    match budu_ext_dir() {
        Some(dir) => {
            assert!(desktop_ext_installed());
            let id = dir.file_name().unwrap().to_string_lossy().into_owned();
            let settings = claude_support_dir()
                .unwrap()
                .join("Claude Extensions Settings")
                .join(format!("{id}.json"));
            if let Ok(json) = std::fs::read_to_string(settings) {
                assert_eq!(desktop_ext_disabled(), ext_settings_disabled(&json));
            }
        }
        None => assert!(!desktop_ext_disabled()),
    }
}

#[test]
fn app_bundle_exe_detection() {
    use std::path::Path;
    assert!(is_app_bundle_exe(Path::new("/Applications/Budu.app/Contents/MacOS/budu")));
    assert!(!is_app_bundle_exe(Path::new("/a/target/debug/budu")));
    assert!(!is_app_bundle_exe(Path::new("/Applications/Budu.app/Contents/Resources/budu")));
    assert!(!is_app_bundle_exe(Path::new("/budu")));
}

// 원복 명령의 셸 인용: JSON 은 따옴표를 반드시 물고 있고, 값 안에 작은따옴표가
// 들어와도 명령이 쪼개지면 안 된다.
#[test]
fn restore_command_quoting() {
    assert_eq!(
        shell_single_quote(r#"{"command":"/a/budu-mcp"}"#),
        r#"'{"command":"/a/budu-mcp"}'"#
    );
    assert_eq!(
        shell_single_quote("/Users/a/it's here/budu-mcp"),
        r#"'/Users/a/it'\''s here/budu-mcp'"#
    );
}

// 실물 statfs 스모크: 루트 볼륨(쓰기 가능한 데이터 볼륨에 병합 마운트)은 이 판정에서
// 읽기 전용이 아니어야 하고, 없는 경로는 판별 불가 → false(사용자를 막지 않는다).
#[test]
fn volume_read_only_smoke() {
    use std::path::Path;
    assert!(!volume_is_read_only(Path::new("/Applications")));
    assert!(!volume_is_read_only(Path::new("/no/such/path/anywhere")));
}

#[test]
fn codex_toml_detection() {
    let t = r#"
model = "x"
[mcp_servers.other]
command = "/nope"

[mcp_servers.budu]
command = "/Applications/Budu.app/Contents/MacOS/budu-mcp"
args = []
"#;
    assert_eq!(codex_toml_budu_command(t).as_deref(), Some("/Applications/Budu.app/Contents/MacOS/budu-mcp"));
    let q = "[mcp_servers.\"budu\"]\ncommand = 'C:\\x y'\n";
    assert_eq!(codex_toml_budu_command(q).as_deref(), Some("C:\\x y"));
    let esc = "[mcp_servers.budu]\ncommand = \"/a \\\"b\\\"\"\n";
    assert_eq!(codex_toml_budu_command(esc).as_deref(), Some("/a \"b\""));
    // 다른 표의 command 를 잡지 않는다.
    assert_eq!(codex_toml_budu_command("[mcp_servers.other]\ncommand = \"/x\"\n"), None);
    assert_eq!(codex_toml_budu_command("[mcp_servers.budu]\nargs = []\n[x]\ncommand = \"/y\"\n"), None);
    assert_eq!(codex_toml_budu_command(""), None);
}

#[test]
fn shell_codex_home_is_read_between_marks() {
    // rc 파일이 찍은 글자가 앞뒤에 섞여도 표지 사이만 읽는다.
    let out = "Welcome!\n__BUDU_CODEX_HOME__/Users/a/.codex-work__BUDU_CODEX_HOME__\n";
    assert_eq!(parse_shell_codex_home(out).as_deref(), Some("/Users/a/.codex-work"));
    // 비어 있으면(셸에도 없음) None → ~/.codex 로 물러선다.
    assert_eq!(parse_shell_codex_home("__BUDU_CODEX_HOME____BUDU_CODEX_HOME__"), None);
    // 표지가 없거나 하나뿐이면(셸 오류) None.
    assert_eq!(parse_shell_codex_home("zsh: command not found"), None);
    assert_eq!(parse_shell_codex_home("__BUDU_CODEX_HOME__/x"), None);
}
