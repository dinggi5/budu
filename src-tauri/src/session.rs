// 자율 결제 — 세션 잠금 해제 + 소액 자동 승인.
//
// 컨셉: 부두 = AI 에이전트 전용 지갑, 사람은 수호자. 매 결제마다 비번을 누르면 자율성이 없다.
// → 보호자가 비번을 "한 번" 입력해 세션을 잠금 해제하면, 그 키를 GUI 프로세스 "메모리에만"
//   (zeroize 보호) 들고 있다가, 자율 한도 이하의 AI 결제 요청은 비번 없이 자동 승인한다.
//
// 보안 불변식:
//   - 키(니모닉)는 디스크에 평문으로 절대 안 나간다. 잠금 해제 동안만 RAM에 존재.
//   - 앱 종료 → 메모리 소멸. 긴급 잠금 → 즉시 소멸(set_locked). 유휴 타임아웃 → 자동 소멸.
//   - 한도 초과는 자율 불가 → 항상 사람 비번(승인 창). 자율 한도 0 = 자율 꺼짐(=기존 동작).
//   - 자율 승인도 do_sign_x402 를 그대로 거치므로 긴급잠금·단일/일일 한도·내역이 동일 적용.

use crate::i18n::ts;
use serde::Serialize;
use std::sync::Mutex;
use zeroize::Zeroizing;

use crate::chain::active_chain;
use crate::keys::SolKeypair;
use crate::ipc::{read_request, PaymentResult};
use crate::limits::parse_usdc_nonneg;
use crate::notify::{auto_pay_notice, show_notification};
use crate::settings::{read_settings, Settings};
use crate::store::now_secs;
use crate::trusted::is_trusted;
use crate::wallet::{
    decrypt_wallet, read_encrypted, with_pinned_account,
};
use crate::x402::do_sign_x402;

/// 자율 승인 불가(세션 잠김·한도 초과·자율 꺼짐) 신호. 프론트가 이걸 보면 사람 승인 모달을 띄운다.
pub(crate) const NEEDS_PASSWORD: &str = "NEEDS_PASSWORD";

/// 잠금 해제된 세션 키(니모닉)를 메모리에만 보관한다. 디스크에 절대 쓰지 않는다.
#[derive(Default)]
pub(crate) struct SessionKey(pub(crate) Mutex<Option<SessionInner>>);

pub(crate) struct SessionInner {
    /// 복호화된 12단어 니모닉. Drop 시 메모리를 0으로 지운다(zeroize).
    mnemonic: Zeroizing<String>,
    /// 마지막 자율 결제 시각(유닉스 초). 유휴 타임아웃 계산용.
    last_active: u64,
}

/// 설정의 자동 잠금(분)을 초로 환산한다. 0이면 유휴 잠금 안 함.
/// 깨진 값(수동 편집 등)이면 기본 30분 — 보안 기능의 실패 방향이 "조용히 꺼짐"이면 안 된다.
fn auto_lock_secs(settings: &Settings) -> u64 {
    settings
        .auto_lock_mins
        .trim()
        .parse::<u64>()
        .unwrap_or(30)
        .saturating_mul(60)
}

/// 자율 한도 이하인지 — 순수 판정(테스트용). USDC 표시 결제만, 한도>0, 금액≤한도.
fn within_auto_limit(token: &str, amount: u128, auto_limit: u128) -> bool {
    token == "USDC" && auto_limit != 0 && amount <= auto_limit
}

/// 세션에 보관된 키로 **활성 계정**(작업이 고정했으면 그 계정)의 서명자를 만든다.
/// 세션은 지갑 하나에 하나다 — 비번 한 번이면 어느 계정이든 자율 결제가 된다. 유휴 타임아웃이
/// 지났으면 자동 잠그고 None. 성공 시 마지막 활동 시각을 갱신한다(MutexGuard 는 .await 전에 반드시 해제).
fn session_signer(state: &SessionKey, idle_secs: u64) -> Option<SolKeypair> {
    let phrase = {
        let mut g = state.0.lock().ok()?;
        let expired = g
            .as_ref()
            .map(|inner| idle_secs > 0 && now_secs().saturating_sub(inner.last_active) > idle_secs)
            .unwrap_or(true);
        if expired {
            *g = None;
            return None;
        }
        let inner = g.as_mut()?;
        inner.last_active = now_secs();
        inner.mnemonic.clone()
    };
    crate::wallet::session_signer_for(&phrase).ok()
}

/// 비번으로 세션을 잠금 해제한다 — 복호화한 니모닉을 메모리에만 보관(디스크 X).
/// 이후 자율 한도 이하 결제는 비번 없이 자동 승인된다. 비번이 틀리면 복호화에서 거부.
#[tauri::command]
pub(crate) fn unlock_session(
    password: String,
    session: tauri::State<'_, SessionKey>,
) -> Result<(), String> {
    let password = Zeroizing::new(password);
    let w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, &password)?; // 비번 검증(틀리면 GCM 실패)
    // EVM 시절 `0x…` 주소면 고쳐 쓴다. 실패해도 잠금 해제는 막지 않는다 — 서명 때 대조가 판단한다.
    let w = crate::wallet::repair_legacy_addresses(&w, &phrase).unwrap_or(w);
    crate::wallet::maybe_upgrade_kdf(&w, &phrase, &password); // 옛 KDF(v2)면 v3로 강화
    let mut g = session
        .0
        .lock()
        .map_err(|_| ts!("세션 상태 잠금 실패", "Couldn't read the session state").to_string())?;
    *g = Some(SessionInner {
        mnemonic: phrase,
        last_active: now_secs(),
    });
    Ok(())
}

/// 세션을 수동으로 잠근다 — 메모리의 키를 즉시 소멸시킨다.
#[tauri::command]
pub(crate) fn lock_session(session: tauri::State<'_, SessionKey>) {
    if let Ok(mut g) = session.0.lock() {
        *g = None;
    }
    crate::job::clear(); // 작업 예산의 키도 같이 버린다
}

/// 프론트로 주는 자율 결제 세션 상태.
#[derive(Serialize)]
pub(crate) struct SessionStatus {
    /// 세션이 잠금 해제돼 있는지(=자율 결제 가능).
    unlocked: bool,
    /// 자동 잠금까지 남은 초. 유휴 타임아웃 0이거나 잠겨 있으면 0.
    remaining_secs: u64,
    /// 자율 결제 한도(USDC, 십진수). "0"이면 자율 결제 꺼짐.
    auto_limit: String,
}

/// 세션 상태를 알려준다(프론트 1초 폴링). 유휴 타임아웃이 지났으면 여기서도 자동 잠근다.
/// 단순 조회이므로 last_active 는 갱신하지 않는다(폴링이 세션을 영원히 살리지 않게).
#[tauri::command]
pub(crate) fn session_status(session: tauri::State<'_, SessionKey>) -> SessionStatus {
    let settings = read_settings();
    let idle = auto_lock_secs(&settings);
    let (unlocked, remaining) = match session.0.lock() {
        Ok(mut g) => match g.as_ref() {
            Some(inner) => {
                let elapsed = now_secs().saturating_sub(inner.last_active);
                if idle > 0 && elapsed > idle {
                    *g = None; // 유휴 → 자동 잠금
                    (false, 0)
                } else {
                    let rem = if idle > 0 {
                        idle.saturating_sub(elapsed)
                    } else {
                        0
                    };
                    (true, rem)
                }
            }
            None => (false, 0),
        },
        Err(_) => (false, 0),
    };
    SessionStatus {
        unlocked,
        remaining_secs: remaining,
        auto_limit: settings.auto_approve_usdc,
    }
}

/// 대기 중인 AI 결제 요청을 "자율 승인"으로 처리 시도한다(비번 없이).
/// 자율 불가(세션 잠김·한도 초과·자율 꺼짐·ETH 송금)면 NEEDS_PASSWORD 를 돌려준다
/// → 프론트가 사람 승인 모달을 띄운다. do_* 가 긴급잠금·한도·내역을 그대로 적용한다.
#[tauri::command]
pub(crate) async fn auto_approve_payment(
    id: String,
    session: tauri::State<'_, SessionKey>,
) -> Result<PaymentResult, String> {
    let req = read_request().ok_or(ts!(
        "대기 중인 결제 요청이 없습니다",
        "There's no payment request waiting"
    ))?;
    if req.id != id {
        return Err(ts!(
            "요청 ID가 일치하지 않습니다",
            "That request ID doesn't match"
        )
        .into());
    }
    crate::ipc::ensure_request_chain(&req)?; // 요청 시점 체인 ≠ 현재 체인이면 거부(자율 경로도 동일)
    crate::ipc::ensure_request_account(&req)?; // 요청 시점 계정 ≠ 현재 계정이면 거부

    // 🔴 **여기서부터 체인을 못박는다**. 위 검사는 *그 순간*만 본다 —
    // 그 뒤로 잔액 조회·서명·전송에 `.await` 가 여러 번 있고, 그동안 사용자가 네트워크를 바꾸면
    // 서명 경로가 **그때 활성인 체인**을 고정해 버린다. 즉 데브넷에서 만들어진 자율 결제가
    // 비번 없이 **메인넷에서 서명될 수 있다** — 요청-체인 가드가 있으나 마나가 된다.
    // 작업 전체를 요청의 체인으로 묶으면 그 창이 사라진다(안쪽 do_* 의 고정은 같은 값이 된다).
    // chain_id 0 = 옛 미각인 요청 → 예전대로 활성 체인을 쓴다.
    let pinned = if req.chain_id != 0 {
        req.chain_id
    } else {
        active_chain().chain_id
    };
    // 계정도 같은 이유로 못박는다 — 자율 경로는 창이 없어 사용자가 계정을 바꾸는 사이에
    // 조용히 지나간다. 요청의 계정으로 묶으면 세션 키에서 파생하는 서명자와 내역 파일이 그 계정이다.
    let account = crate::ipc::request_account_index(&req);
    crate::chain::with_pinned_chain(
        pinned,
        with_pinned_account(account, auto_approve_pinned(req, session)),
    )
    .await
}

/// 체인이 고정된 채로 도는 자율 승인 본체 — 한도·잔액·서명·전송·장부가 모두 같은 체인을 본다.
async fn auto_approve_pinned(
    req: crate::ipc::PaymentRequest,
    session: tauri::State<'_, SessionKey>,
) -> Result<PaymentResult, String> {
    // 사람이 연 작업 예산이 이 요청을 덮으면 그 길로 간다 — 전역 자율 한도보다 먼저. 못 덮으면 아래 예전 규칙.
    if crate::job::covers(&req) {
        return job_approve(req).await;
    }
    // 자율 한도 판정 — USDC 표시 결제(x402 서명·USDC 송금)만, 한도 이하만.
    // 음수 한도는 0(자율 꺼짐)으로, 음수 금액은 오류로 — 거대 수 둔갑 함정 차단(parse_usdc_nonneg).
    let dec = active_chain().usdc_decimals;
    let settings = crate::settings::read_settings_for_active_chain();
    let auto_limit: u128 =
        parse_usdc_nonneg(&settings.auto_approve_usdc, dec).unwrap_or(0);
    let amount: u128 = parse_usdc_nonneg(&req.amount, dec)?;
    if !within_auto_limit(&req.token, amount, auto_limit) {
        return Err(NEEDS_PASSWORD.into());
    }

    // 화이트리스트 가드 — 목록에 없는 주소는 자율 대상이 아니다(금액 한도만으로는 처음 보는 주소로도 비번 없이
    // 나가는 구멍). 목록은 사용자가 직접 넣는다(승인 때 자동 학습은 옵션, 기본 꺼짐).
    if settings.auto_trusted_only && !is_trusted(&req.to) {
        return Err(NEEDS_PASSWORD.into());
    }

    // 🔴 **같은 결제를 짧은 시간에 두 번이면 사람에게 묻는다**.
    // 자율 결제가 성공한 뒤 AI 가 그 응답을 못 받으면(죽었다 켜짐·시간 초과) 「아까 실패했다」고 알고 같은 결제를 또
    // 요청한다 — 겹침 거절(`begin_approval`)은 **겹치는 창**만 닫고, 끝난 뒤의 재요청은 한도 안이면 조용히 또 나갔다.
    // 같은 토큰·받는 곳·금액이 10분 안에 나갔으면
    // (나갔을 수 있는 「불명」 포함) 자율로 처리하지 않는다 — 막는 게 아니라 사람이 승인 창에서 그 사실을 보고 정한다.
    // 내역을 못 읽어 모르는 것도 사람에게.
    if crate::history::autopay_needs_human_for_repeat(&req.token, &req.to, &req.amount) {
        return Err(NEEDS_PASSWORD.into());
    }

    // 여기서부터는 돈이 나갈 수 있는 구간 — 감시 스레드가 이 요청을 실패로 끝내면 안 된다
    // (자율 경로는 창이 없어 「프론트가 잔다」와 구별이 더 어렵다).
    let _in_flight = crate::ipc::begin_approval(&req)?;
    // 여기서부터의 결말은 전부 결제 시도 기록에 남는다(ipc::finish_approval) — 사람 승인과
    // **같은 함수**로 끝낸다. NEEDS_PASSWORD 도 기록상 「실패(안 나감)」라 곧이어 사람이 승인할 수 있다.
    let outcome = auto_work(&req, &session, &settings).await;
    let result = crate::ipc::finish_approval(&req, outcome)?;

    // 자율 결제 사후 통지 — 비번 없이 돈이 나간 유일한 경로이므로,
    // 보호자가 자리에 없어도 OS 알림으로 인지하게 한다. 알림 실패가 결제를 막으면 안 된다.
    // 불명(unknown)은 전송 쪽이 이미 「확인 필요」 알림을 띄웠다 — 「자동으로 결제했어요」를 겹쳐 말하지 않는다.
    if settings.notify_auto && result.status == "approved" {
        let notice = auto_pay_notice(
            &req.kind,
            &req.token,
            &req.amount,
            &req.to,
            &req.resource,
            settings.notify_hide_amount,
        );
        show_notification(&notice.0, &notice.1);
    }
    Ok(result)
}

/// 작업 예산으로 처리한다 — 비번 없이 나가는 두 번째 길이라 자율 경로와 같은 문을 다 거친다:
/// 같은 결제 10분 재요청 검사(결제 대상 URL 까지 같을 때만 — 한 작업이 같은 판매자에게 같은 값의 다른 데이터를 사는 게 정상이다.
/// 앞의 같은 결제가 «정산됨 + 내용 받음» 이면 새로 사는 것으로 보고 통과 — 개발 8, `history::last_same_payment`),
/// 승인 진행 중 표식, 요청 재확인, `do_sign_x402`(잠금·한도·내역), 결말 기록, 사후 알림.
async fn job_approve(req: crate::ipc::PaymentRequest) -> Result<PaymentResult, String> {
    if crate::history::autopay_needs_human_for_repeat_of(&req.token, &req.to, &req.amount, Some(&req.resource)) {
        return Err(NEEDS_PASSWORD.into());
    }
    let settings = read_settings();
    let _in_flight = crate::ipc::begin_approval(&req)?;
    let outcome = job_work(&req).await;
    let result = crate::ipc::finish_approval(&req, outcome)?;
    if settings.notify_auto && result.status == "approved" {
        let notice = auto_pay_notice(
            &req.kind,
            &req.token,
            &req.amount,
            &req.to,
            &req.resource,
            settings.notify_hide_amount,
        );
        show_notification(&notice.0, &notice.1);
    }
    Ok(result)
}

async fn job_work(req: &crate::ipc::PaymentRequest) -> Result<PaymentResult, String> {
    let terms = req.svm.clone().ok_or(NEEDS_PASSWORD)?;
    // 예산을 **먼저 뗀다** — 겹친 두 결제가 둘 다 「남은 예산 안」 으로 보고 예산을 넘지 않게.
    let (phrase, claim) = crate::job::claim(req).ok_or(NEEDS_PASSWORD)?;
    let signed = async {
        let signer = crate::wallet::session_signer_for(&phrase)?;
        // await 를 지나기 전에 요청이 아직 그대로인지 본다(자율 경로와 같은 이유 — 상대가 거둬 간 뒤 서명하면 이중 결제).
        if read_request().map(|r| r.id).as_deref() != Some(req.id.as_str()) {
            return Err(ts!(
                "결제 요청이 이미 취소됐거나 시간이 지났어요.",
                "That payment request was already cancelled or timed out."
            )
            .to_string());
        }
        do_sign_x402(
            &signer,
            req.to.clone(),
            req.amount.clone(),
            terms,
            req.resource.clone(),
            req.job.clone(),
        )
        .await
    }
    .await;
    match signed {
        Ok(payment) => {
            crate::job::attach(claim, &payment.signature);
            Ok(PaymentResult {
                id: req.id.clone(),
                status: "approved".into(),
                tx_hash: String::new(),
                detail: String::new(),
                x402: Some(payment),
            })
        }
        Err(e) => {
            // Err = 서명이 안 나갔다 → 떼어 둔 몫을 돌려준다. 기록이 「정산 대기」 로 남은 경우(`log_signed` 의 Err(Some))에도
            // 그 서명은 예산의 몫으로 적히지 않았으니 체인 확인이 다시 돌려주지 않는다(1차 리뷰 P2 — 예전엔 두 번 돌려줬다).
            crate::job::release_claim(claim);
            Err(e)
        }
    }
}

/// 승인이 시작된 뒤의 자율 처리 본체 — 세션 키·재확인·서명. 결말은 호출자가 기록한다.
async fn auto_work(
    req: &crate::ipc::PaymentRequest,
    session: &tauri::State<'_, SessionKey>,
    settings: &Settings,
) -> Result<PaymentResult, String> {
    // 세션이 잠금 해제돼 있어야 키가 메모리에 있다(유휴 타임아웃도 여기서 검사).
    let idle = auto_lock_secs(settings);
    let signer = session_signer(session, idle).ok_or_else(|| NEEDS_PASSWORD.to_string())?;

    // 🔴 **await 를 지나온 뒤, 그 요청이 아직 대기 중인지 다시 본다**.
    // 서명 경로는 요청 파일을 보지 않으므로, 세션 확인이 늦어 상대가 시간 초과로 요청을
    // 거둬간 뒤에도 그대로 돈을 보내 버린다 — 그쪽은 이미 재시도했을 수 있다(이중 결제).
    // 사라졌거나 다른 요청으로 바뀌었으면 여기서 멈춘다(아무도 승인하지 않은 자율 처리다).
    if read_request().map(|r| r.id).as_deref() != Some(req.id.as_str()) {
        return Err(ts!(
            "결제 요청이 이미 취소됐거나 시간이 지났어요.",
            "That payment request was already cancelled or timed out."
        )
        .into());
    }

    // 실제 처리 — 긴급잠금·단일/일일 한도·내역·누적은 do_* 가 송금과 동일하게 적용.
    // 여기서 Err(잠금·한도 등)이면 요청을 치우지 않는다 → 프론트가 모달로 사람에게 넘긴다.
    use crate::policy::KIND_X402;
    match req.kind.as_str() {
        KIND_X402 => {
            // 조건이 없으면 자율로 처리하지 않고 사람에게 넘긴다(승인 경로가 정직하게 거절한다).
            let terms = req.svm.clone().ok_or(NEEDS_PASSWORD)?;
            let payment = do_sign_x402(
                &signer,
                req.to.clone(),
                req.amount.clone(),
                terms,
                req.resource.clone(),
                req.job.clone(),
            )
            .await?;
            Ok(PaymentResult {
                id: req.id.clone(),
                status: "approved".into(),
                tx_hash: String::new(),
                detail: String::new(),
                x402: Some(payment),
            })
        }
        // 🔴 모르는 kind 는 자율로 처리하지 않는다 — 자율 경로는 창이 없어 사람이 볼 기회조차 없다.
        _ => Err(NEEDS_PASSWORD.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // 자율 한도 판정: USDC 표시 + 한도>0 + 금액≤한도 일 때만 자율 가능.
    #[test]
    fn within_auto_limit_gating() {
        let one = u128::from(1_000_000u64); // 1 USDC
        let two = u128::from(2_000_000u64); // 2 USDC
                                            // 한도 2, 금액 1 → 자율 가능
        assert!(within_auto_limit("USDC", one, two));
        // 한도 2, 금액 정확히 2 → 가능(이하 포함)
        assert!(within_auto_limit("USDC", two, two));
        // 한도 1, 금액 2 → 초과 → 사람 승인 필요
        assert!(!within_auto_limit("USDC", two, one));
        // 한도 0 = 자율 꺼짐 → 항상 불가
        assert!(!within_auto_limit("USDC", one, 0));
        // ETH 송금은 자율 대상 아님(USDC만)
        assert!(!within_auto_limit("ETH", one, two));
    }

    // 자동 잠금(분) → 초 환산. 0이면 유휴 잠금 안 함.
    #[test]
    fn auto_lock_secs_converts_minutes() {
        let mut s = Settings {
            auto_lock_mins: "30".into(),
            ..Default::default()
        };
        assert_eq!(auto_lock_secs(&s), 1800);
        s.auto_lock_mins = "0".into();
        assert_eq!(auto_lock_secs(&s), 0); // 명시적 0 = 사용자가 끈 것 → 존중
        s.auto_lock_mins = "쓰레기".into(); // 깨진 값 → 기본 30분(잠금이 조용히 꺼지면 안 됨)
        assert_eq!(auto_lock_secs(&s), 1800);
    }
}

/// 자동 경로(작업 예산·자율 세션)의 future 도 작게 — `ipc::future_size_guard` 와 같은 이유.
#[cfg(test)]
mod future_size_guard {
    #[test]
    fn auto_futures_stay_small() {
        let req = crate::ipc::tests::req_created(0);
        let n = std::mem::size_of_val(&super::job_approve(req.clone()));
        assert!(n < 16 * 1024, "job_approve future = {n} bytes");
        let n = std::mem::size_of_val(&super::job_work(&req));
        assert!(n < 16 * 1024, "job_work future = {n} bytes");
    }
}
