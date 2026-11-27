// 메뉴바 상주 — 트레이 아이콘 + 팝오버 창 제어.
//
// 개발 7부터 **팝오버는 옵션이고 결제 승인을 받지 않는다.** 승인은 큰 창(appwin.rs)이
// 맡는다 — 팝오버는 잔액·상태·「창 열기」·대기 요청이 있으면 「승인하러 가기」 띠만. 그래서 옛 설계의
// «승인 대기 중엔 blur 로 안 숨김(hold)»·«닫아 둠»·«맨 위 고정» 은 전부 큰 창으로 옮겨 갔고,
// 여기 남은 건 보통 메뉴바 팝오버의 동작뿐이다(바깥을 누르면 숨는다).
//
// 설계 결정:
//   · 메뉴바 아이콘 자체가 옵션이다(설정 「메뉴바에 두기」, `settings.menubar`, 기본 켬). 끄면 트레이를
//     없애고 팝오버도 숨긴다 — 앱은 도크와 큰 창으로 쓴다.
//   · 팝오버 위치는 TrayIcon::rect() 로 직접 계산한다 → 위치 플러그인 의존성 0.

use crate::i18n::ts;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Runtime, WebviewWindow, Window,
};

/// 트레이 아이콘 id — rect() 조회·아이콘 교체 때 다시 찾으려고 고정한다.
const TRAY_ID: &str = "budu";

/// 팝오버와 메뉴바 사이 여백(논리 px). **0 = 메뉴바 경계에 딱 붙임**.
/// 네이티브 메뉴바 앱들(AdGuard 등)이 작업 영역 상단에 그대로 붙어 있어서, 몇 px 만 띄워도
/// 나란히 놓고 보면 혼자 처져 보인다(라이브 비교로 6 → 4 → 0 으로 좁혀 맞춘 값).
/// 그림자는 창 프레임 바깥에 그려지므로 붙여도 죽지 않는다.
const TRAY_GAP: f64 = 0.0;
/// 화면 가장자리 최소 여백 — 트레이가 오른쪽 끝일 때 팝오버가 잘리지 않게.
const SCREEN_MARGIN: f64 = 8.0;

/// 창 기본 크기(논리 px) — **tauri.conf.json 의 width/height 와 같아야 한다.**
/// 지금 창 크기를 읽지 않고 이 값을 기준으로 삼는 이유: 좁은 화면에서 한 번 줄인 뒤
/// 넓은 화면으로 옮겼을 때 원래 크기로 되돌아와야 하기 때문.
const WINDOW_W: f64 = 420.0;
const WINDOW_H: f64 = 640.0;

/// blur 로 숨긴 직후 들어오는 트레이 클릭은 "닫기"로 본다. macOS 는 트레이를 누르면 창이 먼저
/// 포커스를 잃으므로, 이 유예가 없으면 열린 팝오버를 클릭해도 닫혔다 곧바로 다시 열린다.
const REOPEN_GUARD: Duration = Duration::from_millis(250);

const ICON_NORMAL: &[u8] = include_bytes!("../icons/tray-normal.png");
const ICON_LOCKED: &[u8] = include_bytes!("../icons/tray-locked.png");

/// 팝오버 런타임 상태.
#[derive(Default)]
pub(crate) struct PopoverState {
    /// blur 로 자동으로 숨긴 시각 (REOPEN_GUARD 참고).
    last_auto_hide: Mutex<Option<Instant>>,
}

/// 팝오버를 숨긴다(⌘W·「승인하러 가기」). 팝오버는 승인을 안 받으니 «닫아 둠» 같은 기록이 없다.
pub(crate) fn hide<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.get_webview_window("main")
        .map(|w| {
            let _ = w.set_always_on_top(false); // show 가 건 고정을 숨기기 전에 푼다
            w.hide().is_ok()
        })
        .unwrap_or(false)
}

// ---------- 좌표 ----------
// 계산은 전부 **물리 픽셀**로 한다. 모니터 좌표·창 크기가 이미 물리라, 논리로 바꾸면
// "어느 모니터의 배율로 나눌 것인가" 문제가 생긴다(트레이가 있는 화면과 창이 있던 화면의
// 배율이 다를 수 있음 — 혼합 배율 멀티모니터에서 창이 엉뚱한 곳에 뜨는 원인).

/// 사각형(물리 px): x, y, 너비, 높이.
type Rect4 = (f64, f64, f64, f64);

fn tray_rect_physical(r: tauri::Rect, fallback_scale: f64) -> Rect4 {
    // macOS 의 tray-icon 은 물리 좌표를 준다. Logical 로 오는 경우만 배율을 곱하는데,
    // 이때 쓸 배율은 아직 어느 모니터인지 모르므로 주 모니터 배율로 근사한다.
    let (x, y) = match r.position {
        tauri::Position::Physical(v) => (v.x as f64, v.y as f64),
        tauri::Position::Logical(v) => (v.x * fallback_scale, v.y * fallback_scale),
    };
    let (w, h) = match r.size {
        tauri::Size::Physical(v) => (v.width as f64, v.height as f64),
        tauri::Size::Logical(v) => (v.width * fallback_scale, v.height * fallback_scale),
    };
    (x, y, w, h)
}

/// 팝오버 좌상단 위치를 계산한다 (순수 계산 — 테스트 가능). 전부 물리 px.
///
/// 가로는 트레이 아이콘 가운데, **세로는 작업 영역(work area) 상단 = 메뉴바 바로 아래**에
/// 맞춘다. 트레이 rect 의 아래쪽에 붙이면 안 된다 — 상태 아이템 프레임이 메뉴바보다 아래까지
/// 내려오는 경우가 있어 팝오버만 혼자 처져 보인다(네이티브 메뉴바 앱과 눈에 띄게 어긋남).
/// 작업 영역은 메뉴바·독을 제외한 영역이라 노치 유무·메뉴바 높이에 상관없이 정확하다.
///
/// 트레이도 작업 영역도 모르면 None — 이때는 창을 옮기지 않는다(엉뚱한 좌표로 화면 밖에
/// 두느니 있던 자리에 그대로 띄우는 게 낫다).
fn popover_origin(
    tray: Option<Rect4>,
    win_w: f64,
    win_h: f64,
    screen: Option<Rect4>,
    work: Option<Rect4>,
    margin: f64,
) -> Option<(f64, f64)> {
    let gap = TRAY_GAP * margin;
    let pad = SCREEN_MARGIN * margin;

    let (x, y) = match (tray, work) {
        // 가로 = 아이콘 중앙, 세로 = 메뉴바 바로 아래.
        (Some((tx, _, tw, _)), Some((_, wy, _, _))) => (tx + tw / 2.0 - win_w / 2.0, wy + gap),
        // 작업 영역을 모르면 트레이 아래로(차선책).
        (Some((tx, ty, tw, th)), None) => (tx + tw / 2.0 - win_w / 2.0, ty + th + gap),
        // 트레이를 모르면 작업 영역 오른쪽 위(트레이가 사는 자리).
        (None, Some((wx, wy, ww, _))) => (wx + ww - win_w - pad, wy + gap),
        (None, None) => return None,
    };

    // 가로는 **모니터 전체**로 자른다. 작업 영역으로 자르면 독이 좌우에 있을 때 그 폭만큼
    // 팝오버가 아이콘에서 밀려나 시각적 연결이 끊긴다(팝오버가 독 위에 겹치는 건 정상).
    let x = match screen.or(work) {
        Some((sx, _, sw, _)) => {
            let min_x = sx + pad;
            x.clamp(min_x, (sx + sw - win_w - pad).max(min_x))
        }
        None => x,
    };

    // 세로는 작업 영역으로 — 메뉴바를 침범하지도, 독 아래로 내려가지도 않게.
    let y = match work {
        Some((_, wy, _, wh)) => {
            // 하한은 메뉴바 바로 아래(gap 없이).
            let min_y = wy;
            y.clamp(min_y, (wy + wh - win_h - pad).max(min_y))
        }
        None => y,
    };

    Some((x, y))
}

/// 작업 영역에 들어가도록 창 높이를 정한다(물리 px). 순수 계산 — 테스트 가능.
///
/// 좌표만으로는 작업 영역보다 큰 고정 창을 다 보여줄 수 없다. 아래가 잘리면 팝오버 하단의
/// **승인/거부 버튼에 손이 닿지 않아 결제가 그대로 타임아웃**된다. 셸이 이미 내부 스크롤이라
/// 높이를 줄여도 내용은 다 볼 수 있으므로, 화면에 맞춰 줄이는 쪽이 옳다.
///
/// 최소 높이 하한을 두지 않는다. 하한을 두면 작업 영역이 그보다 작을 때 다시 아래가 잘려
/// 이 함수의 목적 자체가 무너진다 — "읽기 좋은 크기"보다 "승인 버튼에 닿는다"가 우선이다.
/// (실제로 작업 영역이 그 정도로 작은 Mac 은 없지만, 보장은 예외 없이 성립해야 한다.)
fn fit_height(desired_h: f64, work_h: Option<f64>, margin: f64) -> f64 {
    let Some(wh) = work_h else {
        return desired_h;
    };
    let room = wh - SCREEN_MARGIN * margin * 2.0;
    // 값이 비정상(0 이하)이면 손대지 않는다.
    if room <= 0.0 {
        return desired_h;
    }
    desired_h.min(room)
}

/// 이번에 적용할 창 크기 (순수 계산 — 물리 px).
///
/// `grow` = 기본 크기로 되돌려도 되는가. 숨어 있다 뜨는 창(show)은 사용자가 보기 전에
/// 끝나므로 되돌려도 된다. **이미 보이는 창**은 다르다 — 승인 모달에 비번을 입력하는 중일
/// 수 있어, 굳이 키우면 버튼이 손 아래에서 움직인다. 그래서 그때는 화면에 안 들어가는
/// 경우(줄여야 하는 경우)에만 손대고 그 외엔 지금 크기를 그대로 둔다.
/// "키우지 않는다"는 **축마다** 성립해야 한다 — 높이만 제한하면, 2x 화면에서 1x 화면으로
/// 불러온 창이 옛 배율 너비(840px)를 그대로 달고 있게 된다.
fn target_size(cur: Option<(f64, f64)>, want: (f64, f64), grow: bool) -> (f64, f64) {
    match cur {
        Some((cw, ch)) if !grow => (cw.min(want.0), ch.min(want.1)),
        _ => want,
    }
}

/// 점을 품는 모니터를 찾는다 (물리 좌표). 트레이가 있는 화면을 기준으로 잘라야
/// 창이 트레이와 다른 화면으로 튀지 않는다.
fn monitor_containing<R: Runtime>(app: &AppHandle<R>, x: f64, y: f64) -> Option<tauri::Monitor> {
    app.available_monitors().ok()?.into_iter().find(|m| {
        let p = m.position();
        let s = m.size();
        x >= p.x as f64
            && x < p.x as f64 + s.width as f64
            && y >= p.y as f64
            && y < p.y as f64 + s.height as f64
    })
}

/// 모니터 전체 경계 — 가로 클램프와 트레이가 어느 화면인지 찾는 데 쓴다.
fn monitor_rect(m: &tauri::Monitor) -> Rect4 {
    let p = m.position();
    let s = m.size();
    (p.x as f64, p.y as f64, s.width as f64, s.height as f64)
}

/// 작업 영역(메뉴바·독 제외). macOS 의 visibleFrame — 팝오버를 붙일 기준이자 세로 클램프.
fn work_rect(m: &tauri::Monitor) -> Rect4 {
    let w = m.work_area();
    (
        w.position.x as f64,
        w.position.y as f64,
        w.size.width as f64,
        w.size.height as f64,
    )
}

/// 팝오버를 트레이 아이콘 바로 아래로 옮긴다(필요하면 크기도 맞춘다).
/// `grow` 의 의미는 target_size 주석 참고 — 보이는 창을 불러올 때는 false.
fn position_at_tray<R: Runtime>(app: &AppHandle<R>, win: &WebviewWindow<R>, grow: bool) {
    let primary = app.primary_monitor().ok().flatten();
    let primary_scale = primary.as_ref().map(|m| m.scale_factor()).unwrap_or(1.0);

    let tray = app
        .tray_by_id(TRAY_ID)
        .and_then(|t| t.rect().ok().flatten())
        .map(|r| tray_rect_physical(r, primary_scale));

    // 기준 화면 = 트레이가 놓인 화면(트레이는 메뉴바 안에 있으므로 '작업 영역'이 아니라
    // 모니터 전체 경계로 찾아야 한다). 트레이를 모르면 주 모니터(메뉴바가 있는 곳).
    let screen_monitor = tray
        .and_then(|(x, y, w, h)| monitor_containing(app, x + w / 2.0, y + h / 2.0))
        .or(primary);
    let scale = screen_monitor
        .as_ref()
        .map(|m| m.scale_factor())
        .unwrap_or(primary_scale);
    let work = screen_monitor.as_ref().map(work_rect);
    let screen = screen_monitor.as_ref().map(monitor_rect);

    // 크기·여백 상수는 논리 px 기준이라 대상 화면 배율만큼 키워 쓴다.
    // 낮은 화면이면 먼저 줄여서 아래가 잘리지 않게 한다(셸이 내부 스크롤이라 내용은 다 보인다).
    // 넓은 화면으로 옮기면 다시 기본 크기로 돌아온다 — 그래서 지금 크기가 아니라 상수 기준.
    let want = (
        WINDOW_W * scale,
        fit_height(WINDOW_H * scale, work.map(|(_, _, _, wh)| wh), scale),
    );
    let cur = win
        .outer_size()
        .ok()
        .map(|s| (s.width as f64, s.height as f64));
    let (win_w, win_h) = target_size(cur, want, grow);

    // 너비까지 함께 비교해야 한다 — 높이만 보면, 배율이 다른 화면으로 옮겨 목표 높이가
    // 우연히 같아진 경우(예: 2x 에서 줄어든 높이 == 1x 기본 높이) 호출을 건너뛰어
    // 너비가 옛 배율 그대로 남는다. 그러면 위치 계산(win_w 전제)과 실제 창이 어긋난다.
    let needs_resize = match cur {
        Some((cw, ch)) => (cw - win_w).abs() > 0.5 || (ch - win_h).abs() > 0.5,
        None => true,
    };
    if needs_resize {
        let _ = win.set_size(tauri::PhysicalSize::new(win_w, win_h));
    }

    if let Some((x, y)) = popover_origin(tray, win_w, win_h, screen, work, scale) {
        let _ = win.set_position(tauri::PhysicalPosition::new(x, y));
    }
}

/// 팝오버를 트레이 아래에 띄우고 포커스를 준다.
pub(crate) fn show<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };
    // 크기를 기본값으로 되돌려도 되는 건 **지금 사용자 눈에 없는 창**일 때뿐이다. show 는 이미
    // 보이는 창에도 불린다(트레이 메뉴 '열기' 등) — 그때
    // 창이 커지면 승인 버튼이 손 아래에서 움직인다.
    // · 최소화된 창은 macOS 에서 visible 로 잡히지만 화면엔 없다 → 되돌려도 안전(안 그러면
    //   줄어든 크기가 숨겼다 열기 전까지 고착된다).
    // · 조회 실패는 "보이는 중"으로 본다 — 틀렸을 때 손해가 작은 쪽(안 키움).
    let hidden = !win.is_visible().unwrap_or(true) || win.is_minimized().unwrap_or(false);
    position_at_tray(app, &win, hidden);
    let _ = win.unminimize();
    // 승인 대기 중인 큰 창은 «항상 위»(floating 레벨)라, 보통 레벨의 팝오버가 그 밑에 깔린다.
    // 팝오버도 같은 레벨에 두고 앞으로 올린다 — 포커스를 잃으면 어차피 숨으니 남는 부작용이 없다.
    let _ = win.set_always_on_top(true);
    let _ = win.show();
    let _ = win.set_focus();
}

/// 트레이 아이콘 좌클릭 — 열려 있으면 닫고, 닫혀 있으면 연다.
fn toggle<R: Runtime>(app: &AppHandle<R>) {
    let Some(win) = app.get_webview_window("main") else {
        return;
    };

    // 방금 blur 로 스스로 숨었다면, 그 blur 를 일으킨 게 바로 이 클릭이다 → 다시 열지 않는다.
    if let Ok(mut g) = app.state::<PopoverState>().last_auto_hide.lock() {
        if g.map(|t| t.elapsed() < REOPEN_GUARD).unwrap_or(false) {
            *g = None;
            return;
        }
        *g = None;
    }

    if win.is_visible().unwrap_or(false) {
        // 보이지만 뒤에 있으면 닫지 말고 앞으로.
        if win.is_focused().unwrap_or(false) {
            hide(app);
        } else {
            // 숨김 없이 앞으로만 올리는 경로에서도 자리를 다시 잡는다
            // (hide_unless_held 의 hold 분기와 같은 이유 — 보이는 동안은 재배치가 없다).
            position_at_tray(app, &win, false);
            let _ = win.set_focus();
        }
    } else {
        show(app);
    }
}

/// 창이 포커스를 잃었을 때 — 팝오버를 숨긴다(lib.rs 의 on_window_event 에서 호출).
/// 승인은 큰 창이 받으니 대기 요청이 있어도 붙잡아 두지 않는다 — 보통 메뉴바 팝오버처럼.
pub(crate) fn on_blur<R: Runtime>(win: &Window<R>) {
    let app = win.app_handle();
    let _ = win.set_always_on_top(false); // show 가 건 고정 — 숨는 창에 남기지 않는다
    // 실제로 숨었을 때만 재열림 가드를 건다 — hide 가 실패했는데 가드가 걸리면
    // 뒤이은 트레이 클릭이 통째로 무시돼 토글이 한 번 안 먹는다.
    if win.hide().is_ok() {
        if let Ok(mut g) = app.state::<PopoverState>().last_auto_hide.lock() {
            *g = Some(Instant::now());
        }
    }
}

/// 긴급 잠금 상태를 메뉴바 아이콘에 반영한다 (평상시 = 열쇠구멍 동전 / 잠김 = 자물쇠).
pub(crate) fn refresh_icon<R: Runtime>(app: &AppHandle<R>) {
    let locked = crate::lock::read_lock();
    let bytes = if locked { ICON_LOCKED } else { ICON_NORMAL };
    let Ok(icon) = Image::from_bytes(bytes) else {
        return;
    };
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        // 아이콘과 템플릿 지정을 한 번에 — 따로 하면 macOS 에서 한 프레임 깜빡인다.
        let _ = tray.set_icon_with_as_template(Some(icon), true);
        let _ = tray.set_tooltip(Some(if locked {
            ts!("Budu — 긴급 잠금", "Budu — emergency lock")
        } else {
            "Budu"
        }));
    }
}

/// 우클릭 메뉴 — 언어가 바뀌면 통째로 다시 만든다(항목 핸들을 들고 있지 않아도 되게).
/// 메뉴 이벤트 핸들러는 트레이 쪽에 붙어 있고 id 는 그대로라, 바꿔 껴도 동작이 유지된다.
fn menu<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<Menu<R>> {
    // 「Budu 열기」 = 큰 창. 팝오버는 아이콘 좌클릭.
    let open_i = MenuItem::with_id(
        app,
        "open",
        ts!("Budu 열기", "Open Budu"),
        true,
        None::<&str>,
    )?;
    let quit_i = MenuItem::with_id(app, "quit", ts!("종료", "Quit"), true, None::<&str>)?;
    Menu::with_items(app, &[&open_i, &quit_i])
}

/// 언어를 바꾼 뒤 메뉴바의 글자를 새 언어로 갈아 끼운다.
/// 실패해도 조용히 넘어간다 — 메뉴 글자 하나 때문에 언어 변경 자체를 실패로 돌릴 이유가 없다.
pub(crate) fn retitle<R: Runtime>(app: &AppHandle<R>) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(m) = menu(app) {
            let _ = tray.set_menu(Some(m));
        }
    }
    refresh_icon(app); // 툴팁도 언어를 탄다
}

/// 메뉴바 아이콘을 만든다. 좌클릭 = 팝오버 토글, 우클릭 = 메뉴.
pub(crate) fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    let menu = menu(app)?;

    TrayIconBuilder::with_id(TRAY_ID)
        .icon(Image::from_bytes(ICON_NORMAL)?)
        .icon_as_template(true)
        .tooltip("Budu")
        .menu(&menu)
        // 좌클릭은 팝오버 토글로 쓰므로 메뉴는 우클릭에만.
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "open" => crate::appwin::show(app),
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                toggle(tray.app_handle());
            }
        })
        .build(app)?;

    // 앱을 껐다 켜도 잠금 상태가 아이콘에 남아 있어야 한다.
    refresh_icon(app);
    Ok(())
}

/// 메뉴바 아이콘을 켜고 끈다(설정 「메뉴바에 두기」). 이미 그 상태면 아무것도 안 한다.
/// 끄면 팝오버도 숨긴다 — 아이콘 없이 떠 있는 팝오버는 닫을 길이 바깥 클릭뿐이다.
pub(crate) fn set_visible<R: Runtime>(app: &AppHandle<R>, on: bool) -> tauri::Result<()> {
    let has = app.tray_by_id(TRAY_ID).is_some();
    if on && !has {
        build(app)?;
    } else if !on && has {
        hide(app);
        let _ = app.remove_tray_by_id(TRAY_ID);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
