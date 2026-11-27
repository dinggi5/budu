use super::*;

const W: f64 = WINDOW_W;
const H: f64 = WINDOW_H;
/// 메뉴바 24 를 뺀 1x 작업 영역 (독 없음).
const WORK: Option<Rect4> = Some((0.0, 24.0, 1440.0, 876.0));
/// 같은 화면의 모니터 전체 경계.
const SCREEN: Option<Rect4> = Some((0.0, 0.0, 1440.0, 900.0));

fn origin(tray: Option<Rect4>) -> (f64, f64) {
    popover_origin(tray, W, H, SCREEN, WORK, 1.0).expect("작업 영역을 아는 한 위치가 나온다")
}

// 가로는 트레이 아이콘 가운데, 세로는 메뉴바 바로 아래에 "딱 붙는다".
// 기댓값을 TRAY_GAP 이 아니라 리터럴로 둔다 — 상수를 상수로 검증하면 값을 4·6 으로
// 되돌려도 통과해서 "붙인다"는 요구사항을 전혀 못 지킨다.
#[test]
fn sits_flush_under_menubar() {
    let (x, y) = origin(Some((700.0, 0.0, 24.0, 24.0)));
    assert_eq!(x, 700.0 + 12.0 - W / 2.0);
    assert_eq!(y, 24.0, "메뉴바 경계에 딱 붙어야 한다(간격 0)");
}

// 🔴 회귀: 상태 아이템 프레임이 메뉴바보다 아래까지 내려와도(높이 40) 팝오버는
// 메뉴바 바로 아래에 붙는다. 트레이 rect 아래에 붙이던 옛 방식은 여기서 처졌다.
#[test]
fn anchors_to_menubar_not_tray_bottom() {
    let (_, y) = origin(Some((700.0, 0.0, 24.0, 40.0)));
    assert_eq!(y, 24.0);
}

// 트레이가 화면 오른쪽 끝이어도 화면 밖으로 나가지 않는다.
#[test]
fn clamps_to_right_edge() {
    let (x, _) = origin(Some((1420.0, 0.0, 20.0, 24.0)));
    assert_eq!(x, 1440.0 - W - SCREEN_MARGIN);
}

// 왼쪽 끝도 마찬가지.
#[test]
fn clamps_to_left_edge() {
    let (x, _) = origin(Some((0.0, 0.0, 20.0, 24.0)));
    assert_eq!(x, SCREEN_MARGIN);
}

// 독이 우측에 있어도 가로는 모니터 전체 기준으로 자른다 — 작업 영역으로 자르면
// 독 폭만큼 팝오버가 아이콘에서 밀려나 시각적 연결이 끊긴다.
#[test]
fn side_dock_does_not_pull_popover_off_icon() {
    // 우측 100 이 독. 작업영역 기준 최대 x = 1340-420-8 = 912,
    // 모니터 기준 최대 x = 1440-420-8 = 1012. 그 사이(950)에 놓이는 트레이를 고른다 —
    // 옛 정책(작업영역으로 가로 클램프)이었다면 912 로 끌려갔을 자리.
    let work_with_side_dock = Some((0.0, 24.0, 1340.0, 876.0));
    let (x, _) = popover_origin(
        Some((1148.0, 0.0, 24.0, 24.0)),
        W,
        H,
        SCREEN,
        work_with_side_dock,
        1.0,
    )
    .unwrap();
    assert_eq!(x, 950.0, "아이콘 중앙(950)을 유지해야 한다");
    assert!(
        x > 1340.0 - W - SCREEN_MARGIN,
        "작업영역 클램프였다면 912 로 밀렸다"
    );
}

// 트레이 위치를 못 얻으면 메뉴바가 있는 오른쪽 위로.
#[test]
fn falls_back_to_top_right() {
    let (x, y) = origin(None);
    assert_eq!(x, 1440.0 - W - SCREEN_MARGIN);
    assert_eq!(y, 24.0);
}

// 창 아래가 독 위에 머문다(세로는 작업 영역 기준).
#[test]
fn clamps_above_dock() {
    let (_, y) = origin(Some((700.0, 0.0, 24.0, 24.0)));
    assert!(y + H <= 24.0 + 876.0, "창 아래가 작업 영역 안이어야 한다");
}

// 작업 영역을 모르면 차선책으로 트레이 아래에 붙인다.
#[test]
fn no_work_area_falls_back_to_tray_bottom() {
    let (x, y) = popover_origin(Some((700.0, 0.0, 24.0, 24.0)), W, H, None, None, 1.0).unwrap();
    assert_eq!(x, 700.0 + 12.0 - W / 2.0);
    assert_eq!(y, 24.0 + TRAY_GAP);
}

// 트레이도 화면도 모르면 아예 옮기지 않는다 — 옛 코드는 여기서 f64::MAX 로 창을 던졌다.
#[test]
fn unknown_tray_and_screen_does_not_move() {
    assert!(popover_origin(None, W, H, None, None, 1.0).is_none());
}

// 레티나: 창 크기·여백이 배율만큼 커진 물리값으로 들어와야 가운데 정렬과 클램프가 맞는다.
#[test]
fn scales_with_display() {
    let screen = Some((0.0, 0.0, 2880.0, 1800.0));
    let work = Some((0.0, 48.0, 2880.0, 1752.0));
    let (x, y) = popover_origin(
        Some((1400.0, 0.0, 48.0, 48.0)),
        W * 2.0,
        H * 2.0,
        screen,
        work,
        2.0,
    )
    .unwrap();
    assert_eq!(y, 48.0, "레티나에서도 메뉴바에 딱 붙는다");
    assert_eq!(x, 1400.0 + 24.0 - W); // 트레이 중심 - 창 절반(= W*2/2)
}

// ---------- 창 높이 맞추기 ----------

// 넉넉한 화면에서는 기본 높이 그대로.
#[test]
fn keeps_default_height_when_it_fits() {
    assert_eq!(fit_height(H, Some(876.0), 1.0), H);
}

// 🔴 작업 영역이 창보다 낮으면 **줄인다** — 안 줄이면 팝오버 하단의 승인 버튼이
// 화면 밖에 남아 결제가 그대로 타임아웃된다.
#[test]
fn shrinks_to_fit_short_work_area() {
    let h = fit_height(H, Some(500.0), 1.0);
    assert_eq!(h, 500.0 - SCREEN_MARGIN * 2.0);
    assert!(h < H && h + SCREEN_MARGIN * 2.0 <= 500.0);
}

// 🔴 핵심 보장: **여백을 뺀 가용 높이가 남는 작업 영역이면** 창 전체가 그 안에 들어간다.
// (fit_height + popover_origin 을 함께 검증 — 최소 높이 하한을 두면 여기가 깨진다.
//  하한이 있던 버전은 작업 영역 300 에서 창 320 을 반환해 아래 20px 가 잘렸다.)
// 가용 높이가 0 이하인 비정상 입력은 이 보장 밖 — absurd_work_area_leaves_size_alone 참고.
#[test]
fn fits_inside_any_usable_work_area() {
    for wh in [
        SCREEN_MARGIN * 2.0 + 1.0,
        300.0_f64,
        500.0,
        660.0,
        876.0,
        1200.0,
    ] {
        let work = Some((0.0, 24.0, 1440.0, wh));
        let h = fit_height(H, Some(wh), 1.0);
        let (_, y) =
            popover_origin(Some((700.0, 0.0, 24.0, 24.0)), W, h, SCREEN, work, 1.0).unwrap();
        assert!(y >= 24.0, "메뉴바를 침범하면 안 된다 (wh={wh})");
        assert!(
            y + h <= 24.0 + wh,
            "창 아래가 작업 영역을 넘으면 안 된다 (wh={wh})"
        );
    }
}

// 작업 영역을 모르면 기본 높이를 그대로 쓴다.
#[test]
fn unknown_work_area_keeps_desired_height() {
    assert_eq!(fit_height(H, None, 1.0), H);
}

// 레티나에서도 물리 기준으로 맞춘다(여백에 배율 적용).
#[test]
fn fits_height_on_retina() {
    assert_eq!(
        fit_height(H * 2.0, Some(1000.0), 2.0),
        1000.0 - SCREEN_MARGIN * 2.0 * 2.0
    );
    // 레티나에서도 "항상 들어간다" 보장이 성립한다.
    let work = Some((0.0, 48.0, 2880.0, 1000.0));
    let h = fit_height(H * 2.0, Some(1000.0), 2.0);
    let (_, y) =
        popover_origin(Some((1400.0, 0.0, 48.0, 48.0)), W * 2.0, h, None, work, 2.0).unwrap();
    assert!(y >= 48.0 && y + h <= 48.0 + 1000.0);
}

// 작업 영역 값이 비정상(가용 높이 0 이하)이면 손대지 않는다 — 0 이하 크기로 창을
// 만들지 않으려는 방어. 이 경우엔 "항상 들어간다" 보장이 성립하지 않는다(의도된 예외:
// 실제 macOS 작업 영역이 여백 이하로 내려오는 경로는 없다).
#[test]
fn absurd_work_area_leaves_size_alone() {
    assert_eq!(fit_height(H, Some(SCREEN_MARGIN * 2.0), 1.0), H);
    assert_eq!(fit_height(H, Some(0.0), 1.0), H);
}

// 숨어 있다 뜨는 창은 기본 크기로 복원한다(사용자가 보기 전에 끝난다).
#[test]
fn hidden_window_restores_design_size() {
    assert_eq!(target_size(Some((300.0, 500.0)), (W, H), true), (W, H));
    assert_eq!(target_size(None, (W, H), true), (W, H));
}

// 🔴 보이는 창은 키우지 않는다 — 승인 모달에 비번을 넣는 중에 창이 커지면 버튼이
// 손 아래에서 움직인다. 크기를 모를 때만 목표값으로 간다.
#[test]
fn visible_window_is_never_grown() {
    assert_eq!(target_size(Some((W, 500.0)), (W, H), false), (W, 500.0));
    assert_eq!(target_size(None, (W, H), false), (W, H));
    // 너비도 마찬가지 — 축마다 성립해야 한다(좁은 창을 넓히지 않는다).
    assert_eq!(target_size(Some((300.0, H)), (W, H), false), (300.0, H));
}

// 🔴 반대 방향: 2x 화면에서 1x 화면으로 불러온 창은 옛 배율 너비를 달고 있다.
// "키우지 않는다"를 높이에만 적용하면 그 과대 너비가 그대로 남는다.
#[test]
fn visible_window_shrinks_stale_scaled_width() {
    assert_eq!(target_size(Some((W * 2.0, H * 2.0)), (W, H), false), (W, H));
}

// 다만 화면에 안 들어가면(줄여야 하면) 보이는 창도 줄인다 — 승인 버튼 접근성이 우선.
#[test]
fn visible_window_still_shrinks_when_too_tall() {
    let want = (W, 400.0); // 낮은 화면이라 목표 높이가 400
    assert_eq!(target_size(Some((W, H)), want, false), (W, 400.0));
}

// 🔴 드리프트 가드: WINDOW_W/H 는 "tauri.conf.json 과 같아야 한다"가 주석으로만 있었다.
// 어긋나면 좁은 화면에서 줄인 창이 **원래 크기가 아닌 값으로 복원**된다(설계상 지금 창
// 크기가 아니라 이 상수로 되돌리기 때문). 두 곳 중 한쪽만 고치는 사고를 컴파일·테스트로 막는다.
#[test]
fn window_size_matches_tauri_conf() {
    let conf: serde_json::Value =
        serde_json::from_str(include_str!("../../tauri.conf.json")).expect("tauri.conf.json 파싱");
    let win = conf["app"]["windows"]
        .as_array()
        .and_then(|ws| ws.iter().find(|w| w["label"] == "main"))
        .expect("main 창 설정이 있어야 한다");
    assert_eq!(win["width"].as_f64(), Some(WINDOW_W));
    assert_eq!(win["height"].as_f64(), Some(WINDOW_H));
}
