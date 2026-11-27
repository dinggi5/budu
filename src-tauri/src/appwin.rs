// 큰 창 — 부두의 본 창이자 **결제 승인 창**.
//
// 설계 결정:
//   · **앱을 켜면 이 창이 뜬다**(보통 맥 앱). 메뉴바 팝오버(`main`, tray.rs)는 설정의 옵션이고
//     잔액·상태·「창 열기」만 한다 — **승인은 받지 않는다.** 승인하는 창이 하나뿐이라 두 창이 같은
//     요청을 동시에 자율 승인하려 드는 갈래가 원천적으로 없다(옛 개발 75 의 반대 방향: 그때는
//     팝오버가 승인하고 이 창이 「승인하러 가기」 띠만 보였다).
//   · 그래서 팝오버가 하던 «승인 창의 일» 이 전부 이리로 왔다: 요청이 오면 앞으로 + 맨 위 고정
//     (`set_pinned`), 사용자가 일부러 닫은 요청은 다시 안 띄우고 만료 1분 전에 한 번만 되살리기
//     (`dismissal`), 감시 스레드가 잠든 웹뷰를 깨우기(ipc::watchdog).
//   · **닫으면 숨긴다**(없애지 않는다). 창을 닫아도 앱은 백그라운드에서 결제 요청을 받는다 —
//     요청이 오면 감시 스레드가 이 창을 다시 띄운다. 창이 아예 없을 때(로그인 자동 시작으로 조용히
//     켜진 직후)는 `show` 가 새로 만든다. 완전 종료는 ⌘Q 나 메뉴바 메뉴의 「종료」.
//   · 상태는 백엔드가 정본이라 두 창이 같은 값을 본다. 한쪽에서 바꾼 걸 다른 쪽이 다시 읽는 건
//     프론트의 `budu-sync` 이벤트와 포커스 복귀가 맡는다(src/lib/sync.ts).

use std::sync::Mutex;

use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// 창 라벨 — capabilities/default.json 의 `windows` 에도 같은 이름이 있어야 커맨드가 닿는다.
pub(crate) const LABEL: &str = "window";

/// 결제 승인을 받는 창의 라벨. 프론트의 `isApprover`(src/lib/win.ts)와 같은 뜻 — 둘이 어긋나면
/// 승인 모달을 그리는 창과 감시 스레드가 깨우는 창이 갈린다(드리프트 가드 테스트 참고).
pub(crate) const APPROVER: &str = LABEL;

/// 처음 여는 크기(논리 px). 사이드바 + 본문 두 단이 들어가는 폭.
const OPEN_W: f64 = 960.0;
const OPEN_H: f64 = 720.0;
/// 이보다 좁히면 승인 모달(카드 폭 448 + 여백)이 잘린다.
const MIN_W: f64 = 480.0;
const MIN_H: f64 = 560.0;
const _: () = assert!(MIN_W > 420.0 && OPEN_W >= MIN_W && OPEN_H >= MIN_H);

/// 닫아 둔 승인 창을 만료 이만큼 전에 한 번 되살린다(초). 개발 53.
///
/// 사용자가 닫은 건 「지금 말고」지 「영영」이 아니다 — 그런데 MCP 는 5분이 지나면 요청을 거둬
/// 가므로, 닫아 둔 채 잊으면 결제가 조용히 시간 초과된다. 60초면 비번을 넣기에 충분하다.
pub(crate) const REMIND_SECS: u64 = 60;

#[derive(Default)]
pub(crate) struct ApprovalState {
    /// 사용자가 **일부러 닫아 둔** 승인 요청의 id. 요청 id 단위라 다음 요청은 평소대로
    /// 창을 깨운다. 어떤 경로로든 창이 다시 뜨면(`show`) 지운다.
    dismissed: Mutex<Option<String>>,
    /// 만료 직전 한 번 되살린 요청의 id — 같은 요청을 두 번 되살리지 않는다.
    reminded: Mutex<Option<String>>,
}

/// 사용자가 닫아 둔 요청인가.
pub(crate) fn user_dismissed<R: Runtime>(app: &AppHandle<R>, req_id: &str) -> bool {
    app.state::<ApprovalState>()
        .dismissed
        .lock()
        .map(|g| g.as_deref() == Some(req_id))
        .unwrap_or(false)
}

/// 닫아 둔 승인 창을 지금 어떻게 할지 (순수 — 테스트 가능).
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Dismissal {
    /// 닫아 둔 요청이 아니다 — 평소 규칙(잠든 프론트면 깨운다)대로.
    No,
    /// 닫아 둔 채 둔다.
    Hold,
    /// 만료 직전 — 한 번 되살린다.
    Remind,
}

/// `dismissed`·`reminded` = 상태에 적힌 id, `req_id` = 지금 대기 중인 요청, `secs_left` = 그 요청의
/// 남은 승인 시간. 이미 0 이면 되살리지 않는다 — MCP 가 곧 거둬 갈 요청을 띄워 봐야 비번을 넣는
/// 순간 「시간이 지났어요」다(살아 있는 요청으로 잡히는 유예 60초 동안의 일).
pub(crate) fn dismissal(
    dismissed: Option<&str>,
    reminded: Option<&str>,
    req_id: &str,
    secs_left: u64,
) -> Dismissal {
    if dismissed != Some(req_id) {
        return Dismissal::No;
    }
    if reminded == Some(req_id) || secs_left == 0 || secs_left > REMIND_SECS {
        return Dismissal::Hold;
    }
    Dismissal::Remind
}

/// 감시 스레드용: 닫아 둔 요청을 지금 되살려야 하면 **되살린 것으로 적고** Remind 를 돌려준다.
pub(crate) fn dismissal_for<R: Runtime>(
    app: &AppHandle<R>,
    req_id: &str,
    secs_left: u64,
) -> Dismissal {
    let st = app.state::<ApprovalState>();
    let (Ok(d), Ok(mut r)) = (st.dismissed.lock(), st.reminded.lock()) else {
        return Dismissal::No;
    };
    let verdict = dismissal(d.as_deref(), r.as_deref(), req_id, secs_left);
    if verdict == Dismissal::Remind {
        *r = Some(req_id.to_string());
    }
    verdict
}

/// "항상 위" 고정을 원하는 값으로 맞춘다. 1초 폴링(get_pending_request)마다 불려 어긋난 상태를
/// 스스로 고친다 — 캐시를 두지 않는 이유는 옛 tray.rs 주석 그대로(창이 아직 없을 때의 실패가
/// 적용된 것처럼 남으면 자가 치유가 고착의 원인이 된다).
pub(crate) fn set_pinned<R: Runtime>(app: &AppHandle<R>, pinned: bool) {
    if let Some(w) = app.get_webview_window(APPROVER) {
        let _ = w.set_always_on_top(pinned);
    }
}

/// 지금 고정해야 하나 = 살아 있는 대기 요청이 있고, 사용자가 그 요청의 창을 닫아 두지 않았다.
/// 닫아 둔 요청까지 고정하면 숨은 창이 고정을 단 채 남아, `show` 를 거치지 않는 경로가 생기는 순간 다른 앱 위에 박힌다.
pub(crate) fn should_pin<R: Runtime>(app: &AppHandle<R>, req: Option<&crate::ipc::PaymentRequest>) -> bool {
    req.is_some_and(|r| !user_dismissed(app, &r.id))
}

/// 지금 상태(대기 요청 유무·닫아 둠)에 맞춰 고정을 재조정한다.
pub(crate) fn sync_always_on_top<R: Runtime>(app: &AppHandle<R>) {
    let live = crate::ipc::live_request();
    set_pinned(app, should_pin(app, live.as_ref()));
}

/// 창을 띄운다 — 없으면 만들고, 숨어 있거나 최소화돼 있으면 꺼내 앞으로.
///
/// 어떤 경로로든 창이 다시 떴다 = 「닫아 둠」은 끝났다: 독·메뉴·팝오버의 「창 열기」·
/// 만료 직전 되살리기 전부. 승인 대기 중이면 맨 위 고정도 같이 — 새로 만든 창은 고정이 안 걸린
/// 채 태어나므로 다음 폴링(1초)을 기다리지 않게 여기서 건다.
pub(crate) fn show<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window(LABEL) {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    } else {
        let built = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::default())
            .title("Budu")
            .inner_size(OPEN_W, OPEN_H)
            .min_inner_size(MIN_W, MIN_H)
            .resizable(true)
            .center()
            // 신호등만 남기고 제목 막대는 내용과 한 판으로 — 프론트가 맨 위 띠를 끌기 영역으로 둔다.
            .title_bar_style(tauri::TitleBarStyle::Overlay)
            .hidden_title(true)
            .build();
        match built {
            Ok(w) => {
                let _ = w.set_focus();
            }
            Err(e) => {
                eprintln!("[budu] 창을 못 열었어요: {e}");
                return;
            }
        }
    }
    // 「닫아 둠」을 먼저 지우고 고정을 맞춘다 — 순서가 반대면 방금 꺼낸 창이 닫아 둔 요청으로 보여 고정이 안 걸린다.
    if let Ok(mut g) = app.state::<ApprovalState>().dismissed.lock() {
        *g = None;
    }
    sync_always_on_top(app);
}

/// 사용자가 **명시적으로** 창을 닫는다(빨간 단추·⌘W). 실제로 숨겼으면 true.
///
/// 승인 대기 중이어도 숨기되 **그 요청을 「닫아 둠」으로 적는다** — 감시 스레드가 잠든
/// 프론트를 깨우는 경로와 프론트의 raise_main_window 가 이 표식을 보고 다시 띄우지 않는다.
/// 만료 직전 한 번 되살리는 건 감시 스레드 몫(REMIND_SECS).
pub(crate) fn hide_by_user<R: Runtime>(app: &AppHandle<R>) -> bool {
    let Some(win) = app.get_webview_window(LABEL) else {
        return false;
    };
    // 숨기기 전에 적는다 — 숨긴 뒤 적으면 그 사이 1초 루프가 「깨울 요청」으로 볼 수 있다.
    if let Some(req) = crate::ipc::live_request() {
        if let Ok(mut g) = app.state::<ApprovalState>().dismissed.lock() {
            *g = Some(req.id);
        }
    }
    // 맨 위 고정을 먼저 푼다 — 숨긴 창이 고정을 단 채 남으면 다음에 꺼낼 때 다른 앱 위에 박힌다.
    let _ = win.set_always_on_top(false);
    win.hide().is_ok()
}

/// Budu 의 창 중 하나라도 포커스를 쥐고 있는가 — 「자리비움 잠금」이 창 사이 이동을 자리비움으로
/// 착각하지 않게(팝오버 → 큰 창으로 옮기는 순간 팝오버는 blur 를 받는다).
pub(crate) fn any_focused<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.webview_windows()
        .values()
        .any(|w| w.is_focused().unwrap_or(false))
}

/// 팝오버의 「창으로 열기」·메뉴의 「Budu 열기」.
#[tauri::command]
pub(crate) fn open_app_window(app: AppHandle) {
    show(&app);
}

/// 팝오버에서 「승인하러 가기」 — 대기 중인 결제 요청을 큰 창에서 띄운다.
///
/// `raise_main_window` 와 달리 「닫아 둠」을 따지지 않는다: 그 표식은 「지금은 띄우지 마」
/// 였고, 이 버튼은 사람이 「이제 띄워」라고 누른 것이다. 띄우면 `show` 가 표식을 지운다.
/// 팝오버는 치운다 — 승인 창이 그 아래에 깔리지 않게.
#[tauri::command]
pub(crate) fn show_approval(app: AppHandle) {
    if crate::ipc::live_request().is_none() {
        return;
    }
    crate::tray::hide(&app);
    show(&app);
}

#[cfg(test)]
mod tests {
    use super::*;

    // 🔴 드리프트 가드: 라벨이 capabilities 의 windows 목록에 없으면 이 창의 **모든 커맨드가
    // 권한 오류로 막힌다**(창은 뜨는데 잔액·내역이 비어 있는 상태). 한쪽만 고치는 사고를 막는다.
    #[test]
    fn label_is_in_capabilities() {
        let cap: serde_json::Value = serde_json::from_str(include_str!("../capabilities/default.json"))
            .expect("capabilities 파싱");
        let wins = cap["windows"].as_array().expect("windows 목록");
        assert!(wins.iter().any(|w| w == LABEL), "capabilities 에 {LABEL} 가 없다");
        assert!(wins.iter().any(|w| w == "main"), "팝오버 권한이 빠졌다");
        let perms = cap["permissions"].as_array().expect("permissions");
        assert!(
            perms.iter().any(|p| p == "core:window:allow-start-dragging"),
            "제목 막대를 숨겼으니 끌기 권한이 있어야 창을 옮길 수 있다"
        );
    }

    // 🔴 드리프트 가드: 승인을 받는 창이 백엔드(APPROVER)와 프론트(win.ts isApprover)에서 같아야 한다.
    // 어긋나면 모달은 큰 창에 뜨는데 감시 스레드는 팝오버를 깨우거나(또는 반대) — 결제가 아무도 안
    // 보는 창에서 5분을 흘려보낸다. 프론트 소스를 글자로 읽어 대조한다.
    #[test]
    fn approver_matches_frontend() {
        let win_ts = include_str!("../../src/lib/win.ts");
        let line = format!("export const isApprover = label() === \"{APPROVER}\";");
        assert!(win_ts.contains(&line), "src/lib/win.ts 의 isApprover 가 {APPROVER} 를 가리켜야 한다");
    }

    // ---------- 닫아 둔 승인 창 ----------

    #[test]
    fn dismissal_ignores_other_requests() {
        assert_eq!(dismissal(None, None, "b", 200), Dismissal::No);
        assert_eq!(dismissal(Some("a"), None, "b", 200), Dismissal::No);
        assert_eq!(dismissal(None, Some("b"), "b", 30), Dismissal::No);
    }

    #[test]
    fn dismissed_holds_until_deadline_nears() {
        assert_eq!(dismissal(Some("a"), None, "a", 299), Dismissal::Hold);
        assert_eq!(dismissal(Some("a"), None, "a", REMIND_SECS + 1), Dismissal::Hold);
    }

    // 경계는 리터럴로 — 상수를 상수로 검증하면 값을 0 으로 바꿔도 통과한다.
    #[test]
    fn dismissed_reminds_once_near_deadline() {
        assert_eq!(dismissal(Some("a"), None, "a", 60), Dismissal::Remind);
        assert_eq!(dismissal(Some("a"), None, "a", 1), Dismissal::Remind);
        assert_eq!(dismissal(Some("a"), Some("a"), "a", 30), Dismissal::Hold);
    }

    #[test]
    fn dismissed_expired_is_not_revived() {
        assert_eq!(dismissal(Some("a"), None, "a", 0), Dismissal::Hold);
    }
}
