// 생존 감시 스레드 — 하트비트·잠든 웹뷰 깨우기·죽은 요청 치우기·창 전 한도 거절.
// ipc.rs 에서 떼어 낸 것(동작 그대로). 요청 파일·승인 중재는 부모 모듈(ipc.rs)에 있다.

use super::*;

// ---------- 생존 감시 ----------
//
// 개발 48 실측: 창을 안 건드리면 **약 5분 뒤 프론트 폴링이 완전히 멎는다**(WebView 타이머
// 억제). 하트비트가 그 폴링에만 매달려 있어 같이 멎었고, MCP 는 그 뒤 도착한 결제를
// 「지갑 앱이 실행 중이 아니에요」로 거절했다. 트레이 상주 앱이라 **창이 숨어 있는 게 평상시**고,
// "AI 와 대화하다 결제가 필요해진" 순간은 대개 지갑을 5분 넘게 안 건드린 뒤다 → 드문 일이 아니다.
//
// 고침은 **두 개가 한 쌍**이다. ① 하트비트를 러스트가 찍는다(프로세스가 살아 있다는 건 러스트만
// 정직하게 안다). ②만 빼고 ①만 하면 **개악**이다 — 정직한 즉시 거절이 "요청은 받았는데 창은
// 안 뜸"이라는 5분짜리 침묵으로 바뀐다. 그래서 ② **요청이 왔는데 프론트가 자고 있으면 러스트가
// 창을 깨운다**. 창이 뜨면 WebView 가 되살아나 기존 경로(자율 승인 시도 → 실패 시 모달)가
// 그대로 이어진다.

/// 하트비트를 다시 쓰는 주기(초). MCP 신선도 기준(payment.rs ALIVE_SECS = 10초)의 5배 여유.
pub(super) const HEARTBEAT_SECS: u64 = 2;

/// 프론트 폴링이 이만큼 끊기면 "WebView 가 잠들었다"로 본다. 폴링 주기가 1초라 3초면
/// 두 번을 내리 놓친 뒤다 — 한 박자 밀린 것과 구별된다.
pub(super) const FRONT_STALE_SECS: u64 = 3;

/// 잠든 프론트를 깨우려고 창을 다시 띄우는 최소 간격(초). 한 번의 show 로 안 깨어난
/// 경우에만 다시 시도한다(깨어나면 폴링이 돌아와 아래 조건에서 걸러진다).
pub(super) const WAKE_RETRY_SECS: u64 = 15;

/// 창을 이만큼 깨워 봐도 폴링이 안 돌아오면 **WebView 가 죽었다**고 본다.
/// `WAKE_RETRY_SECS` 간격이므로 3회 = 약 45초.
///
/// **창을 띄우는 것 자체가 확인 절차다** — 살아만 있으면 숨어 있던 WebView 도 창이 뜨는 순간
/// 타이머가 되살아난다. 개발 51 실측(디버그 번들 + 가짜 HOME):
/// · 창을 한 번도 안 띄운 앱은 **약 7초 뒤 폴링이 멎는다**(그 뒤 400초 넘게 0회) — 즉 결제가
///   올 때 프론트가 «자고 있는» 건 예외가 아니라 **평상시**다.
/// · 그 상태에 요청이 오면 깨움 **1회로 폴링이 5초 안에 돌아오고**(1회/초 재개) 이 카운터는
///   0으로 리셋된다 → **정상 앱을 죽었다고 오판하지 않는다**(실측: ui_ok 계속 true, 요청은
///   사람을 기다린 채 유지).
/// · 반대로 렌더러가 멈춘(hang) 앱은 깨움 3회를 채워 **36초 만에** 정직한 실패로 끝났다.
/// · WebContent 프로세스를 SIGKILL 하면 **WebKit 이 스스로 되살린다**(폴링 계속) → 오탐 없음.
pub(super) const DEAD_WAKES: u32 = 3;

/// 프론트(WebView)가 마지막으로 폴링한 시각. 0 = 아직 한 번도.
pub(super) static LAST_POLL: AtomicU64 = AtomicU64::new(0);

/// **아무도 안 기다리는 요청인가** — `is_stale` 의 두 갈래 중 「늙었다」만 본다.
///
/// `is_stale` 은 미래 시각(시계가 크게 뒤로 밀린 경우)도 만료로 치는데, 그건 「보여주지
/// 말라」는 뜻이지 「기다리는 쪽이 없다」는 뜻이 아니다 — 그 요청은 **방금 만들어져 MCP 가
/// 여전히 5분을 기다리는 중**일 수 있다. 지우면 single-flight 슬롯이 열려 다음 요청 B 가
/// 들어오고, A 쪽은 시간 초과를 받아 재시도하게 된다. 그래서 지우기 판정은
/// 「늙어서 버려진 것」만 본다. 시계가 영구히 뒤로 밀린 채면 이 파일은 안 지워지지만,
/// 그건 개발 63 이전과 같은 상태이고 화면엔 어차피 안 보인다.
pub(super) fn is_abandoned(req: &PaymentRequest) -> bool {
    let now = now_secs();
    if req.created > now.saturating_add(STALE_GRACE_SECS) {
        return false; // 미래 시각 — 늙은 게 아니다.
    }
    now.saturating_sub(req.created) > APPROVAL_WINDOW_SECS + STALE_GRACE_SECS
}

/// 이 요청 파일을 치워야 하는가 (순수 — 테스트용).
///
/// 조건 둘을 **모두** 만족할 때만: ① 승인 창 시간(5분)에 유예 60초까지 지났다 = 기다리는
/// 쪽이 이미 없다(`is_abandoned`) ② 지금 진행 중인 승인이 없다 — 진행 중이면 그쪽이
/// `resolve_request` 로 결과를 쓸 때 자기 요청이 아직 있는지 보므로, 먼저 지우면
/// **성공한 결제의 결과가 버려진다**.
/// ①′ 또는 임대가 오래 끊겼다(`lease_dead` — 기다리던 MCP 가 죽었다).
pub(super) fn should_clear_request(req: Option<&PaymentRequest>, in_flight: bool, lease_dead: bool) -> bool {
    match req {
        Some(r) => !in_flight && (is_abandoned(r) || lease_dead),
        None => false,
    }
}

static LAST_CLEAR_TICK: AtomicU64 = AtomicU64::new(0);
static AWAKE_SINCE: AtomicU64 = AtomicU64::new(0);

/// 감시 스레드가 끊김 없이 10초 넘게 돌았나 — 앱 시작 직후·잠자기에서 깬 직후는 false.
pub(super) fn settled() -> bool {
    let now = now_secs();
    let last = LAST_CLEAR_TICK.swap(now, Ordering::Relaxed);
    if now.saturating_sub(last) > 10 {
        AWAKE_SINCE.store(now, Ordering::Relaxed);
    }
    now.saturating_sub(AWAKE_SINCE.load(Ordering::Relaxed)) >= 10
}

/// 죽은 요청 파일을 치운다 — 판정과 삭제를 **한 잠금 안에서** 한다(승인 시작과 순서를 다투지
/// 않게). 잠금을 못 잡으면 아무것도 안 한다: 이건 청소이지 안전 장치가 아니라, 다음 초에 다시 온다.
pub(super) fn clear_dead_request() {
    let Ok(_lock) = APPROVAL_ARBITER.lock() else {
        return;
    };
    let req = read_request();
    // 있는데 못 읽는 요청 파일(옛 MCP 가 쓰다 죽은 반쪽 등)도 늙었으면 옆으로 치운다 — 안 치우면
    // MCP 는 「대기 중」이라 새 요청을 영영 거절한다. 승인 창 시간 + 유예가 지나야만: 그 전엔 아직 쓰는 중일 수 있다.
    if req.is_none() {
        if let Ok(p) = request_path() {
            set_aside_if_stale_unreadable(&p);
        }
    }
    // 잠자기에서 막 깼으면(감시 스레드 틱 사이가 10초 넘게 벌어짐) MCP 도 방금 깨어 아직 임대를 못 갱신했다 —
    // 그 순간의 «2분 끊김» 은 죽음이 아니다. 깬 뒤 10초가 지나서도 끊겨 있을 때만 치운다.
    let dead = settled() && req.as_ref().is_some_and(lease_dead);
    if should_clear_request(req.as_ref(), approval_in_flight(), dead) {
        if let (Ok(p), Some(r)) = (request_path(), req) {
            // 본 그 요청만 — 판정한 뒤 MCP 가 새 요청을 썼으면 그건 살아 있는 요청이다.
            crate::policy::remove_request_if_mine(&p, &r.id);
        }
    }
}

/// 못 읽는 요청 파일이 승인 창 시간 + 유예보다 늙었으면 `<이름>.broken.<초>` 로 치운다. 읽히는 파일·없는 파일은 그대로.
pub(super) fn set_aside_if_stale_unreadable(path: &std::path::Path) {
    // 바이트로 읽는다 — `read_to_string` 은 깨진 UTF-8 에서 실패해 그런 파일을 영영 못 치웠다.
    let Ok(bytes) = fs::read(path) else {
        return;
    };
    if serde_json::from_slice::<PaymentRequest>(&bytes).is_ok() {
        return;
    }
    let old = fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age.as_secs() > APPROVAL_WINDOW_SECS + STALE_GRACE_SECS);
    if old {
        let name = path
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let _ = fs::rename(path, path.with_file_name(format!("{name}.broken.{}", now_secs())));
    }
}

/// 하트비트를 찍고, 프론트가 자는 동안 도착한 결제 요청에 창을 깨우는 상주 스레드.
///
/// **프론트가 깨어 있으면 아무것도 하지 않는다**(하트비트만 계속 찍는다). 이게 중요하다 —
/// 요청이 오자마자 창을 띄우면 자율 결제(창 없이 조용히 끝나야 하는 경로)까지 창이 튀어나온다.
/// 지금 동작은 "프론트가 살아 있으면 예전 그대로, 자고 있을 때만 러스트가 대신 깨운다"다.
///
/// 창 조작은 AppKit 을 건드리므로 반드시 메인 스레드에서 한다(트레이 rect 조회 포함).
pub(super) fn watchdog(app: tauri::AppHandle) {
    let mut st = WatchState::default();
    // 결제 시도 기록 청소 마지막 시각. 0 = 켜자마자 한 번.
    let mut last_prune = 0u64;
    loop {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let now = now_secs();
        if now.saturating_sub(last_prune) >= 3600 {
            last_prune = now;
            prune_attempts();
        }
        // 🔴 **죽은 요청 파일을 치운다 — 루프의 맨 앞에서**.
        //
        // 여기 말고는 요청 파일을 지우는 곳이 `resolve_request` 하나뿐이다 — `is_stale` 은
        // 보여줄지 말지를 **거르기만** 하고 파일은 남긴다. 그래서 MCP 가 승인 대기 중
        // SIGKILL 로 죽거나(`CancelOnDrop` 이 못 도는 경우) `budu pay` 를 Ctrl-C 로 끊으면
        // 고아 파일이 남고, MCP 의 `has_pending()` 은 **존재만** 보므로 그 뒤 모든 결제가
        // 「이미 승인 대기 중인 결제가 있어요」로 막힌다 — 영구히, 복구는 터미널에서 JSON
        // 파일 지우기. 사용자는 그 파일이 있는 줄도 모른다.
        //
        // 🔴 **아래 「프론트가 살아 있으면 멈춤」보다 먼저 있어야 한다** (개발 63 2차
        // 리뷰). 뒤에 두면 청소가 **창이 잠들었을 때만** 도는데, 결제가 안 된다고 느낀
        // 사용자가 제일 먼저 하는 일이 창을 여는 것이다 → 여는 순간 폴링이 되살아나 청소가
        // 멎고, 화면엔 (stale 이라) 아무것도 안 보인다. 고치려고 한 행동이 고침을 막는 자리였다.
        // 프론트와 부딪히지 않는다: 프론트가 보는 건 `live_request()`(=stale 아닌 것)뿐이라
        // 여기서 지우는 파일은 어차피 그쪽에 없는 것이다.
        clear_dead_request();
        // 한도를 넘는 요청은 창을 깨우기 전에 돌려보낸다 — 프론트가 잠들어 있어도.
        refuse_over_limit();
        // 파일은 한 번만 읽어 아래 판단 전부에 같은 값을 쓴다.
        let live = live_request();
        let dismissal = live
            .as_ref()
            .map(|r| crate::appwin::dismissal_for(&app, &r.id, secs_left(r, now)))
            .unwrap_or(crate::appwin::Dismissal::No);
        let out = watch_tick(
            &mut st,
            TickIn {
                now,
                front_alive: now.saturating_sub(LAST_POLL.load(Ordering::Relaxed))
                    <= FRONT_STALE_SECS,
                live: live.is_some(),
                dismissal,
                can_beat: !crate::wallet::needs_setup(),
            },
        );
        if let Some(ui_ok) = out.beat {
            let _ = write_json(
                heartbeat_path().unwrap_or_default(),
                &Heartbeat {
                    ts: now,
                    ui_ok,
                    kinds: supported_kinds(),
                },
            );
        }
        if out.remind {
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || crate::appwin::show(&handle));
        }
        // WebView 가 죽었다고 판정된 순간, **기다리는 요청을 5분 침묵으로 두지 않는다**.
        // 승인 창이 뜰 수 없다는 걸 아는 쪽이 러스트뿐이므로 여기서 끝내 준다. 「사람이 거부」가
        // 아니라 「띄우지 못했다」라 status 는 failed 다 — 아무도 승인하지 않았고, AI 는 45초 안에
        // 이유를 읽는다. 사람에게도 알린다: 화면이 죽은 걸 알 방법이 알림뿐이다(창이 안 뜬다).
        // 🔴 **승인이 진행 중이면 손대지 않는다**. 사람이 비번을 넣은
        // 직후 WebView 가 죽으면 러스트는 아직 서명·전송 중일 수 있다 — 그때 「실패」라고 답하면
        // AI 가 재시도해 **같은 돈이 두 번** 나갈 수 있다. 진짜로 나가는 중인 결제는
        // 5분 대기가 정직한 답이다(그 5분 안에 성공하면 성공이 그대로 전달된다).
        //
        // 판단과 실행을 **한 잠금 안에서** 한다: 보고 나서 놓으면 그 틈에 승인이
        // 시작돼, 돈은 나가는데 여기선 실패로 적는 상태가 된다. 알림은 잠금 밖에서 띄운다.
        let mut told_dead = false;
        if out.resolve_dead {
            if let Ok(_lock) = APPROVAL_ARBITER.lock() {
                if !approval_in_flight() {
                    if let Some(req) = live_request() {
                        if let Some((r, failed)) = dead_window_result(req.id) {
                            told_dead = failed;
                            let _ = resolve_request(&r);
                        }
                    }
                }
            }
        }
        if told_dead {
            crate::notify::show_notification(
                ts!("지갑 화면이 응답하지 않아요", "The wallet window isn't responding"),
                ts!(
                    "결제 승인 창을 띄우지 못했어요. 앱을 완전히 종료했다 다시 켜 주세요.",
                    "A payment approval prompt couldn't be shown. Quit the app completely and open it again."
                ),
            );
        }
        if let Some(pinned) = out.set_pinned {
            let wake = out.wake;
            let handle = app.clone();
            let _ = app.run_on_main_thread(move || {
                crate::appwin::set_pinned(&handle, pinned);
                if wake {
                    crate::appwin::show(&handle);
                }
            });
        }
    }
}

/// 창 전에 돌려보내는 이유 갈래 — AI 에게 할 말이 다르다.
#[derive(PartialEq, Clone, Copy)]
pub(super) enum Refusal {
    Over,
    Unreadable,
    Legacy,
}

/// 승인 전 한도 검사를 이미 통과한 요청 id. **요청마다 한 번만** 본다 — 창이 뜬 뒤 다른 결제로 한도가 줄었다고
/// 사람이 비번을 넣는 중인 창을 치우지 않는다(그 경우는 서명 직전의 `reserve_spend` 가 이유와 함께 막는다).
pub(super) static PRECHECKED: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

/// 🔴 **한도를 넘는 결제는 승인 창을 띄우지 않고 돌려보낸다**.
///
/// Kura 는 한도를 비번 뒤 서명 단계에서만 걸었다 — 그러면 50 USDC 를 달라는 프롬프트 주입 결제에도 창이 떠서, 사람이
/// 실수로 비번을 넣거나(서명 단계가 막긴 한다) 쓸데없이 판단을 강요받는다. 여기서는 요청을 처음 볼 때 한 번 한도와
/// 대조해, 넘으면 「failed + 이유」 로 끝낸다 — AI 는 이유를 읽고, 사람은 창을 보지 않는다. **읽기만 한다**(예약 없음).
///
/// 승인 진행 중·이미 결말이 적힌 요청은 건드리지 않는다(`reject_payment` 와 같은 잠금·같은 판정). 요청의 체인이 지금과
/// 다르면 손대지 않는다 — 장부가 체인별이라 엉뚱한 장부로 판정하게 되고, 그 요청은 승인 단계가 어차피 거부한다.
/// 단 지갑이 옛 형식이면 체인과 상관없이 돌려보낸다 — 승인 창이 없는 화면이라 요청이 5분 동안 조용히 남는다.
pub(super) fn refuse_over_limit() {
    let Some(req) = live_request() else {
        return;
    };
    if let Ok(done) = PRECHECKED.lock() {
        if *done == req.id {
            return;
        }
    }
    let legacy = crate::wallet::file_has_legacy_address();
    if req.kind != crate::policy::KIND_X402 || (!legacy && ensure_request_chain(&req).is_err()) {
        return;
    }
    // 지갑 파일이 옛 형식(`0x…` 주소)이면 승인 창 대신 「옛 형식 화면」 이 떠 있어 사람은 결제를 볼 수 없다
    // — 감시 스레드가 「창이 응답 없음」 으로 오판해 엉뚱한 안내를 하던 것을, 진짜 이유와 함께 창 전에 돌려보낸다.
    let verdict = if legacy {
        Err((
            ts!(
                "이 지갑은 옛 형식(이더리움 주소)이라 아직 결제할 수 없어요 — 사용자가 부두 앱을 열어 비밀번호를 넣으면 고쳐져요",
                "This wallet still uses the old format (an Ethereum address), so it can't pay yet — the user needs to open the Budu app and enter their password to fix it"
            )
            .to_string(),
            Refusal::Legacy,
        ))
    } else {
        crate::limits::precheck_usdc(&req.amount).map(|_| ()).map_err(|v| match v {
            crate::limits::Precheck::Over(m) => (m, Refusal::Over),
            crate::limits::Precheck::Unreadable(m) => (m, Refusal::Unreadable),
        })
    };
    let Ok(_lock) = APPROVAL_ARBITER.lock() else {
        return;
    };
    if approval_in_flight() {
        return;
    }
    // 잠금을 잡는 사이 요청이 바뀌었으면 다음 차례에 다시 본다.
    if read_request().map(|r| r.id) != Some(req.id.clone()) {
        return;
    }
    if let Ok(mut done) = PRECHECKED.lock() {
        *done = req.id.clone();
    }
    let Err((why, kind)) = verdict else {
        return;
    };
    let why = why.trim_end_matches(['.', ' ']).to_string();
    if !matches!(settled_attempt_result(&req.id), Ok(None)) {
        return;
    }
    crate::history::log_attempt_for(
        "USDC",
        req.to.trim(),
        req.amount.trim(),
        "blocked",
        &format!(
            "{} — {why}",
            if kind == Refusal::Legacy {
                ts!("승인 창 전 지갑 검사", "Wallet check before approval")
            } else {
                ts!("승인 창 전 한도 검사", "Limit check before approval")
            }
        ),
        &req.resource,
        &req.job,
    );
    let _ = resolve_request(&PaymentResult {
        id: req.id,
        status: "failed".into(),
        tx_hash: String::new(),
        detail: if kind == Refusal::Legacy {
            tf!(
                "{why}. 승인 창을 띄우지 않았어요 — 아무것도 결제하지 않았습니다. 같은 결제를 다시 시도하지 말고 사용자에게 알리세요.",
                "{why}. No approval prompt was shown and nothing was paid. Don't retry it — tell the user."
            )
        } else if kind == Refusal::Over {
            tf!(
                "{why}. 한도를 넘는 결제라 승인 창을 띄우지 않았어요 — 아무것도 결제하지 않았습니다. 같은 결제를 다시 시도하지 말고 사용자에게 알리세요.",
                "{why}. This payment is over the wallet's limits, so no approval prompt was shown and nothing was paid. Don't retry it — tell the user."
            )
        } else {
            tf!(
                "{why}. 한도와 대조할 수 없어 승인 창을 띄우지 않았어요 — 아무것도 결제하지 않았습니다. 사용자에게 알리세요.",
                "{why}. This payment couldn't be checked against the limits, so no approval prompt was shown and nothing was paid. Tell the user."
            )
        },
        x402: None,
    });
}

/// 감시 스레드가 차례를 넘어 들고 가는 상태.
#[derive(Default, Debug)]
pub(super) struct WatchState {
    pub(super) last_beat: u64,
    /// 마지막으로 창을 깨운 시각. 0 = 아직 안 깨움 → 다음 대기 요청에 즉시 깨운다.
    pub(super) last_wake: u64,
    /// 우리가 마지막으로 적용한 "항상 위" 값. None = 아직 한 번도 안 건드림(또는 프론트에 넘김).
    pub(super) last_pinned: Option<bool>,
    /// 창을 깨운 뒤에도 프론트가 폴링을 안 한 횟수. 폴링이 한 번이라도 돌아오면 0으로 리셋.
    pub(super) wakes_without_poll: u32,
}

/// 한 차례의 입력 — 시각과 파일·창에서 읽은 사실.
pub(super) struct TickIn {
    pub(super) now: u64,
    /// 프론트가 `FRONT_STALE_SECS` 안에 폴링했다.
    pub(super) front_alive: bool,
    /// 보여줄 대기 요청이 있다.
    pub(super) live: bool,
    pub(super) dismissal: crate::appwin::Dismissal,
    /// 하트비트를 찍어도 되는 상태(지갑이 있다).
    pub(super) can_beat: bool,
}

/// 한 차례에 할 일 — 파일·창을 만지는 건 호출자가 한다.
#[derive(Debug, PartialEq, Default)]
pub(super) struct TickOut {
    /// 하트비트를 찍는다(값 = ui_ok).
    pub(super) beat: Option<bool>,
    /// 닫아 둔 승인 창을 만료 직전에 한 번 되살린다.
    pub(super) remind: bool,
    /// WebView 가 죽었다 — 기다리는 요청을 실패로 끝낼지 본다(진행 중 승인이 없을 때만, 호출자가 잠금 안에서).
    pub(super) resolve_dead: bool,
    /// "항상 위" 를 이 값으로 맞춘다(None = 안 건드림).
    pub(super) set_pinned: Option<bool>,
    /// 맞추면서 창도 띄운다(잠든 프론트 깨우기).
    pub(super) wake: bool,
}

/// 감시 스레드 한 차례의 판단. 예전엔 이 판단이 1초 루프 안에 흩어져 있어
/// 「언제 깨우고·언제 죽었다고 보고·언제 고정을 푸는가」를 테스트가 볼 수 없었다.
///
/// **프론트가 깨어 있으면 하트비트·되살리기 말고는 아무것도 하지 않는다** — 폴링이 이미 같은 일(고정 수렴·모달·
/// 자율 승인)을 한다. 요청이 오자마자 창을 띄우면 창 없이 조용히 끝나야 하는 자율 결제까지 창이 튀어나온다.
pub(super) fn watch_tick(st: &mut WatchState, i: TickIn) -> TickOut {
    use crate::appwin::Dismissal;
    let mut out = TickOut::default();
    if i.front_alive {
        st.wakes_without_poll = 0;
    }
    // 창을 여러 번 깨워도 폴링이 안 돌아온다 = WebView 가 죽었다.
    // **마지막 깨움에도 회복할 시간을 준다** — 카운터는 깨운 *뒤*에 올라가므로
    // 이 조건만 보면 3번째 깨움은 다음 차례에 곧장 사망 판정을 맞는다. 실측상 깨운 뒤 폴링이
    // 돌아오는 데 5초쯤 걸린다. 마지막 깨움이 `WAKE_RETRY_SECS` 만큼 묵은 뒤에야 판정 = "3회 ≈ 45초".
    let ui_ok =
        st.wakes_without_poll < DEAD_WAKES || i.now.saturating_sub(st.last_wake) < WAKE_RETRY_SECS;
    // 하트비트가 뜻하는 건 **"여기서 사람이 승인까지 할 수 있다"** — 지갑이 없으면(첫 실행) 승인 창을 띄울
    // 경로가 없으니 찍지 않는다.
    if i.now.saturating_sub(st.last_beat) >= HEARTBEAT_SECS && i.can_beat {
        st.last_beat = i.now;
        out.beat = Some(ui_ok);
    }
    // 사용자가 **일부러 닫아 둔** 승인 창은 깨우기에서 빼고, 만료 직전에 한 번만 되살린다. 프론트 생사보다
    // 먼저 본다 — 닫힌 창의 WebView 는 아직 폴링 중일 수도, 이미 잠들었을 수도 있는데 되살릴 수 있는 건 러스트뿐이다.
    if i.dismissal == Dismissal::Remind {
        st.last_wake = i.now; // 되살린 직후 아래 깨우기가 한 번 더 띄우지 않게
        out.remind = true;
    }
    if i.front_alive {
        st.last_pinned = None; // 주도권을 넘긴다 — 다시 잠들면 그때 처음부터 다시 맞춘다.
        return out;
    }
    // 여기서부터는 "프론트가 잔다" — 고정 수렴도 우리가 대신한다. 닫아 둔 요청(Hold·Remind)엔 고정을 걸지 않는다
    // (개발 7 1차 리뷰 P3 — 숨은 창에 고정이 남지 않게. Remind 는 `appwin::show` 가 표식을 지운 뒤 스스로 고정한다).
    let pinned = i.live && i.dismissal == Dismissal::No;
    // Remind 차례는 `appwin::show` 가 창을 띄우고 표식을 지운 뒤 스스로 고정한다. 여기서 고정을 건드리면
    // (`last_pinned == Some(true)` → `set_pinned(false)`) 방금 건 고정을 풀 수 있고, `last_wake` 를 0 으로 지우면
    // 위에서 적은 값이 사라져 다음 차례에 곧바로 깨우기가 걸린다(사망 카운터가 한 칸 일찍 오른다). 둘 다 건너뛴다
    //. 다음 차례에 처음부터 다시 맞춘다.
    if i.dismissal == Dismissal::Remind {
        st.last_pinned = None;
        out.resolve_dead = !ui_ok;
        return out;
    }
    if !pinned {
        st.last_wake = 0;
    }
    // 닫아 둔 요청은 깨우지 않는다(Hold·Remind 둘 다 — 위 pinned 가 이미 false). 그래서 사망 판정 카운터도 안 오른다 —
    // 폴링이 멎은 건 WebView 가 죽어서가 아니라 사용자가 닫아서다.
    let wake = pinned && i.now.saturating_sub(st.last_wake) >= WAKE_RETRY_SECS;
    if wake {
        st.last_wake = i.now;
        st.wakes_without_poll = st.wakes_without_poll.saturating_add(1);
    }
    out.resolve_dead = !ui_ok;
    if pinned || st.last_pinned == Some(true) || wake {
        st.last_pinned = Some(pinned);
        out.set_pinned = Some(pinned);
        out.wake = wake;
    }
    out
}

/// 감시 스레드를 띄운다 (setup 에서 한 번).
pub(crate) fn spawn_watchdog(app: &tauri::AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || watchdog(app));
}
