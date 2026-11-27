// 결제 승인 IPC + AI 연결 배지.
//
// AI 에이전트가 MCP로 결제를 "요청"하면, 별도 프로세스인 MCP 서버가 ~/.budu에 요청 파일을
// 쓴다. GUI 앱(이 프로세스)이 1초마다 폴링해 발견 → 승인 시트 → 사용자가 비번 입력 → x402 결제
// 서명(x402.rs) → 결과 파일 작성 → MCP가 읽어 판매자에 다시 요청하고 에이전트에 반환.
//
// 핵심 보안: 비번은 절대 요청/결과 파일이나 MCP에 들어가지 않는다. 키 접근(서명)은 오직 이
// GUI 프로세스만 한다 → MCP는 "요청"만, 승인은 사람이. 잠금·한도·내역은 모든 승인 경로가
// 같은 `x402::do_sign_x402` 를 거치므로 자동 적용된다.

use crate::i18n::{tf, ts};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

use crate::store::{budu_dir, now_secs, write_json};
use crate::policy::SvmSigned;
use crate::x402::sign_x402_payment;

/// AI 에이전트가 보낸 결제 요청 1건. 비밀은 없다(비번은 GUI에서만 입력).
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct PaymentRequest {
    /// 고유 id (MCP가 생성한 유닉스 나노초 문자열). 결과 매칭용.
    pub(crate) id: String,
    /// 토큰 기호 — 지금은 "USDC" 하나(다른 값이면 승인하지 않는다).
    pub(crate) token: String,
    /// 받는 주소.
    pub(crate) to: String,
    /// 금액 (십진수 문자열).
    pub(crate) amount: String,
    /// 무엇에 대한 결제인지 — 사용자가 승인 팝업에서 보고 판단한다.
    pub(crate) memo: String,
    /// 요청 생성 유닉스 초 (타임아웃 카운트다운용).
    pub(crate) created: u64,
    /// 요청 종류 — 지금은 "x402"(오프체인 서명) 하나다. 없거나 모르는 값이면 승인하지 않는다.
    #[serde(default)]
    pub(crate) kind: String,
    /// x402일 때 결제 대상 리소스 URL (팝업 표시용). transfer면 빈 문자열.
    #[serde(default)]
    pub(crate) resource: String,
    /// 요청이 만들어진 시점의 체인 ID (MCP가 활성 체인으로 각인). 승인 시 현재 활성 체인과 다르면
    /// 거부한다 — "테스트넷이라 생각하고 만든 대기 요청"이 메인넷으로 바뀐 뒤 실행되는 것 차단.
    /// 0 = 미각인(옛 요청 파일 호환) → 체인 검사 건너뜀.
    #[serde(default)]
    pub(crate) chain_id: u64,
    /// 요청이 만들어진 시점의 **활성 계정** — MCP 가 wallet.enc 의 활성 계정을 각인한다.
    /// 승인 시 지금 계정과 다르면 거부하고, 승인 작업 전체를 이 계정으로 고정한다(체인과 같은 처방).
    /// `from` 이 빈 값 = 미각인(옛 요청 파일·옛 MCP) → **계정 0 의 요청**으로 본다(활성이 0 일 때만 승인).
    #[serde(default)]
    pub(crate) account: u32,
    #[serde(default)]
    pub(crate) from: String,
    /// **임대를 갱신하는 요청인가**. 새 MCP 는 기다리는 동안 요청 파일의 수정 시각을
    /// 주기적으로 갱신한다 — 멈추면(프로세스가 SIGKILL 로 죽어 `CancelOnDrop` 도 못 돈 경우) GUI 는
    /// `REQUEST_LEASE_SECS` 뒤부터 그 요청을 보여주지도 승인하지도 않는다. 옛 MCP 의 요청엔 이 필드가
    /// 없다(false) → 예전 규칙(5분+유예) 그대로다.
    #[serde(default)]
    pub(crate) lease: bool,
    /// x402 솔라나 결제 조건 — MCP 가 402 에서 고른 요구. 없으면 서명하지 않는다(닫힌 쪽).
    /// 승인 창이 이 값의 `fee_payer`·`memo` 를 같이 보여 준다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) svm: Option<crate::policy::SvmTerms>,
    /// 작업 이름표 — MCP 가 `clip_job` 으로 정리해 싣는다. 우리도 쓰기 전에 다시 정리한다(요청 파일은 아무나 쓴다).
    /// 이름표일 뿐 권한이 아니다 — 작업 예산은 사람이 승인 창에서만 연다(`job.rs`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub(crate) job: String,
}

/// 대기 요청이 만들어진 체인과 현재 활성 체인이 같은지 검사한다. 다르면 거부 —
/// 승인/자율 경로 공용. chain_id 0 = 옛 미각인 요청이므로 검사를 건너뛴다(후방호환).
pub(crate) fn ensure_request_chain(req: &PaymentRequest) -> Result<(), String> {
    if req.chain_id != 0 && req.chain_id != crate::chain::active_chain().chain_id {
        return Err(
            ts!("결제 요청이 만들어진 네트워크와 현재 네트워크가 달라요 — 네트워크를 확인하고 다시 요청하세요.", "This request was made on a different network than the one you're on now — check the network and ask again.")
                .into(),
        );
    }
    Ok(())
}

/// 대기 요청이 각인한 계정과 지금 활성 계정이 같은지 검사한다. 다르면 거부 —
/// 승인/자율 경로 공용. MCP 는 요청을 쓸 때 wallet.enc 의 활성 계정(인덱스+주소)을 적고, GUI 는
/// 승인 순간 같은 파일을 다시 읽어 대조한다. 그 사이 사용자가 계정을 바꿨으면 「화면 계정 ≠ 돈
/// 나가는 계정」이므로 보내지 않는다. `from` 빈 값 = 옛 요청 → 계정 0 의 요청으로(아래).
pub(crate) fn ensure_request_account(req: &PaymentRequest) -> Result<(), String> {
    if req.from.is_empty() {
        // 각인이 없는 요청은 **계정 0 의 요청**이다. 이 앱보다 먼저 떠 있던
        // 옛 사이드카는 계정을 모르고 `address` 필드(= 계정 0)의 주소·잔액을 AI 에게 말해 왔다 —
        // 그 정보로 만든 결제를 지금 활성인 다른 계정에서 내보내면 AI 가 본 계정과 돈이 나가는
        // 계정이 갈린다. 지갑을 못 읽으면(옛 평문 지갑) 예전 동작 그대로 — 어차피 승인은 못 한다.
        if let Ok(w) = crate::wallet::read_encrypted() {
            if w.active_account().index != 0 {
                return Err(ts!(
                    "이 결제 요청엔 계정 정보가 없어요(옛 AI 연결). 계정 1로 바꾸거나, AI 연결을 다시 한 뒤 요청하세요.",
                    "This request carries no account (an older AI connection made it). Switch to Account 1, or reconnect the AI and ask again."
                )
                .into());
            }
        }
        return Ok(());
    }
    let active = crate::wallet::active_account()?;
    // base58 은 대소문자를 가린다 — 정확히 같아야 같은 계정.
    if active.index != req.account || active.address.trim() != req.from.trim() {
        return Err(ts!(
            "결제 요청이 만들어진 계정과 지금 계정이 달라요 — 계정을 확인하고 다시 요청하세요.",
            "This request was made for a different account than the one that's active now — check the account and ask again."
        )
        .into());
    }
    Ok(())
}

/// 승인 작업을 고정할 계정 인덱스 — 각인된 요청은 그 계정, 각인 없는 옛 요청은 계정 0
/// (ensure_request_account 가 그때 활성도 0 임을 이미 확인했다).
pub(crate) fn request_account_index(req: &PaymentRequest) -> u32 {
    if req.from.is_empty() {
        0
    } else {
        req.account
    }
}

/// 결제 요청 처리 결과. GUI가 쓰고 MCP가 읽는다.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct PaymentResult {
    pub(crate) id: String,
    /// "approved" | "rejected" | "failed".
    pub(crate) status: String,
    /// approved + transfer 일 때 tx 해시, 아니면 "".
    pub(crate) tx_hash: String,
    /// rejected/failed 사유.
    pub(crate) detail: String,
    /// x402 승인일 때 부분 서명한 트랜잭션(MCP 가 PAYMENT-SIGNATURE 헤더로 조립). 그 밖엔 None.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) x402: Option<SvmSigned>,
}

/// GUI 생존 표시(하트비트). MCP는 이게 신선해야만 요청을 띄운다(앱 꺼져 있으면 5분 안 기다림).
#[derive(Serialize, Deserialize)]
struct Heartbeat {
    ts: u64,
    /// **승인 창을 실제로 띄울 수 있는 상태인가**. 러스트 스레드는 프로세스가 살아
    /// 있는 한 계속 하트비트를 찍으므로, WebView 만 죽으면 「앱은 살아 있다」고 말하면서
    /// 승인 창은 영영 안 뜬다 → MCP 가 요청을 받아 두고 5분을 조용히 기다린다.
    /// 창을 여러 번 깨워도 폴링이 안 돌아오면 이 값을 false 로 내려 MCP 가 **즉시 정직하게**
    /// 거절하게 한다. 기본 true = 이 필드가 없던 옛 파일과의 호환.
    #[serde(default = "ui_ok_default")]
    ui_ok: bool,
    /// 🔴 **이 앱이 처리할 수 있는 결제 방식들**. MCP 가 요청을 쓰기 전에 본다.
    ///
    /// 왜 필요한가: MCP 사이드카는 앱 번들 안에 함께 실려 보통 앱과 같은 버전이지만, 소스 빌드로
    /// 등록했거나 앱만 옛것인 조합이 있을 수 있다. 그때 **새 MCP + 옛 앱**이면 옛 앱은
    /// 새 방식의 요청을 엉뚱하게 처리할 수 있다. 그래서 앱은 모르는 kind 를 거절하고, MCP 는
    /// **여기에 없는 방식이면 요청 자체를 안 쓴다**. 필드가 없으면 아무것도 못 하는 것으로 본다(닫힌 쪽).
    #[serde(default = "kinds_default")]
    kinds: Vec<String>,
}

fn ui_ok_default() -> bool {
    true
}

/// 이 필드가 없는 하트비트 — 아무 방식도 못 한다고 본다(정본 `policy::kinds_legacy`, 빈 목록).
fn kinds_default() -> Vec<String> {
    crate::policy::kinds_legacy()
}

/// 지금 이 앱이 처리하는 결제 방식 전부. `approve_payment`·`auto_approve_payment` 의 match 팔과
/// **같이 움직여야 한다** — 하나를 늘리면 여기도 늘린다(안 늘리면 MCP 가 새 방식을 안 보낸다).
fn supported_kinds() -> Vec<String> {
    crate::policy::kinds_supported()
}

fn request_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("payment_request.json"))
}

fn result_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("payment_result.json"))
}

fn heartbeat_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("app_alive.json"))
}

/// 대기 중인 결제 요청을 읽는다 (없으면 None).
pub(crate) fn read_request() -> Option<PaymentRequest> {
    request_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
}

/// MCP 가 승인을 기다리는 시간(초). budu-mcp 의 APPROVAL_TIMEOUT 과 같은 값.
const APPROVAL_WINDOW_SECS: u64 = 300;
/// 그 뒤로도 이만큼은 "살아 있는 요청"으로 본다 — GUI 가 MCP 보다 먼저 포기해서
/// 사용자가 승인할 수 있는 요청을 못 보게 되는 일이 없도록 한 여유분.
const STALE_GRACE_SECS: u64 = 60;

/// 승인 창이 닫혔어도 소용없을 만큼 오래된(또는 시각이 말이 안 되는) 요청인가.
/// 정상 타임아웃이면 MCP 가 cancel_request 로 파일을 치우지만, MCP 프로세스가 대기 중
/// 죽으면(예: AI 앱 재시작) 파일이 그대로 남는다. 그걸 영원히 "대기 중"으로 보면
/// 승인 창이 영영 맨 위에 박혀 있다.
///
/// created 는 프로세스를 넘나드는 파일이라 벽시계일 수밖에 없다. 시계가 크게 뒤로 밀려
/// created 가 "미래"가 되면 경과가 늘 0이라 영원히 안 늙으므로, 유예를 넘는 미래 시각도
/// 만료로 본다(잠깐의 시계 오차는 유예 안에서 그대로 통과).
fn is_stale(req: &PaymentRequest) -> bool {
    let now = now_secs();
    if req.created > now.saturating_add(STALE_GRACE_SECS) {
        return true;
    }
    now.saturating_sub(req.created) > APPROVAL_WINDOW_SECS + STALE_GRACE_SECS
}

/// 이 요청의 남은 승인 시간(초) — 승인 창 카운트다운(format.ts secsLeft)과 같은 식.
/// created 가 미래면(시계 오차) 경과 0 으로 본다(is_stale 의 유예를 넘는 미래는 거기서 걸러진다).
pub(crate) fn secs_left(req: &PaymentRequest, now: u64) -> u64 {
    APPROVAL_WINDOW_SECS.saturating_sub(now.saturating_sub(req.created))
}

/// 지금 사람 승인을 기다리는 살아 있는 요청 (없거나 만료면 None).
/// 승인 창의 항상 위 고정(appwin)과 프론트 승인 모달이 **같은 이 값**에서 나와야
/// "모달은 떠 있는데 게이트는 꺼진" 불일치가 생기지 않는다.
pub(crate) fn live_request() -> Option<PaymentRequest> {
    read_request().filter(|r| !is_stale(r) && !lease_lapsed(r))
}

/// 기다리는 쪽(MCP)이 임대 갱신을 멈췄나 — 멈췄으면(30초) 그 요청은 보여주지도 승인하지도 않는다.
/// 여기선 파일을 지우지 않는다: 잠깐 끊겼다 다시 갱신되면 되살아나야 한다. 2분 넘게 끊기면(`lease_dead`)
/// 감시 스레드가 치운다 — 맥이 잠들었다 깬 직후는 MCP 가 갱신할 틈을 주려고 10초 기다린다(`watchdog::settled`).
fn lease_lapsed(req: &PaymentRequest) -> bool {
    crate::policy::lease_lapsed(req.lease, request_age())
}

/// 요청 파일 수정 시각으로부터 지난 초 — 임대 갱신이 마지막으로 닿은 때. 못 읽으면 None.
fn request_age() -> Option<u64> {
    request_path()
        .ok()
        .and_then(|p| fs::metadata(p).ok())
        .and_then(|m| m.modified().ok())
        .and_then(|t| std::time::SystemTime::now().duration_since(t).ok())
        .map(|d| d.as_secs())
}

/// 임대가 **오래** 끊겼나(임대 시간의 4배 = 2분) — 기다리던 MCP 가 죽었다고 볼 만큼.
/// 잠깐 끊긴 것(노트북 잠자기)은 `lease_lapsed` 로 숨기기만 하고, 이만큼 끊기면 감시 스레드가 파일을 치운다 —
/// 안 치우면 MCP 의 한 건 가드(`has_pending`)가 5분+유예 동안 새 결제를 전부 «이미 대기 중» 으로 거절하는데
/// 사람 화면엔 그 요청이 안 보여 처리할 길이 없다. 임대가 끊긴 요청은 승인도 안 되므로 지워도 나간 돈은 없다.
pub(crate) fn lease_dead(req: &PaymentRequest) -> bool {
    req.lease && request_age().is_some_and(|a| a > crate::policy::REQUEST_LEASE_SECS * 4)
}

/// 처리 결과를 기록하고 대기 요청을 치운다 (single-flight 해제).
///
/// 🔴 **결과 파일도 요청 파일도, 「이 결과의 요청」이 아직 대기 중일 때만 건드린다**
///. 결과 파일은 **한 칸뿐**이라, 늦게 끝난 승인 A 가 그 뒤
/// 승인된 B 의 결과를 덮으면 **B 를 기다리던 쪽은 돈이 나갔는데도 시간 초과**로 보고
/// 재시도한다(이중 결제). 요청 파일 쪽도 같은 이유로 남의 것을 치우면 안 된다.
///
/// 내 요청이 이미 사라졌다면(상대가 시간 초과로 거둬감) **조용히 아무것도 안 쓴다** —
/// 그 결과를 기다리는 쪽은 이미 없고, 결과를 남기면 다음 사람의 답을 가린다.
/// 사람에게는 어차피 호출자(승인 창)가 반환값으로 결과를 보여준다.
pub(crate) fn resolve_request(result: &PaymentResult) -> Result<(), String> {
    let pending = read_request().map(|r| r.id);
    if !owns_pending(pending.as_deref(), &result.id) {
        return Ok(());
    }
    write_json(result_path()?, result)?;
    // 확인과 삭제를 한 걸음으로 — 위에서 본 뒤 MCP 가 거두고 새 요청이 들어왔으면 그건 안 지운다.
    if let Ok(p) = request_path() {
        crate::policy::remove_request_if_mine(&p, &result.id);
    }
    Ok(())
}

/// 이 결과가 「지금 대기 중인 그 요청」의 것인가 (순수 — 테스트용).
/// 대기 요청이 없으면 상대는 이미 떠난 뒤다. **다른 id** 면 남의 요청이므로 절대 건드리지 않는다.
fn owns_pending(pending_id: Option<&str>, result_id: &str) -> bool {
    pending_id == Some(result_id)
}

/// 지금 **승인 처리가 진행 중인** 결제 수. 0 이 아니면 서명·전송이 체인으로 나가는 중일 수 있다.
/// 개발 63 부터 `begin_approval` 이 겹침을 거절하므로 실제 값은 0 아니면 1 이다 — 카운터로 둔 건
/// 가드가 `?` 로 빠져나가도 Drop 이 정확히 하나만 되돌리게 하는 모양이 그대로 맞기 때문이다.
static APPROVALS_IN_FLIGHT: AtomicU64 = AtomicU64::new(0);

/// 🔴 **승인 시작과 감시 스레드의 실패 처리를 한 줄로 세우는 잠금**.
///
/// 카운터를 각자 읽고 각자 행동하면 원자적이지 않다: 감시 스레드가 「진행 중 아님」을 본
/// **직후** 승인이 시작되면, 승인은 체인으로 돈을 보내는데 감시 스레드는 같은 요청을 실패로
/// 적고 치운다 → 상대는 실패로 알고 재시도한다(이중 결제). 두 쪽이 이 잠금 안에서
/// **「요청이 아직 대기 중인가」를 보고 상태를 바꾸므로** 순서가 하나로 정해진다.
static APPROVAL_ARBITER: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 승인 처리 구간을 표시하는 RAII 가드. 중간에 `?` 로 빠져나가도 Drop 이 카운터를 되돌린다.
pub(crate) struct ApprovalGuard;

impl ApprovalGuard {
    /// 카운터만 올린다. **`begin_approval` 안에서만 쓴다** — 밖에서 쓰면 「요청이 아직 대기
    /// 중인가」 확인과 잠금을 건너뛰어 4차 P1 의 경합이 되살아난다.
    fn acquire() -> Self {
        APPROVALS_IN_FLIGHT.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for ApprovalGuard {
    fn drop(&mut self) {
        APPROVALS_IN_FLIGHT.fetch_sub(1, Ordering::SeqCst);
    }
}

/// 이 요청의 승인 처리를 시작한다. 두 가지를 거절한다.
///
/// 1. **감시 스레드가 이미 실패로 끝냈거나 상대가 거둬간 요청** — 그 경우 상대는 이미 실패를
///    받아 재시도했을 수 있으므로, 여기서 또 보내면 이중 결제다.
/// 2. 🔴 **다른 승인이 아직 진행 중**.
///    승인 처리는 한 번에 하나다. 예전엔 같은 요청에 `approve_payment` 가 겹쳐 들어오면(자율 시도와
///    수동 승인, 혹은 버튼 연타) 둘 다 여기를 통과해 **둘 다 체인으로 나갈 수 있었다** — 프론트가
///    버튼을 잠그고 자율 처리 중엔 모달을 안 띄우지만, 돈의 경계는 프론트가 아니라 여기다.
///    그리고 이 거절이 있어야 MCP 쪽 「대기 중 AI 가 죽으면 요청을 거둔다」(payment.rs
///    `CancelOnDrop`)가 안전하다: A 의 전송 중에 요청 파일이 사라져 새 요청 B 가 생겨도, A 가
///    끝날 때까지 B 의 승인은 여기서 거절된다.
///
/// ⚠️ **이 거절이 닫는 것은 「겹치는 창」뿐이다.** A 가 끝나 카운터가 내려온 뒤에 오는 B
/// (다시 켜진 AI 가 「아까 실패했다」고 알고 재요청한 것)는 여기서 못 막는다 — 그건 순차
/// 재시도이고, 막는 것은 한도·신뢰 주소·사람이 보는 승인 창이다. 자율 승인이 켜져 있고
/// 한도 안이면 B 도 조용히 나간다. 그 갈래는 이 함수가 아니라 「같은 결제를 짧은 시간에
/// 두 번」을 보는 규칙이 필요하다.
///
/// 거절된 쪽이 자율 경로면 프론트가 사람 승인 모달로 넘기고, 사람이면 오류를 보고 잠시 뒤
/// 다시 누른다 — 어느 쪽도 돈이 나가지 않는다.
pub(crate) fn begin_approval(req: &PaymentRequest) -> Result<ApprovalGuard, String> {
    let _lock = APPROVAL_ARBITER.lock().map_err(|_| {
        ts!(
            "승인을 시작할 수 없어요. 앱을 다시 시작해 주세요.",
            "Couldn't start the approval. Please restart the app."
        )
    })?;
    // 겹침 거절을 **기록을 건드리기 전에** 본다 — 진행 중인 승인의 기록을 이 시도가 덮었다가
    // 되돌리면, 그 사이 진행 중인 쪽이 적은 결말(done)을 옛 값(sending)으로 되돌려 쓰게 된다.
    if approval_in_flight() {
        return Err(in_flight_message());
    }
    // 🔴 **결제 시도 기록을 먼저 쓰고, 그다음 요청이 아직 있는지 본다**.
    // MCP 는 시간 초과 때 요청을 먼저 지우고 그다음 기록을 읽는다(shared/policy.rs 순서 규약) — 이 순서가
    // 둘 다 지켜져야 「GUI 는 요청을 보고 전송을 시작했는데 MCP 는 아무것도 못 보고 『응답 없음』이라
    // 답한다」가 불가능해진다. 두 프로세스 사이엔 잠금이 없어서, 닫는 것은 순서뿐이다.
    // 기록이 있는데 못 읽으면 다시 승인하지 않는다 — 깨진 게 `sending`·`done` 이었으면 두 번째 결제다.
    let Ok(prev) = read_attempt(&req.id) else {
        return Err(ts!(
            "이 결제의 처리 기록을 읽지 못했어요. 이미 나갔을 수 있어 다시 승인하지 않습니다 — 내역을 확인하세요.",
            "Couldn't read this payment's processing record. It may already have gone out, so it won't be approved again — check the history."
        )
        .into());
    };
    if !crate::policy::attempt_allows_retry(prev.as_ref()) {
        // 이미 나갔거나(done·unknown) 앱이 전송 중에 죽은(sending) 요청 — 다시 승인하면 두 번째 결제다.
        return Err(ts!(
            "이 결제는 이미 처리됐거나, 나갔는지 확인하지 못한 상태예요. 다시 승인하지 않습니다 — 내역을 확인하세요.",
            "This payment was already processed, or its outcome couldn't be confirmed. It won't be approved again — check the history."
        )
        .into());
    }
    let now = now_secs();
    write_attempt(&crate::policy::AttemptRecord {
        v: 1,
        id: req.id.clone(),
        state: crate::policy::ATTEMPT_SENDING.into(),
        kind: req.kind.clone(),
        chain_id: req.chain_id,
        started: now,
        updated: now,
        status: String::new(),
        tx_hash: String::new(),
        detail: String::new(),
        x402: None,
    })?; // 기록을 못 남기면 시작하지 않는다 — MCP 가 시간 초과 뒤 이 승인을 못 보게 된다.
         // 임대가 끊긴 요청은 「없는 요청」과 같다 — 기다리는 AI 가 없다. 파일에서 다시 읽은 값으로 본다.
    let pending = read_request().filter(|r| !lease_lapsed(r)).map(|r| r.id);
    if let Err(e) = admit_approval(pending.as_deref(), &req.id, false) {
        // 요청이 그새 사라졌다(상대가 거둬감) — 우리가 쓴 기록을 되돌린다. MCP 가 그 사이 `sending` 을
        // 봤다면 곧 사라지는 걸 보고 「아무것도 안 나감」으로 끝낸다.
        restore_attempt(&req.id, prev.as_ref());
        return Err(e);
    }
    Ok(ApprovalGuard::acquire())
}

/// 겹침 거절 문구.
fn in_flight_message() -> String {
    // 🔴 이 거절은 예전엔 **앱이 살아 있는 동안 안 풀릴 수도** 있었다(전송에 시간 상한이 없었다).
    // 개발 66 부터 전송은 상한이 있고 넘기면 「불명」으로 끝나므로 곧 풀린다 — 그래도 문구는 남긴다.
    ts!(
        "결제를 아직 처리하는 중이에요. 끝난 뒤 다시 시도하세요. 한참 이대로면 앱을 다시 시작해 주세요.",
        "A payment is still being processed. Try again once it finishes — and if it stays this way, restart the app."
    )
    .into()
}

/// 승인 처리를 끝낸다 — 결제 시도 기록에 결말을 적고, 요청이 아직 대기 중이면 결과 파일을 쓴다.
///
/// **기록을 먼저 쓴다.** MCP 가 시간 초과로 요청을 거둔 뒤라면 `resolve_request` 는 조용히 아무것도 안
/// 쓰는데(남의 결과를 덮지 않으려고), 그때 MCP 가 결말을 알 수 있는 곳은 이 기록뿐이다.
///   Ok(approved) → done  ·  Ok(unknown) → unknown(요청은 소비한다 — 다시 승인하면 두 번째 결제)
///   Err          → failed(요청은 남는다 — 확실히 안 나갔으니 다시 승인·거부할 수 있다)
///   Err(NEEDS_PASSWORD) → handed(자율이 사람에게 넘김 — 시작한 것 없음)
pub(crate) fn finish_approval(
    req: &PaymentRequest,
    outcome: Result<PaymentResult, String>,
) -> Result<PaymentResult, String> {
    let started = read_attempt(&req.id)
        .ok()
        .flatten()
        .map(|r| r.started)
        .unwrap_or_else(now_secs);
    let mut rec = crate::policy::AttemptRecord {
        v: 1,
        id: req.id.clone(),
        state: String::new(),
        kind: req.kind.clone(),
        chain_id: req.chain_id,
        started,
        updated: now_secs(),
        status: String::new(),
        tx_hash: String::new(),
        detail: String::new(),
        x402: None,
    };
    match outcome {
        Ok(r) => {
            rec.state = if r.status == "unknown" {
                crate::policy::ATTEMPT_UNKNOWN
            } else {
                crate::policy::ATTEMPT_DONE
            }
            .into();
            rec.status = r.status.clone();
            rec.tx_hash = r.tx_hash.clone();
            rec.detail = r.detail.clone();
            rec.x402 = r.x402.as_ref().and_then(|p| serde_json::to_value(p).ok());
            let _ = write_attempt(&rec);
            // 결과 파일만 못 썼어도 결제는 끝났다 — 오류로 돌리면 화면이 「실패」로 보인다.
            // MCP 는 결과가 안 오면 시간 초과 뒤 위 기록을 읽고, 남은 요청의 거부·감시 실패는 `settled_attempt_result` 가 막는다.
            if let Err(e) = resolve_request(&r) {
                eprintln!("[ipc] 결과 파일을 못 씀: {e}");
            }
            Ok(r)
        }
        // 자율이 사람에게 넘긴 것(NEEDS_PASSWORD)은 실패가 아니다 — 「넘김」으로 적는다.
        Err(e) if e == crate::session::NEEDS_PASSWORD => {
            rec.state = crate::policy::ATTEMPT_HANDED.into();
            let _ = write_attempt(&rec);
            Err(e)
        }
        Err(e) => {
            rec.state = crate::policy::ATTEMPT_FAILED.into();
            rec.status = "failed".into();
            rec.detail = e.clone();
            let _ = write_attempt(&rec);
            Err(e)
        }
    }
}

/// 거부·「실패」로 덮어도 되는 요청인가. 결과 파일만 못 쓴 승인 뒤엔 요청이 남는데, 그 칸에 거부나
/// 감시 스레드의 실패가 들어가면 **나간 돈이 「안 나감」으로** 읽혔다. 시도 기록에 결말(done·unknown)이 있으면 그 결과를
/// 돌려준다(그걸 다시 전한다). 덮어도 되면 `Ok(None)`, 나가는 중이었거나 기록을 못 읽으면 `Err` — 아무것도 쓰지 않는다.
fn settled_attempt_result(id: &str) -> Result<Option<PaymentResult>, ()> {
    let Some(rec) = read_attempt(id)? else {
        return Ok(None);
    };
    if crate::policy::attempt_allows_retry(Some(&rec)) {
        return Ok(None);
    }
    if rec.state != crate::policy::ATTEMPT_DONE && rec.state != crate::policy::ATTEMPT_UNKNOWN {
        return Err(());
    }
    Ok(Some(PaymentResult {
        id: rec.id,
        status: rec.status,
        tx_hash: rec.tx_hash,
        detail: rec.detail,
        x402: rec.x402.and_then(|v| serde_json::from_value(v).ok()),
    }))
}

/// 화면이 죽어 승인 창을 못 띄울 때 기다리는 요청에 줄 답.
/// 이미 끝난 결제는 실패가 아니라 그 결과를 다시 전하고(`false`), 나가는 중이었거나 기록을 못 읽으면 손대지 않는다(`None`).
/// 아무것도 시작 안 한 요청만 「실패」(`true` — 사람에게도 알린다).
fn dead_window_result(id: String) -> Option<(PaymentResult, bool)> {
    match settled_attempt_result(&id) {
        Ok(Some(r)) => Some((r, false)),
        Err(()) => None,
        Ok(None) => Some((
            PaymentResult {
                id,
                status: "failed".into(),
                tx_hash: String::new(),
                detail: ts!(
                    "지갑 앱 화면이 응답하지 않아 승인 창을 띄우지 못했어요. 앱을 다시 시작한 뒤 다시 요청하세요.",
                    "The wallet app's window isn't responding, so the approval prompt couldn't be shown. Restart the app and try again."
                )
                .into(),
                x402: None,
            },
            true,
        )),
    }
}

/// 결제 시도 기록 — 없으면 Ok(None), 있는데 못 읽으면 Err. 홈을 못 정해도 Err.
fn read_attempt(id: &str) -> Result<Option<crate::policy::AttemptRecord>, ()> {
    crate::policy::read_attempt_at(&budu_dir().map_err(|_| ())?, id).map_err(|_| ())
}

fn write_attempt(rec: &crate::policy::AttemptRecord) -> Result<(), String> {
    let path = crate::policy::attempt_path(&budu_dir()?, &rec.id).ok_or(ts!(
        "결제 요청 번호가 올바르지 않아요.",
        "That payment request id isn't valid."
    ))?;
    write_json(path, rec)
}

/// 시작하지 못한 승인이 남긴 기록을 되돌린다 — 이전 기록이 있었으면 그대로, 없었으면 지운다.
fn restore_attempt(id: &str, prev: Option<&crate::policy::AttemptRecord>) {
    match prev {
        Some(r) => {
            let _ = write_attempt(r);
        }
        None => {
            if let Some(p) = budu_dir()
                .ok()
                .and_then(|d| crate::policy::attempt_path(&d, id))
            {
                let _ = fs::remove_file(p);
            }
        }
    }
}

/// 오래된 결제 시도 기록을 지운다(감시 스레드가 한 시간에 한 번). 청소일 뿐이라 실패는 조용히 넘긴다.
fn prune_attempts() {
    let Some(dir) = budu_dir()
        .ok()
        .map(|d| d.join(crate::policy::APPROVALS_DIR))
    else {
        return;
    };
    let Ok(entries) = fs::read_dir(&dir) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        let age = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d.as_secs());
        if age.is_some_and(crate::policy::attempt_prunable) {
            let _ = fs::remove_file(e.path());
        }
    }
}

/// `begin_approval` 의 판정만 떼어 낸 순수 함수 (테스트용). **잠금 안에서, 카운터를 올리기 전에**
/// 부른다 — 여기서 Ok 가 나온 뒤 카운터를 올리는 사이에 다른 승인이 끼어들 수 없는 건
/// `APPROVAL_ARBITER` 덕분이다.
fn admit_approval(pending_id: Option<&str>, id: &str, in_flight: bool) -> Result<(), String> {
    if pending_id != Some(id) {
        return Err(ts!(
            "결제 요청이 이미 취소됐거나 시간이 지났어요.",
            "That payment request was already cancelled or timed out."
        )
        .into());
    }
    if in_flight {
        return Err(in_flight_message());
    }
    Ok(())
}

pub(crate) fn approval_in_flight() -> bool {
    APPROVALS_IN_FLIGHT.load(Ordering::SeqCst) > 0
}

mod watchdog;
pub(crate) use watchdog::spawn_watchdog;
use watchdog::{refuse_over_limit, LAST_POLL};

/// GUI가 1초마다 폴링한다. 대기 요청을 돌려준다.
///
/// **하트비트는 여기서 찍지 않는다**. WebView 의 타이머는 창이 숨은 채 5분이 지나면
/// 멎어서 하트비트가 같이 멎었고,
/// 그동안 MCP 가 결제를 「앱이 실행 중이 아니에요」로 거절했다 — 앱은 멀쩡히 떠 있는데도.
/// 지금은 러스트 스레드(watchdog)가 찍고, 이 커맨드는 **프론트가 아직 깨어 있다는 표식**만
/// 남긴다. watchdog 이 그 표식을 보고 "WebView 가 잠들었는지"를 판단한다.
///
/// **표식은 승인 창(`appwin::APPROVER` = 큰 창)이 부를 때만 남긴다**. 팝오버도
/// 「승인하러 가기」 띠 때문에 이 커맨드를 부르는데, 팝오버는 승인을 안 한다 — 그 폴링이 표식을 갱신하면 큰 창
/// 웹뷰가 잠든 동안에도 watchdog 이 「프론트가 깨어 있다」고 보고 큰 창을 안 깨워, 결제가 아무도 안 보는 채로
/// 5분을 흘려보낸다. 고정·창 전 한도 검사도 승인 창의 폴링에서만.
#[tauri::command]
pub(crate) fn get_pending_request(
    app: tauri::AppHandle,
    window: tauri::WebviewWindow,
) -> Option<PaymentRequest> {
    if window.label() != crate::appwin::APPROVER {
        return live_request();
    }
    LAST_POLL.store(now_secs(), Ordering::Relaxed);
    // 한도를 넘는 요청은 모달이 뜨기 전에 돌려보낸다. 자율 결제도 이 폴링이 고르므로 같이 막힌다.
    refuse_over_limit();
    // 파일은 한 번만 읽고 그 값으로 둘 다 정한다 — 두 번 읽으면 그 사이에 요청이
    // 생기거나 사라질 때 한 폴링 안에서 "모달은 뜨는데 고정은 꺼진" 상태가 나온다.
    let pending = live_request();
    // 승인 대기 여부에 맞춰 "항상 위" 고정을 매 초 다시 맞춘다.
    // raise/release 커맨드는 프론트에서 비동기로 날아와 도착 순서가 뒤집힐 수 있는데,
    // 이 폴링이 상태에서 다시 파생시키므로 어긋나도 1초 안에 스스로 고쳐진다.
    crate::appwin::set_pinned(&app, crate::appwin::should_pin(&app, pending.as_ref()));
    pending
}

/// 결제 요청을 승인한다 — 비번으로 키를 열어 x402 결제에 서명하고 결과를 기록한다.
/// `sign_x402_payment` 를 거치므로 긴급잠금·한도·거래내역이 자동 적용된다.
/// 실패(비번 오류·잠금·한도 등) 시엔 요청을 치우지 않는다 → 팝업에서 재시도하거나 거부할 수 있다.
#[tauri::command]
pub(crate) async fn approve_payment(
    id: String,
    password: String,
    job_budget: Option<String>,
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
    ensure_request_chain(&req)?; // 요청 시점 체인 ≠ 현재 체인이면 거부(메인넷 오발사 차단)
    ensure_request_account(&req)?; // 요청 시점 계정 ≠ 현재 계정이면 거부
    // 작업 예산은 **서명 전에** 검사한다 — 틀린 예산이면 아무것도 서명하지 않는다.
    let budget = job_budget
        .as_deref()
        .map(|b| crate::job::check_budget(&req.job, b, &req.amount))
        .transpose()?;

    // 🔴 **여기서부터 계정을 못박는다**. 위 검사는 *그 순간*만 본다 — 아래 비번 검증·
    // RPC 왕복 사이에 사용자가 계정을 바꾸면 `unlock_signer` 가 **그때 활성인 계정의 키**로
    // 서명하고 내역도 그 계정 파일에 적힌다. 요청의 계정으로 작업 전체를 묶으면 그 창이 사라진다.
    // 🔴 **체인도 여기서 못박는다**. 예전엔 「안쪽 do_* 가 진입 시 고정한다」에 맡겼는데,
    // 위 검사와 그 고정 사이에 비번 복호화가 끼어 있어 그 틈에 설정에서 체인을 바꾸면 do_* 가 **바뀐**
    // 체인을 고정했다 — 연습용으로 만든 요청이 메인넷에서 서명될 수 있었다.
    // 자율 경로(session.rs)와 같은 모양: 요청의 체인으로 작업 전체를 묶는다(0 = 옛 미각인 요청 → 지금 체인).
    let chain = if req.chain_id != 0 {
        req.chain_id
    } else {
        crate::chain::active_chain().chain_id
    };
    let pinned = request_account_index(&req);
    crate::chain::with_pinned_chain(
        chain,
        crate::wallet::with_pinned_account(pinned, approve_pinned(req, password, budget)),
    )
    .await
}

/// 계정이 고정된 채로 도는 승인 본체 — 서명 키·내역 파일이 모두 요청의 계정을 본다.
async fn approve_pinned(
    req: PaymentRequest,
    password: String,
    budget: Option<u64>,
) -> Result<PaymentResult, String> {
    // 여기서부터는 돈이 나갈 수 있는 구간 — 감시 스레드가 이 요청을 실패로 끝내면 안 된다.
    // (감시 스레드가 이미 끝낸 뒤면 여기서 거절된다 — 그 경우 상대는 재시도했을 수 있다.)
    let _in_flight = begin_approval(&req)?;
    let outcome = approve_kind(&req, password, budget.is_some()).await;
    // 작업 예산 — 서명이 됐을 때만 연다/더한다. 키는 메모리에만(`job.rs`).
    let outcome = outcome.map(|(r, phrase)| {
        if r.status == "approved" {
            crate::job::after_human_approval(
                &req.job,
                crate::chain::active_chain().chain_id,
                &req.from,
                &req.to,
                &req.amount,
                r.x402.as_ref().map(|p| p.signature.as_str()).unwrap_or_default(),
                budget,
                phrase,
            );
        }
        r
    });
    finish_approval(&req, outcome)
}

/// kind 에 따라 처리한다. 둘 다 잠금·한도·내역을 자동 적용한다(같은 하부 함수 재사용).
/// `Err` = 확실히 안 나감(요청 유지 → 팝업에서 재시도/거부 가능). 불명은 `Ok(status: "unknown")`.
async fn approve_kind(
    req: &PaymentRequest,
    password: String,
    keep_key: bool,
) -> Result<(PaymentResult, Option<zeroize::Zeroizing<String>>), String> {
    use crate::policy::KIND_X402;
    match req.kind.as_str() {
        // x402 솔라나: 트랜잭션을 조립해 우리 몫만 부분 서명한다(페이실리테이터가 수수료 서명을 더해 정산).
        KIND_X402 => {
            // x402 서명은 언제나 USDC 다 — 다른 토큰 표시로 온 요청을 화면만 믿고 서명하지 않는다
            // (자율·작업 예산 경로는 이미 USDC 만 받는다).
            if req.token != "USDC" {
                return Err(ts!(
                    "USDC 가 아닌 결제 요청이라 서명하지 않았어요. 아무것도 결제하지 않았습니다.",
                    "This request isn't in USDC, so nothing was signed or paid."
                )
                .into());
            }
            let terms = req.svm.clone().ok_or(ts!(
                "결제 조건이 비어 있어 서명하지 않았어요. 아무것도 결제하지 않았습니다.",
                "The payment terms are missing, so nothing was signed or paid."
            ))?;
            let (payment, phrase) = sign_x402_payment(
                password,
                req.to.clone(),
                req.amount.clone(),
                terms,
                req.resource.clone(),
                req.job.clone(),
                keep_key,
            )
            .await?;
            Ok((
                PaymentResult {
                    id: req.id.clone(),
                    status: "approved".into(),
                    tx_hash: String::new(),
                    detail: String::new(),
                    x402: Some(payment),
                },
                phrase,
            ))
        }
        // 🔴 모르는 kind 는 거절하고 이유를 말한다 — 송금 등으로 떨어뜨리지 않는다.
        other => Err(tf!(
            "이 앱이 모르는 결제 방식이에요({other}). 지갑 앱을 최신 버전으로 업데이트해 주세요.",
            "This app doesn't know that payment kind ({other}). Please update the wallet app."
        )),
    }
}

/// 결제 요청을 거부한다 — MCP에 "거부됨"을 알리고 대기 요청을 치운다.
///
/// 🔴 **진행 중인 승인이 있으면 거부도 받지 않는다**. 거부는 요청 파일을 치우고
/// 결과 칸에 「거부됨」을 쓴다 — 그게 전송 중인 승인과 겹치면 **돈은 나갔는데 상대는
/// 「거부됨」으로 읽고**, 그 뒤 도착한 진짜 결과는 `resolve_request` 가 조용히 버린다.
/// 지금은 프론트가 이 상황을 안 만들지만(승인 중 버튼 잠금·자율 처리 중 모달 없음),
/// 돈의 경계는 프론트가 아니라 여기다 — 승인과 같은 문을 쓰게 한다.
#[tauri::command]
pub(crate) fn reject_payment(id: String, reason: Option<String>) -> Result<(), String> {
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
    // 🔴 판정과 행동을 **한 잠금 안에서** 한다. 보고 나서 놓으면 그 틈에
    // `begin_approval` 이 승인을 시작해, 아래 `resolve_request` 가 「거부됨」을 쓰고 요청을
    // 치운 뒤 **진짜 결제가 체인으로 나간다** — 상대는 거부로 읽고 재시도한다. 감시 스레드가
    // 실패를 적을 때 쓰는 것과 같은 모양이다. `resolve_request` 는 이 잠금을 안 잡으므로
    // 겹쳐 잠길 일이 없다.
    let _lock = APPROVAL_ARBITER.lock().map_err(|_| {
        ts!(
            "거부를 처리할 수 없어요. 앱을 다시 시작해 주세요.",
            "Couldn't process the rejection. Please restart the app."
        )
    })?;
    if approval_in_flight() {
        return Err(ts!(
            "결제를 처리하는 중이라 지금은 거부할 수 없어요. 한참 이대로면 앱을 다시 시작해 주세요.",
            "That payment is being processed right now, so it can't be rejected — and if it stays this way, restart the app."
        )
        .into());
    }
    match settled_attempt_result(&req.id) {
        Ok(None) => {}
        Ok(Some(r)) => {
            resolve_request(&r)?;
            return Err(ts!(
                "이 결제는 이미 처리돼서 거부할 수 없어요. 처리 결과를 AI 에게 다시 전했어요 — 내역을 확인하세요.",
                "This payment was already processed, so it can't be rejected. Its result was passed to the AI again — check the history."
            )
            .into());
        }
        Err(()) => {
            return Err(ts!(
                "이 결제는 나갔는지 확인하지 못한 상태라 거부할 수 없어요 — 내역을 확인하세요.",
                "This payment's outcome couldn't be confirmed, so it can't be rejected — check the history."
            )
            .into());
        }
    }
    resolve_request(&PaymentResult {
        id: req.id,
        status: "rejected".into(),
        tx_hash: String::new(),
        detail: reason
            .unwrap_or_else(|| ts!("사용자가 거부했습니다", "Rejected by the user").into()),
        x402: None,
    })
}

/// MCP 서버(=AI 클라이언트)의 생존 하트비트. MCP가 살아있는 동안 주기적으로 쓴다.
#[derive(Deserialize)]
struct McpAlive {
    ts: u64,
    #[serde(default)]
    client: String,
}

/// 프론트로 주는 AI 연결 상태 — 메인 화면 "연결됨" 배지용.
#[derive(Serialize)]
pub(crate) struct AgentStatus {
    connected: bool,
    /// 연결한 클라이언트 이름(예: "claude-code"). 모르면 빈 문자열.
    client: String,
}

fn mcp_alive_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("mcp_alive.json"))
}

/// AI 에이전트(MCP)가 지금 이 지갑에 연결돼 있는지 알려준다.
/// MCP가 5초마다 하트비트를 쓰므로, 15초 이내면 "연결됨"으로 본다(3회 여유).
#[tauri::command]
pub(crate) fn get_agent_status() -> AgentStatus {
    let alive = mcp_alive_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str::<McpAlive>(&s).ok());
    match alive {
        Some(a) if now_secs().saturating_sub(a.ts) <= 15 => AgentStatus {
            connected: true,
            client: a.client,
        },
        _ => AgentStatus {
            connected: false,
            client: String::new(),
        },
    }
}

#[cfg(test)]
pub(crate) mod tests;

/// 🔴 승인 future 크기 상한(실측 크래시 — 디버그에서 215KB 가 tokio 작업 스레드 스택을 넘쳐 승인 직후 앱이 죽었다).
/// 체인·계정 고정 래퍼가 안쪽을 힙에 올리지 않게 되돌리면 여기서 걸린다.
#[cfg(test)]
mod future_size_guard {
    #[test]
    fn approval_futures_stay_small() {
        let f = super::approve_payment(String::new(), String::new(), None);
        let n = std::mem::size_of_val(&f);
        assert!(n < 16 * 1024, "approve_payment future = {n} bytes");
    }

    // 남은 비동기 경로 — 10초마다 도는 정산 확인, 잔액 조회.
    // (자율 승인 `auto_approve_payment` 는 tauri::State 를 받아 테스트에서 못 만든다 — 본체 `job_approve`·`job_work` 는
    //  session.rs 가 재고, 체인·계정 래퍼가 박싱하므로 나머지는 작다.)
    #[test]
    fn background_futures_stay_small() {
        let n = std::mem::size_of_val(&crate::confirm::tick());
        assert!(n < 16 * 1024, "confirm::tick future = {n} bytes");
        let n = std::mem::size_of_val(&crate::balance::get_balances(String::new()));
        assert!(n < 16 * 1024, "get_balances future = {n} bytes");
    }
}
