// 부두(Budu) 코어 — 로컬 전용 AI 에이전트 지갑.
//
// 모듈 지도 — 처음 읽는다면 docs/ARCHITECTURE.md 부터.
//   autostart 로그인 시 자동 시작 + 희망값 보존·복구
//   chain     클러스터 상수(솔라나 데브넷/메인넷 RPC·USDC 민트·genesis)
//   keys      BIP-39 + SLIP-0010 ed25519 키 파생(팬텀 호환 m/44'/501'/n'/0')
//   solana    (shared/solana.rs) 주소·ATA·잔액 JSON-RPC — MCP 와 같은 소스
//   connect   AI 연결 — 동봉 확장 열기·claude CLI 등록 대행·설치 감지
//   store     ~/.budu 경로, 원자적 파일 쓰기, 시간 헬퍼
//   wallet    니모닉 생성·주소 파생·비번 암호화(Argon2id + AES-256-GCM) + 계정 여러 개
//   settings  사용자 설정(한도·자율·RPC·잠금 동작)
//   limits    단일/일일 한도 검사 + 오늘 사용액 장부
//   lock      긴급 잠금 (비상 스위치)
//   trusted   화이트리스트(신뢰 주소) — 자율 결제 가드
//   history   거래 내역 + x402 정산 추적
//   i18n      화면 언어(한국어·영어) — 사람에게 보이는 문자열 두 벌
//   balance   잔액 조회(SOL + USDC)
//   x402      결제 정책 껍데기 — 조건 대조·잠금·한도 예약·블록해시·서명 호출·내역
//   svm_x402  결제 코어 — v0 메시지 손 직렬화·자체 검증·부분 서명(정답지 테스트로 고정)
//   confirm   정산 확정 — 체인에서 우리 서명을 찾아 settled/reverted/expired 로 끝내고, 안 나갔으면 한도 환불
//   ipc       MCP ↔ GUI 결제 승인 파일 IPC + AI 연결 배지 (ipc/watchdog = 생존 감시 스레드)
//   session   자율 결제 세션(메모리 키) + 자동 승인
//   job       작업 예산 — 사람이 연 «이 작업 전체를 N USDC 까지» (메모리 키)
//   job_names 작업 별명(화면 이름만, 예산 판정 열쇠는 AI 이름표 그대로)
//   receipt   영수증 CSV 내보내기
//   notify    OS 알림 (자율 결제 사후 통지)
//   tray      메뉴바 상주(옵션) — 트레이 아이콘 + 팝오버 위치·자동 숨김. 승인은 안 받는다
//   appwin    큰 창 — 앱의 본 창이자 결제 승인 창. 맨 위 고정·닫아 둠·되살리기
//
// 이 파일에는 앱 셸만 남긴다: 창 제어 커맨드 + run().

mod appwin;
mod autostart;
mod balance;
mod chain;
mod confirm;
mod connect;
mod history;
mod i18n;
mod ipc;
mod job;
mod job_names;
mod keys;
mod limits;
mod lock;
mod notify;
/// ~/.budu 파일 해석 규칙의 정본 — MCP(budu-mcp)와 **같은 소스 파일**을 컴파일한다.
/// 크레이트가 아니라 `#[path]` 모듈이라 Cargo·Tauri 빌드에 새 의존성은 없다. 조건은 파일 머리.
#[path = "../../shared/policy.rs"]
mod policy;
mod receipt;
mod session;
mod settings;
mod send;
/// 솔라나 주소·ATA·잔액 RPC — MCP 와 **같은 소스 파일**. 조건은 파일 머리.
#[path = "../../shared/solana.rs"]
mod solana;
mod store;
mod svm_x402;
mod tray;
mod trusted;
mod wallet;
mod x402;

use session::SessionKey;

/// 자리비움 잠금이 blur 뒤 기다리는 시간(ms) — 같은 앱의 다른 창이 포커스를 받는 데 충분하고,
/// 사람이 다른 앱에서 무언가를 하기엔 짧다.
const BLUR_LOCK_DELAY_MS: u64 = 300;

/// 결제 승인 창이 다른 창에 가려져 5분 타임아웃을 놓친 실사례 → 사람 승인이 필요한 동안 창을 전면 고정.
/// macOS 14+는 백그라운드 앱의 포커스 뺏기(activateIgnoringOtherApps)를 무시하므로 set_focus만으론
/// 안 뜬다 — 항상 위(always-on-top)로 고정해 두고, 승인이 끝나면 release_main_window로 푼다.
///
/// 승인 창 = 큰 창(appwin.rs). 프론트의 승인 모달은 큰 창에서만 돌므로 이 커맨드도 큰 창만 부른다.
#[tauri::command]
fn raise_main_window(app: tauri::AppHandle) {
    // 이미 처리된 요청에 대한 늦은 호출이면 아무것도 하지 않는다(항상-위가 켜진 채 남지 않게).
    let Some(req) = ipc::live_request() else {
        return;
    };
    // 사용자가 이 요청의 창을 일부러 닫아 뒀으면 다시 띄우지 않는다. 프론트는 요청
    // id 가 바뀔 때 한 번 부르지만, 감시 스레드가 깨운 창을 프론트가 요청을 그리기 전에 닫으면
    // 그 한 번이 닫은 **뒤**에 도착한다. 고정도 걸지 않는다 — 숨은 창에 고정만 남으면 다음에 꺼낼 때 박힌다.
    if appwin::user_dismissed(&app, &req.id) {
        return;
    }
    appwin::set_pinned(&app, true);
    appwin::show(&app);
}

/// 사용자가 ⌘W 로 창을 닫는다.
///
/// 🔴 이 커맨드가 없으면 팝오버에서 ⌘W 는 **아무 일도 하지 않는다**. macOS 에서 ⌘W 는 앱 메뉴의
/// 「닫기」(`performClose:`)로 가는데, 팝오버는 `decorations: false` 라 닫기 버튼이 없어
/// AppKit 이 그 항목을 비활성화한다 → `CloseRequested` 가 아예 안 날아온다. 개발 53 은
/// 그 이벤트에 `hide_by_user` 를 달아 놓고 「⌘W 로도 닫힌다」고 적었지만, 실물에서 키를
/// 눌러 본 적이 없어(보조 접근 권한이 없어 자동화가 막혔다) 실물 확인에서야
/// 드러났다. 릴리스 노트 v0.4.0 에도 된다고 적혀 있었다 — 발행 전에 잡았다.
///
/// 창 닫기(`CloseRequested`)와 **같은 `hide_by_user` 를 탄다** — 「닫아 둠」 표식·만료
/// 직전 되살리기 규칙이 두 경로에서 갈리지 않게.
///
/// 부른 창을 보고 가른다 — 웹뷰의 ⌘W 처리기는 두 창이 같은 코드다. 큰 창(승인 창)은 숨기면서
/// 대기 요청을 「닫아 둠」으로 적고(`appwin::hide_by_user`), 팝오버는 그냥 숨긴다. (큰 창은 테두리가 있어
/// AppKit 의 「닫기」도 살아 있다 — 둘 다 와도 두 번째는 이미 숨은 창이라 같은 결과다.)
#[tauri::command]
fn hide_main_window(app: tauri::AppHandle, window: tauri::WebviewWindow) {
    if window.label() == appwin::LABEL {
        appwin::hide_by_user(&app);
    } else {
        tray::hide(&app);
    }
}

/// 승인 처리(승인/거부/타임아웃)가 끝나면 항상-위 고정을 즉시 재조정.
/// 다음 요청이 이미 대기 중이면 켜진 채로 남는다(A→B 전환에서 A 의 늦은 해제가 B 의 전면
/// 고정을 꺼버리지 않게). 어차피 1초 폴링도 같은 값으로 수렴시킨다 — 이건 그 대기를 없애는 용.
#[tauri::command]
fn release_main_window(app: tauri::AppHandle) {
    appwin::sync_always_on_top(&app);
}

/// 로그인 자동 시작으로 켜졌나 — 그때는 창을 띄우지 않고 조용히 상주한다(autostart 플러그인이 붙이는 인자).
pub(crate) const AUTOSTART_ARG: &str = "--autostart";

fn launched_by_autostart() -> bool {
    std::env::args().any(|a| a == AUTOSTART_ARG)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 같은 데이터 폴더를 쓰는 앱은 하나만 — 트레이·창을 만들기 전에 정한다.
    // 못 쥔 채(Unavailable — 권한 등) 뜨면 화면·잔액은 보이되 결제는 `limits::reserve_spend` 가 막는다.
    match store::hold_app_lock(std::time::Duration::from_secs(8)) {
        store::AppLock::Busy => {
            eprintln!("[budu] 같은 지갑 폴더를 쓰는 Budu 가 이미 떠 있어요 — 이 실행은 끝냅니다");
            return;
        }
        store::AppLock::Unavailable => {
            eprintln!("[budu] 지갑 폴더 잠금을 못 잡았어요 — 결제는 막힌 채로 뜹니다");
        }
        store::AppLock::Held => {}
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec![AUTOSTART_ARG]),
        ))
        .manage(SessionKey::default())
        .manage(tray::PopoverState::default())
        .manage(appwin::ApprovalState::default())
        .setup(|app| {
            // 언어를 제일 먼저 정한다 — 트레이 메뉴부터 이미 사람이 읽는 글자다.
            i18n::init();
            // 메뉴바 아이콘은 옵션 — 설정이 꺼져 있으면 안 만든다. 못 만들어도 앱은 뜬다(도크·큰 창).
            if settings::read_settings().menubar {
                if let Err(e) = tray::build(app.handle()) {
                    eprintln!("[budu] 메뉴바 아이콘을 못 만들었어요: {e}");
                }
            }
            // 업데이트가 자동 시작 설정을 지우고 갔는지 확인해 되살린다.
            // 트레이보다 뒤, 창을 띄우기 전 — 실패해도 앱은 그대로 뜬다.
            autostart::reconcile(app.handle());
            // 진짜 신규(설정도 지갑도 없음)면 신규 기본(메인넷)을 파일에 박는다 — 안 박으면 지갑을 만든 순간
            // 「파일 없음 + 지갑 있음」 갈래(데브넷)로 넘어가 화면·MCP 의 체인이 바뀐다. reconcile 이 OS 상태를
            // 못 읽어 저장을 건너뛴 경우의 안전망.
            settings::pin_new_install();
            // Claude Code 등록이 이 설치본을 가리키는지 보고 어긋났으면 고친다. 새 등록은 안 만든다.
            connect::spawn_heal();
            // 로그인 셸의 CODEX_HOME — Codex 등록 확인이 맞는 설정 폴더를 보게.
            connect::spawn_codex_home_probe();
            // 생존 감시 — 하트비트를 러스트가 찍고, 프론트가 잠든 사이 온 결제 요청에
            // 창을 깨운다. 트레이가 만들어진 뒤여야 창을 트레이 아래로 배치할 수 있다.
            ipc::spawn_watchdog(app.handle());
            // 정산 확정 — 서명한 결제의 결말을 체인에서 찾고, 만료면 한도를 돌려준다.
            confirm::spawn();
            // 켜면 큰 창이 뜬다. 로그인 자동 시작일 때만 조용히 상주한다 —
            // 단 지갑이 없으면(첫 실행) 그때도 띄운다: 아무것도 안 뜨면 뭘 해야 할지 알 수 없다.
            if !launched_by_autostart() || wallet::needs_setup() {
                appwin::show(app.handle());
            }
            Ok(())
        })
        .on_window_event(|window, event| match event {
            // 상시 구동: 창 닫기는 종료가 아니라 숨기기 — 백그라운드에서 결제 요청을
            // 계속 받는다. 완전 종료는 Cmd+Q(ExitRequested는 안 막음) 또는 트레이 메뉴의 "종료".
            tauri::WindowEvent::CloseRequested { api, .. } => {
                use tauri::Manager;
                api.prevent_close();
                if window.label() == appwin::LABEL {
                    // 승인 창. 승인 대기 중에도 숨긴다 — 단 「닫아 둠」으로 적어, 감시
                    // 스레드가 도로 띄우지 않고 만료 직전 한 번만 되살린다.
                    appwin::hide_by_user(window.app_handle());
                } else {
                    tray::hide(window.app_handle());
                }
            }
            tauri::WindowEvent::Focused(false) => {
                use tauri::Manager;
                // 자리비움 자동 잠금: 설정이 켜져 있으면 세션 키를 소멸.
                // 창이 둘이 된 뒤로는 **Budu 밖으로 나갔을 때만** — 팝오버에서 큰 창으로
                // 옮기는 순간에도 팝오버는 blur 를 받는다. 새 창의 Focused(true) 는 blur 직후에 오므로
                // 잠깐 기다렸다 Budu 창 중 아무도 포커스가 없을 때 잠근다. 메인 스레드를 막지 않게 따로.
                if settings::read_settings().lock_on_blur {
                    let app = window.app_handle().clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(BLUR_LOCK_DELAY_MS));
                        if !appwin::any_focused(&app) {
                            if let Ok(mut g) = app.state::<SessionKey>().0.lock() {
                                *g = None;
                            }
                            job::clear(); // 작업 예산의 키도
                        }
                    });
                }
                // 팝오버 자동 숨김. 승인 대기 중이면 tray 쪽에서 걸러 안 숨긴다.
                // 큰 창은 보통 맥 창이라 다른 앱을 눌러도 그 자리에 있는다.
                if window.label() == "main" {
                    tray::on_blur(window);
                }
            }
            _ => {}
        })
        .invoke_handler(tauri::generate_handler![
            wallet::get_wallet_status,
            wallet::create_wallet,
            wallet::migrate_wallet,
            wallet::import_wallet,
            wallet::reveal_mnemonic,
            wallet::repair_wallet_address,
            wallet::mark_backed_up,
            wallet::add_account,
            wallet::switch_account,
            wallet::rename_account,
            balance::get_balances,
            settings::get_settings,
            settings::settings_file_broken,
            settings::set_settings,
            settings::save_settings_edit,
            limits::get_today_spend,
            limits::limit_room,
            lock::is_locked,
            lock::set_locked,
            history::get_history,
            history::recent_same_payment_secs,
            ipc::get_pending_request,
            ipc::approve_payment,
            ipc::reject_payment,
            ipc::get_agent_status,
            session::unlock_session,
            session::lock_session,
            session::session_status,
            session::auto_approve_payment,
            history::apply_x402_settlements,
            confirm::take_chain_updates,
            job::job_status,
            job::end_job,
            job::resume_job,
            job_names::get_job_names,
            job_names::rename_job,
            receipt::export_job_receipt,
            trusted::is_trusted_addr,
            trusted::get_trusted_addrs,
            trusted::remove_trusted_addr,
            trusted::add_trusted_addr,
            raise_main_window,
            release_main_window,
            hide_main_window,
            appwin::open_app_window,
            appwin::show_approval,
            autostart::get_autostart,
            autostart::set_autostart,
            settings::set_lang,
            settings::get_lang,
            settings::get_menubar,
            settings::set_menubar,
            connect::get_connect_status,
            connect::get_connect_ready,
            connect::connect_claude_desktop,
            connect::connect_claude_code,
            connect::connect_codex,
            send::send_funds,
            send::get_sends,
            send::check_sends,
            send::send_rules,
            send::usdc_send_quote
        ])
        .build(tauri::generate_context!())
        .expect("error while running tauri application")
        .run(|app, event| {
            // 숨겨진 창을 Dock 아이콘 클릭으로 복원 (macOS applicationShouldHandleReopen).
            // 개발 7 부터 큰 창이 본 창이자 승인 창 — 독을 누르면 언제나 그 창.
            if let tauri::RunEvent::Reopen { .. } = event {
                appwin::show(app);
            }
        });
}
