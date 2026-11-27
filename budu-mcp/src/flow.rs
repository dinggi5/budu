// 어댑터 공통 결제 흐름 — MCP 서버(main.rs)가 쓰는 x402 결제 로직.
//
// 결제·x402 의 보안 민감한 부분(리다이렉트 가드·single-flight·정산 게이팅·승인 대기)을 여기 한 곳에
// 모은다. 두 어댑터가 각자 구현하면 한쪽만 고쳐져 보안이 갈라질 위험이 있다 → 하나의 진실.
//
// 비번은 절대 여기로 들어오지 않는다. 이 모듈은 GUI 에 "요청"만 하고, 서명·전송은 GUI 가
// 사람 승인을 받아 수행한다(payment.rs 의 파일 IPC). 한도·긴급잠금·화이트리스트도 GUI 가 강제한다.

use crate::chain::active_chain;
use crate::fetch::{self, FetchRequest, FetchedBody};
use crate::{payment, x402};
use crate::{tf, ts};

/// 402 응답 본문 상한(바이트) — 외부 서버가 거대 본문으로 어댑터 메모리를 채우지 못하게 스트리밍으로 끊는다.
/// 콘텐츠 응답의 상한은 `fetch::MAX_RESPONSE_BYTES`.
const MAX_BODY_BYTES: u64 = 4 * 1024 * 1024; // 4 MiB

/// 최초 probe 용 HTTP 클라이언트 — 리다이렉트를 따라간다(결제 헤더가 없어 유출 위험 없음 —
/// "리다이렉트 후 402" 같은 정상 케이스를 깨지 않게). 요청당 30초 타임아웃.
/// **인자 없는 GET 일 때만** 쓴다 — method·headers·body 가 있으면 첫 요청도 리다이렉트를 따라가지 않는다
/// (301/302 는 POST 를 GET 으로 바꾸고 body 를 버리며, 호출자 헤더는 다른 호스트로도 따라간다).
fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| {
            tf!(
                "HTTP 클라이언트 생성 실패: {e}",
                "Couldn't create the HTTP client: {e}"
            )
        })
}

/// 결제 재요청 전용 — **리다이렉트를 따라가지 않는다**(`Policy::none`). PAYMENT-SIGNATURE/X-PAYMENT
/// (서명된 EIP-3009 인가)는 reqwest 의 cross-host 민감 헤더 제거 대상이 아니라, 리다이렉트를 따라가면
/// 다른 origin 으로 서명이 유출될 수 있다 → 402 를 낸 바로 그 최종 URL 에만 결제 헤더를 보낸다.
fn http_client_no_redirect() -> Result<reqwest::Client, String> {
    http_client_no_redirect_for(HTTP_TIMEOUT)
}

/// 요청당 시간 상한(기본).
const HTTP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(30);

/// 서명 갈래의 결제 재요청 시간 상한 — 이미지 생성처럼 응답까지 수십 초 걸리는 유료 API 가 있다.
/// 30초에 끊으면 페이실리테이터는 정산했는데 콘텐츠는 못 받는다.
const PAID_SIGNED_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

fn http_client_no_redirect_for(timeout: std::time::Duration) -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(timeout)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| {
            tf!(
                "HTTP 클라이언트 생성 실패: {e}",
                "Couldn't create the HTTP client: {e}"
            )
        })
}

/// 결제 헤더를 실어도 되는 주소인가 — https, 또는 이 컴퓨터 안의 개발 서버(localhost·루프백 IP)만.
fn payment_url_ok(u: &reqwest::Url) -> bool {
    if u.scheme() == "https" {
        return true;
    }
    if u.scheme() != "http" {
        return false;
    }
    let Some(host) = u.host_str() else {
        return false;
    };
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return ip.is_loopback();
    }
    let d = host.trim_end_matches('.').to_ascii_lowercase();
    d == "localhost" || d.ends_with(".localhost")
}

/// 승인 창을 띄울 수 없을 때의 안내. **하트비트가 없는 이유는 하나가 아니다.**
///
/// GUI 는 지갑이 아직 없으면(첫 실행·평문 마이그레이션 대기) 하트비트를 **일부러 안 찍는다**
/// (`src-tauri/src/ipc.rs` watchdog: `!needs_setup()` 일 때만 찍는다 — 승인할 사람이 없는데
/// 살아 있다고 하면 MCP 가 요청을 두고 5분을 조용히 기다린다). 그래서 `app_alive()==false` 는
/// 「앱이 꺼짐」이거나 「앱은 켜져 있는데 지갑이 없음」이다. 둘을 한 문구로 뭉치면 사용자가
/// **이미 켜져 있는 앱을 다시 켜려 한다**. 지갑 파일 상태로 갈라서 안내한다.
fn app_unavailable() -> String {
    // 앱은 떠 있는데 화면만 죽은 경우 — 켜라고 하면 안 된다(이미 켜져 있다). 다시 시작해야 한다.
    if payment::ui_stalled() {
        return ts!(
            "지갑 앱 화면이 응답하지 않아요. 앱을 완전히 종료했다 다시 켠 뒤 시도하세요.",
            "The wallet app's window isn't responding. Quit the app completely, open it again, then try."
        )
        .to_string();
    }
    match crate::wallet::wallet_status().map(|s| s.state) {
        Ok(s) if s == "none" => ts!(
            "지갑을 아직 만들지 않았어요. 지갑 앱에서 지갑을 만든 뒤 다시 시도하세요.",
            "The wallet hasn't been created yet. Create one in the wallet app, then try again."
        )
        .to_string(),
        Ok(s) if s == "legacy" => ts!(
            "지갑에 비밀번호가 아직 없어요. 지갑 앱에서 비밀번호 설정을 끝낸 뒤 다시 시도하세요.",
            "The wallet isn't password-protected yet. Finish setting a password in the wallet app, then try again."
        )
        .to_string(),
        // 지갑은 있는데 하트비트가 없다 = 앱이 꺼져 있다. 상태를 못 읽는 경우도 여기로 —
        // 「앱을 켜라」가 둘 다에 맞는 안내다(지갑 파일은 앱이 만든다).
        _ => ts!(
            "지갑 앱이 실행 중이 아니에요. 앱을 켠 뒤 다시 시도하세요.",
            "The wallet app isn't running. Open it and try again."
        )
        .to_string(),
    }
}

/// x402 리소스 가져오기 결과.
// 호출마다 하나 만들어 바로 JSON 으로 내보내는 값이라 갈래 크기 차이는 상관없다.
#[allow(clippy::large_enum_variant)]
pub enum X402Outcome {
    /// 402 가 아니었다 — 결제 불필요, 본문 그대로.
    NotPaid { http_status: u16, body: FetchedBody },
    /// 결제가 필요했지만 사용자가 승인하지 않았다(rejected | failed).
    Declined { status: String, detail: String },
    /// 🔴 **돈이 나갔을 수 있는데 콘텐츠를 못 받았다** — 서명이 서버에 닿은 뒤 연결이 끊긴 경우("unknown").
    /// 다른 reason 값은 옛 직접 제출 갈래의 것이다(부두는 그 갈래를 뺐다). 개발 3 에서 솔라나 흐름으로 다시 정한다.
    /// 이 갈래가 따로 있는 이유: 이걸 `Declined`(안 냈다)나 `Err`(진행 불가)로 뭉치면 AI 가
    /// **다시 결제한다**. tx 해시와 「다시 요청하면 또 결제된다」를 구조로 들고 나간다.
    PaidNoContent {
        tx: String,
        explorer: String,
        /// "pending"(GUI 는 받혔다는데 영수증을 못 봄) | "unknown"(GUI 도 받혔는지 모름)
        /// | "undelivered"(채굴됐는데 증거를 못 보냄) | "reverted"(체인에서 실패 — 결제액은 안 나감)
        reason: String,
        notice: String,
    },
    /// 결제하고 콘텐츠를 받았다.
    Paid {
        /// **재시도하면 안 된다**는 말이 필요할 때만 채워진다(저장 실패·응답 끊김 등).
        /// 빈 값 = 덧붙일 말 없음. 🔴 빈 값이라고 «안 나감» 이 아니다 — 정산 실패(`ok == false`)에 tx 가 없으면
        /// `payment_state` 는 UNKNOWN(나갔을 수 있음)이다. 돈의 행방은 늘 `payment_state` 로 판정한다.
        notice: String,
        /// 판매자가 정산 증빙으로 알려 온 트랜잭션(2xx 일 때만, 없으면 빈 값).
        tx: String,
        explorer: String,
        http_status: u16,
        /// 2xx(정산 성공) 여부. false 면 결제 헤더는 보냈으나 정산 단계에서 실패.
        ok: bool,
        amount: String,
        pay_to: String,
        resource: String,
        /// 정산 증빙 헤더(base64). 없으면 빈 문자열.
        settlement: String,
        /// 서명 갈래에서 **판매자가 정산 증빙에 적은** tx 와 그 탐색기 링크 — 지갑이 체인에서 확인한 값이
        /// 아니다. `tx` 에 섞지 않는 이유: `tx` 는 「우리가 올린 트랜잭션」이고 `payment_state` 가 그걸로 「나갔다」를
        /// 판정한다. 판매자 말로 그 판정이 바뀌면 안 된다.
        settlement_tx: String,
        settlement_explorer: String,
        body: FetchedBody,
    },
}

/// 돈의 행방 세 갈래. `paid` 한 비트로는 「확실히 나갔다」와 「나갔을 수 있다」가
/// 한 덩이였다 — 그래서 서명 갈래에서 서버가 402/500 을 돌려준 경우(페이실리테이터가 정산했는지 이 응답만으론
/// 모른다)를 `paid:true` 로 말할 수밖에 없었다. `false` 로 바꾸면 개발 51 의 「정직한 실패 = 이중 결제」가 된다.
pub const PAYMENT_CONFIRMED: &str = "confirmed";
/// 나갔을 수 있다 — **재시도하면 두 번 나갈 수 있다.** 사람에게 알리고 확인한다.
pub const PAYMENT_UNKNOWN: &str = "unknown";
/// 확실히 아무것도 안 나갔다 — 재시도해도 된다.
pub const PAYMENT_NONE: &str = "none";

impl X402Outcome {
    /// 이 결과에서 돈이 어떻게 됐는가 — **이 함수 하나로 정한다**(같은 규칙을 두 벌로
    /// 두면 한쪽이 뒤처진다).
    pub fn payment_state(&self) -> &'static str {
        match self {
            X402Outcome::NotPaid { .. } | X402Outcome::Declined { .. } => PAYMENT_NONE,
            X402Outcome::PaidNoContent { reason, .. } => match reason.as_str() {
                // 체인이 거절했다 — 가스만 나갔고 결제액은 그대로다.
                "reverted" => PAYMENT_NONE,
                // 영수증까지 봤다(성공) — 서버만 모른다.
                "undelivered" => PAYMENT_CONFIRMED,
                // pending·unknown·모르는 값 → 모른다. 모르는 값을 「안 나감」으로 접으면 재시도를 부른다.
                _ => PAYMENT_UNKNOWN,
            },
            // 정산 성공(2xx) 또는 직접 제출(영수증이 채굴 성공일 때만 여기까지 온다) → 나갔다.
            // 서명 갈래에서 서버가 거절했으면 페이실리테이터가 정산했는지 모른다.
            X402Outcome::Paid { ok, tx, .. } => {
                if *ok || !tx.is_empty() {
                    PAYMENT_CONFIRMED
                } else {
                    PAYMENT_UNKNOWN
                }
            }
        }
    }

    /// 예전 `paid` 필드 — **「안 나갔다고 단정할 수 없다」** 면 true(= 그냥 재시도하지 말라). 호환용으로 남긴다.
    pub fn paid(&self) -> bool {
        self.payment_state() != PAYMENT_NONE
    }
    /// AI 에 내보내는 JSON — **한 곳에서 만든다**(어댑터마다 같은 match 를 두 벌 두면 한쪽만 고쳐진다).
    pub fn to_json(&self) -> serde_json::Value {
        let paid = self.paid();
        let payment = self.payment_state();
        let mut v = match self {
            X402Outcome::NotPaid { http_status, body } => serde_json::json!({
                "paid": paid, "payment": payment, "status": "ok",
                "http_status": http_status, "body": body.text,
            }),
            X402Outcome::Declined { status, detail } => serde_json::json!({
                "paid": paid, "payment": payment,
                "status": status,   // rejected | failed
                "detail": detail,
            }),
            // 🔴 돈은 나갔거나 나갔을 수 있는데 콘텐츠가 없다(직접 제출). revert 만 「안 나감」이다.
            X402Outcome::PaidNoContent {
                tx,
                explorer,
                reason,
                notice,
            } => serde_json::json!({
                "paid": paid, "payment": payment,
                "status": reason,   // pending | unknown | undelivered | reverted
                "tx": tx, "explorer": explorer, "notice": notice,
            }),
            X402Outcome::Paid {
                notice,
                tx,
                explorer,
                http_status,
                ok,
                amount,
                pay_to,
                resource,
                settlement,
                settlement_tx,
                settlement_explorer,
                body,
            } => serde_json::json!({
                "paid": paid, "payment": payment,
                "status": if *ok { "ok" } else { "settlement_failed" },
                "http_status": http_status,
                "amount": amount,
                "asset": "USDC",
                "pay_to": pay_to,
                "resource": resource,
                // 판매자가 정산 증빙에 적은 트랜잭션(없으면 빈 값). `settlement_failed` 에 tx 가 없으면
                // 행방은 «나갔을 수 있음» — GUI 의 정산 확인(confirm.rs)이 체인에서 우리 서명으로 결말을 낸다.
                "tx": tx,
                "explorer": explorer,
                "settlement": settlement,   // X-PAYMENT-RESPONSE (base64) — 정산 증빙
                // 판매자가 증빙에 적은 정산 tx — 서명 갈래 성공일 때만. 지갑이 확인한 값은 아니다.
                "settlement_tx": settlement_tx,
                "settlement_explorer": settlement_explorer,
                // 빈 값이 아니면 **돈은 나갔는데 서버가 증거를 안 받은 것**이다 — 재시도 금지.
                "notice": notice,
                "body": body.text,
            }),
        };
        // 받은 응답의 종류와 저장 위치 — 본문이 있는 갈래에만, 값이 있을 때만 싣는다.
        if let Some(b) = self.body() {
            if !b.content_type.is_empty() {
                v["content_type"] = serde_json::json!(b.content_type);
            }
            if !b.saved_file.is_empty() {
                v["saved_file"] = serde_json::json!(b.saved_file);
            }
            // 저장이 늦어 결과가 먼저 나갔다 — 파일은 뒤에 이 경로에 생긴다.
            if !b.saving_file.is_empty() {
                v["saving_file"] = serde_json::json!(b.saving_file);
            }
        }
        v
    }

    /// 받은 응답(본문이 있는 갈래만).
    pub fn body(&self) -> Option<&FetchedBody> {
        match self {
            X402Outcome::NotPaid { body, .. } | X402Outcome::Paid { body, .. } => Some(body),
            _ => None,
        }
    }
}

/// x402 유료 리소스를 가져온다. GET → 402 면 결제 요구 파싱 → GUI 승인(서명) → 결제 헤더로 재요청.
///
/// `memo` = 호출자가 준 결제 사유(없으면 서버 설명으로 폴백).
/// `request` = 보낼 method·headers·body. **첫 요청과 결제 재요청이 이 값 하나를 쓴다.**
/// `Err` = 진행 불가(요청/파싱 실패·앱 꺼짐·대기 중·시간 초과·서명 누락·재요청 실패).
pub async fn run_x402(
    url: &str,
    request: &FetchRequest,
    memo: Option<&str>,
    job: Option<&str>,
) -> Result<X402Outcome, String> {
    let url = url.trim().to_string();
    // 인자 없는 GET 만 예전처럼 리다이렉트를 따라간다(위 `http_client` 주석).
    let follow = request.is_plain_get();
    let client = if follow {
        http_client()?
    } else {
        http_client_no_redirect()?
    };

    // 1) 먼저 그냥 보내 본다. 402가 아니면 결제 불필요 → 본문 그대로 반환.
    let resp = request
        .apply(&client, &url)
        .send()
        .await
        .map_err(|e| tf!("요청 실패: {e}", "The request failed: {e}"))?;
    let http_status = resp.status().as_u16();
    if !follow && matches!(http_status, 301 | 302 | 303 | 307 | 308) {
        // 상대 경로면 요청한 주소 기준으로 풀어서 알려 준다.
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(|l| {
                resp.url()
                    .join(l)
                    .map(|u| u.to_string())
                    .unwrap_or_else(|_| l.to_string())
            })
            .unwrap_or_else(|| "?".to_string());
        return Err(tf!(
            "서버가 다른 주소로 가라고 했어요(HTTP {http_status} → {location}). 그 주소로 다시 요청하세요. 아무것도 결제하지 않았어요.",
            "The server redirected this request (HTTP {http_status} → {location}). Send it to that address instead. Nothing was paid.",
        ));
    }
    if http_status != 402 {
        let body = fetch::read_response(resp).await;
        return Ok(X402Outcome::NotPaid { http_status, body });
    }

    // probe 가 리다이렉트를 따라갔을 수 있으니 402 를 실제로 낸 "최종" URL 을 잡아둔다 —
    // 결제 재요청(서명 헤더 포함)은 이 최종 URL 에만 보낸다(원본 URL 로 보내면 다시 리다이렉트).
    let final_url = resp.url().clone();
    // 402 를 받은 시각 — 승인이 판매자의 `maxTimeoutSeconds` 를 넘기면 제출 전에 다시 확인한다.
    let challenged_at = std::time::Instant::now();
    // 🔴 **평문 HTTP 로는 결제하지 않는다**. 결제 헤더(서명된 인가)·호출자 헤더·본문이 그대로
    // 보이고 바뀔 수 있다 — 서명한 트랜잭션은 받는 사람·금액만 묶고 method·본문은 안 묶어서, 중간자가 그걸 먼저 내 버리거나
    // 다른 걸 사게 만들 수 있다. HTTPS → HTTP 로 내려간 리다이렉트도 여기서 걸린다(최종 주소를 본다). 로컬 개발 서버만 예외.
    if !payment_url_ok(&final_url) {
        return Err(tf!(
            "이 주소는 암호화되지 않은 http 라 결제하지 않았어요: {}",
            "Not paying over unencrypted http: {}",
            final_url
        ));
    }

    // 2) 결제 요구 파싱 → 우리가 처리 가능한 요구 선택.
    // V2는 요구가 `payment-required` 헤더(base64)에, V1은 본문에 온다. 본문을 소비하기 전에 헤더 먼저.
    let pr_header = resp
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    // 402 본문도 스트리밍 상한으로 읽는다(Content-Length 없는 거대 응답 차단). 잘렸으면 = 비정상.
    let (body402_bytes, truncated) = fetch::read_limited(resp, MAX_BODY_BYTES as usize).await?;
    if truncated {
        return Err(ts!(
            "402 응답 본문이 너무 큽니다",
            "The 402 response body is too large"
        )
        .into());
    }
    let body402 = String::from_utf8_lossy(&body402_bytes).into_owned();
    let required = x402::parse_required(pr_header.as_deref(), &body402).map_err(|e| {
        tf!(
            "402 형식 파싱 실패: {e}",
            "Couldn't parse the 402 response: {e}"
        )
    })?;
    // 체인을 **한 번** 읽어 고르기·대조·요청 각인을 모두 그 값으로 한다.
    let picked = active_chain();
    let req = x402::pick_requirement_for(&required, &picked)?;
    // 🔴 **요구를 고른 그 순간의 체인을 붙잡는다**. 고른 뒤 사용자가 네트워크를
    // 바꾸면 요청 작성기가 **바뀐 체인**을 각인하고 GUI 는 그 각인과 현재가 같으니 그대로 승인한다.
    // 그래서 고른 체인을 요청에 각인하고, GUI 는 승인할 때 현재 체인과 다시 대조한다.
    let picked_chain = picked.chain_id;
    let picked_explorer = (picked.explorer_tx_prefix, picked.explorer_tx_suffix);
    let amount_usdc = x402::base_units_to_usdc(&req.amount)?;
    // 🔴 승인 창·내역·알림에 보이는 리소스 URL은 **우리가 실제로 요청한 최종 URL**이다.
    // 예전엔 402 응답이 주장한 `resource` 문자열을 우선 썼다 — 그러면 evil.example 이
    // `resource: "https://api.trusted.io/x"` 라고 적어 두는 것만으로 **사람이 보는 창에 신뢰
    // 도메인이 뜬다**(비번을 넣는 판단 근거가 공격자가 쓴 문자열이 된다). 개발 47 은 ERC-8004
    // 도메인 대조만 final_url 로 옮겼고 **표시는 주장값 그대로 남아** 있었다 — 같은 창에서 위쪽
    // URL(주장)과 아래쪽 경고의 「실제 요청 주소」(final_url)가 서로 다를 수 있었다.
    // 서버의 주장은 프로토콜 제출(V2 raw 에코)에만 쓰고, 사람에게는 보여주지 않는다.
    let resource = final_url.to_string();
    // 팝업에 보일 사유: 호출자 memo > 서버 description(V1 요구별/V2 최상위) > 빈 값.
    let memo = memo
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
        .or_else(|| {
            let d = required.description(&req);
            (!d.is_empty()).then_some(d)
        })
        .unwrap_or_default();

    // 3) 승인할 앱이 켜져 있는지 + single-flight. **조회보다 먼저** 본다 — 어차피 못 띄울
    // 요청이면 레지스트리를 4번 읽어 봐야 결과를 버릴 뿐이고, 느린 RPC 만큼 즉시 줘야 할
    // 안내가 늦어진다. 꺼져 있으면 먼저 깨운다.
    if !payment::ensure_app_alive().await {
        return Err(app_unavailable());
    }
    if payment::has_pending() {
        return Err(ts!(
            "이미 승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 다시 요청하세요.",
            "A payment is already waiting for approval. Let the user handle it, then ask again."
        )
        .into());
    }

    // 3-1) 판매자 토큰 계정(ATA)이 있는지 — 승인 창 **전에**. 없으면 사람이 비번을 넣어 봐야 정산이 실패한다.
    // 못 확인하면(RPC 오류) 결제하지 않는다(닫힌 쪽).
    {
        let client = http_client()?;
        let rpc = crate::wallet::effective_rpc();
        match crate::solana::token_account_exists(
            &client,
            &rpc,
            picked.genesis_hash,
            req.pay_to.trim(),
            picked.usdc_mint,
            picked.usdc_decimals,
        )
        .await
        {
            Ok(true) => {}
            Ok(false) => {
                return Err(ts!(
                    "판매자의 USDC 계정이 아직 체인에 없어 결제할 수 없어요. 아무것도 결제하지 않았습니다 — 판매자에게 알려 주세요.",
                    "The seller's USDC account doesn't exist on-chain yet, so this can't be paid. Nothing was paid — let the seller know."
                )
                .into());
            }
            Err(e) => {
                return Err(tf!(
                    "판매자 계정을 확인하지 못해 결제하지 않았어요({}). 아무것도 결제하지 않았습니다 — 잠시 뒤 다시 시도하세요.",
                    "Couldn't check the seller's account, so nothing was paid ({}). Try again shortly.",
                    crate::wallet::redact_urls(&e)
                ));
            }
        }
    }

    // 4) GUI에 서명 요청 → 사람 승인 → 서명 트랜잭션 수신.
    // 요청을 쓰기 직전에 **고를 때의 체인 그대로인지** 본다. 바뀌었으면 아직 아무것도
    // 안 썼으니 여기서 멈추는 게 답이다 — 다시 부르면 새 체인의 챌린지로 처음부터 간다.
    if active_chain().chain_id != picked_chain {
        return Err(ts!(
            "결제를 준비하는 사이 네트워크가 바뀌었어요. 아무것도 결제하지 않았습니다 — 다시 시도하세요.",
            "The network changed while this payment was being prepared. Nothing was paid — try again."
        )
        .into());
    }
    if !payment::app_supports(crate::policy::KIND_X402) {
        return Err(ts!(
            "켜져 있는 지갑 앱이 이 결제 방식을 몰라요. 앱을 다시 빌드·실행한 뒤 시도하세요. 아무것도 결제하지 않았습니다.",
            "The running wallet app doesn't support this payment method. Rebuild and reopen the app, then try. Nothing was paid."
        )
        .into());
    }
    let id = payment::write_x402_request(
        req.pay_to.trim(),
        &amount_usdc,
        &memo,
        &resource,
        picked_chain,
        req.terms.clone(),
        job.unwrap_or_default(),
    )?;
    let (result, app_gone) = payment::await_result(&id, payment::APPROVAL_TIMEOUT).await;
    let result = match result {
        Some(r) => r,
        // 아무것도 안 나갔다(요청은 await_result 가 거뒀다).
        // 기다리는 사이 앱이 죽었으면(하트비트 끊김) «사용자가 응답 안 함» 이 아니다(실측: 승인 직후 앱이 죽은 경우).
        None if app_gone => {
            return Err(ts!(
                "승인을 기다리는 사이 지갑 앱이 꺼졌거나 응답을 멈췄어요. 아무것도 결제하지 않았습니다 — 사용자에게 알리고, 앱을 다시 켠 뒤 시도하세요.",
                "The wallet app quit or stopped responding while waiting for approval. Nothing was paid — tell the user, and try again after the app is reopened."
            )
            .into());
        }
        None => {
            return Err(ts!(
                "승인 시간 초과(5분). 사용자가 응답하지 않았어요.",
                "Approval timed out after 5 minutes — the user didn't respond."
            )
            .into());
        }
    };
    // 「불명」 = GUI 가 서명을 끝내기 전에 우리가 기다림을 접었다 → 서명은 **서버로 가지 않는다**(보낼
    // 사람이 우리뿐이다). 그러니 이건 「아무것도 안 나감」이다.
    if result.status == "unknown" {
        return Ok(X402Outcome::Declined {
            status: "failed".into(),
            detail: ts!(
                "지갑이 서명을 끝내기 전에 기다림이 끝났어요. 서명은 서버로 보내지 않았으니 아무것도 결제되지 않았습니다 — 다시 시도해도 됩니다.",
                "This call stopped waiting before the wallet finished signing. The signature was never sent to the server, so nothing was paid — it is safe to try again."
            )
            .into(),
        });
    }
    if result.status != "approved" {
        return Ok(X402Outcome::Declined {
            status: result.status, // rejected | failed
            detail: result.detail,
        });
    }
    // 5) 서명으로 제출물을 만든다.
    let payment = result.x402.ok_or(ts!(
        "승인됐지만 서명 페이로드가 비어 있어요",
        "Approved, but the signature payload is empty"
    ))?;
    // 5-1) 🔴 승인이 판매자가 잡은 유효 시간을 넘겼으면 **제출 전에 402 를 다시 받아** 같은 조건인지 본다.
    // 사람이 본 금액·받는 곳이 그 사이 바뀌었으면 서명을 보내지 않는다 — 보낼 사람이 우리뿐이라 그러면 아무것도 안 나간다.
    let mut req = req;
    let waited = challenged_at.elapsed().as_secs();
    if waited > req.max_timeout_secs {
        match reconfirm_offer(request, &final_url, &req.raw).await {
            Ok(fresh) => req.raw = fresh,
            Err(why) => {
                // 내역은 「정산 대기」 로 둔다 — 「정산 실패」 는 «나갔을 수 있음» 으로 읽히는 상태라 AI 에게 한 말(안 나감)과
                // 반대가 된다. 이 서명은 블록해시가 만료되면 개발 5 의 만료 확인이 「만료 · 안 나감」 으로 닫고
                // 한도를 돌려준다. 그 전까지는 한도 예약이 붙잡혀 있다.
                return Ok(X402Outcome::Declined {
                    status: "failed".into(),
                    detail: tf!(
                        "승인에 {waited}초가 걸려 판매자의 결제 유효 시간({}초)을 넘겼고, 다시 확인해 보니 {why}. 서명을 보내지 않았으니 아무것도 결제되지 않았습니다 — 같은 요청을 다시 하면 새 승인 창이 뜹니다.",
                        "Approval took {waited}s, past the seller's {}s payment window, and on re-checking {why}. The signature was never sent, so nothing was paid — asking again shows a new approval prompt.",
                        req.max_timeout_secs
                    ),
                });
            }
        }
    }
    let sub = required.build_submission(&req, &payment)?;
    deliver(Delivery {
        sub,
        url: final_url.as_str(),
        request,
        signed: payment,
        explorer: picked_explorer,
        chain_id: picked_chain,
        amount: amount_usdc,
        pay_to: req.pay_to,
        resource,
    })
    .await
}

/// 유효 시간이 지난 요구를 다시 확인한다 — 같은 요청을 결제 헤더 없이 다시 보내 402 를 받고, 우리가 서명한 것과
/// 같은 요구(힌트 제외)가 아직 있으면 그 새 원본을 돌려준다. 아니면 이유(사람·AI 용 짧은 문장).
/// 리다이렉트는 따라가지 않는다 — 결제 재요청이 갈 곳(`final_url`)과 같은 곳이어야 의미가 있다.
async fn reconfirm_offer(
    request: &FetchRequest,
    final_url: &reqwest::Url,
    picked: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = http_client_no_redirect()?;
    let resp = request
        .apply(&client, final_url.as_str())
        .send()
        .await
        .map_err(|_| ts!("판매자에게 다시 연결하지 못했어요", "the seller couldn't be reached").to_string())?;
    if resp.status().as_u16() != 402 {
        return Err(tf!(
            "판매자가 결제 요구 대신 HTTP {} 로 답했어요",
            "the seller answered HTTP {} instead of a payment challenge",
            resp.status().as_u16()
        ));
    }
    let header = resp
        .headers()
        .get("payment-required")
        .and_then(|v| v.to_str().ok())
        .map(String::from);
    let (body, truncated) = fetch::read_limited(resp, MAX_BODY_BYTES as usize).await?;
    if truncated {
        return Err(ts!("402 응답이 너무 커요", "the 402 response was too large").into());
    }
    let pr = x402::parse_required(header.as_deref(), &String::from_utf8_lossy(&body))?;
    x402::find_same_offer(&pr, picked).ok_or_else(|| {
        ts!(
            "결제 조건(금액·받는 곳·메모 등)이 바뀌었어요",
            "the payment terms (amount, recipient, memo…) had changed"
        )
        .to_string()
    })
}

/// 서명 갈래에서 판매자가 정산 증빙에 적은 tx 와 탐색기 링크(78 실결제에서 확인한 빈칸 — AI 가 결제 링크를
/// 바로 못 보여 줬다). 지갑이 체인에서 확인한 값이 아니라 **판매자 주장**이다. 2xx·`success` 가 거짓이 아님·tx 해시 꼴일
/// 때만 — 아무 문자열이나 링크로 만들면 판매자가 고른 주소로 사람을 보내게 된다. 직접 제출은 `tx` 가 이미 우리 것이라 빈 값.
fn seller_settlement_link(
    http_ok: bool,
    settlement: &str,
    explorer: (&str, &str),
) -> (String, String) {
    match x402::parse_settlement(settlement) {
        Some((tx, true)) if http_ok && x402::is_tx_hash(&tx) => {
            let link = format!("{}{tx}{}", explorer.0, explorer.1);
            (tx, link)
        }
        _ => (String::new(), String::new()),
    }
}

/// 결제 제출물 하나를 보낼 때 필요한 것 — `deliver` 의 인자.
struct Delivery<'a> {
    sub: x402::Submission,
    /// 402 를 실제로 낸 최종 URL — 결제 헤더는 여기에만, 리다이렉트를 따라가지 않고 보낸다.
    url: &'a str,
    /// 첫 요청과 같은 method·headers·body — 서명한 결제와 다른 걸 사지 않게.
    request: &'a FetchRequest,
    /// 부분 서명한 트랜잭션(정산 기록을 우리 서명으로 잇는다 — GUI 내역의 「signed」 줄 detail 과 같은 값).
    signed: crate::policy::SvmSigned,
    /// 결제한 체인의 탐색기 tx 링크 앞부분 — 서명 갈래의 판매자 정산 tx 를 링크로 만들 때.
    explorer: (&'static str, &'static str),
    /// 결제한 체인 — 정산 기록 파일을 고른다. 응답을 기다리는 사이 활성 체인이 바뀔 수 있다.
    chain_id: u64,
    amount: String,
    pay_to: String,
    resource: String,
}

/// 결제 제출물을 보내고 응답을 결과로 만든다. 「돈이 나갔을 수 있는데 서버가 거절·무응답」을 가르는 판정이 여기 있다.
async fn deliver(d: Delivery<'_>) -> Result<X402Outcome, String> {
    let Delivery {
        sub,
        url,
        request,
        signed,
        explorer,
        chain_id,
        amount,
        pay_to,
        resource,
    } = d;
    // 결제 헤더(서명된 인가)는 리다이렉트를 따라가지 않는 클라이언트로 최종 URL 에만 보낸다.
    let pay_client = http_client_no_redirect_for(PAID_SIGNED_TIMEOUT)?;
    // 첫 요청과 **같은 조립 함수**로 만들고 결제 헤더만 얹는다.
    let mut builder = request
        .apply(&pay_client, url)
        .header(sub.header_name, &sub.value);
    if let Some(alt) = sub.alt_header {
        builder = builder.header(alt, &sub.value); // 같은 값 — 헤더 이름만 다르게 읽는 서버 대비
    }
    let paid_resp = match builder.send().await {
        Ok(r) => r,
        Err(e) => {
            let msg = tf!("결제 재요청 실패: {e}", "The paid re-request failed: {e}");
            // 🔴 **서명 갈래도 연결이 한 번 붙었으면 「안 냈다」가 아니다**.
            // 서명된 인가는 헤더에 실려 **이미 서버에 닿았을 수 있고**, 서버·페이실리테이터는 응답을 보내기 전에
            // 정산할 수 있다. 여기서 평범한 `Err` 를 돌리면 AI 는 다시 부르고 **새 트랜잭션(새 memo)으로 두 번째 결제**를
            // 받는다 — 둘 다 정산될 수 있다. 연결 자체가 안 됐을 때(`is_connect`)만 확실히 아무것도 안 갔다.
            if !e.is_connect() {
                // 내역에 「내용 못 받음」 표시 — 결말(정산·만료)은 GUI 체인 확인이 우리 서명으로 정한다.
                let _ = payment::record_settlement(chain_id, &signed.signature, "", false, true, false);
                return Ok(X402Outcome::PaidNoContent {
                    notice: x402::signed_unknown_notice(&msg),
                    tx: String::new(),
                    explorer: String::new(),
                    reason: "unknown".into(),
                });
            }
            return Err(msg);
        }
    };
    let paid_status = paid_resp.status().as_u16();
    // 정산 증빙 헤더도 이름이 갈린다 — V2 는 PAYMENT-RESPONSE, 옛 어댑터는 X-PAYMENT-RESPONSE. 둘 다 본다(우선순위는 버전이 정한 이름).
    let mut response_names = vec![sub.response_header];
    if sub.alt_header.is_some() {
        response_names.push("X-PAYMENT-RESPONSE");
    }
    let settlement = response_names
        .iter()
        .filter_map(|name| paid_resp.headers().get(*name))
        .find_map(|v| v.to_str().ok())
        .map(String::from)
        .unwrap_or_default();
    let body = fetch::read_response(paid_resp).await;

    // 정산 tx 해시를 뽑아 GUI 내역에 반영되도록 기록(우리 서명으로 "signed" 항목과 매칭).
    // **2xx(실제 정산 성공) 응답일 때만** 기록한다 — 비-2xx 응답에 위조 PAYMENT-RESPONSE 헤더를
    // 실어 보내 GUI 내역을 가짜 "정산됨"으로 오염시키는 걸 막는다.
    // 여긴 **서명만 한 결제**를 정산 tx 로 잇는 자리다.
    let http_ok = (200..300).contains(&paid_status);
    let mut recorded = false;
    if http_ok && !settlement.is_empty() {
        // 정산 tx 는 판매자 말이다 — 솔라나 서명 꼴(64바이트 base58)이 아니면 내역에 잇지 않는다.
        // 체인에서 signatures[1] 을 대조하는 확인은 개발 5.
        // 실패 보고는 tx 없이도 잇는다 — 엉터리 tx 칸 때문에 내역이 「signed」 로 남지 않게.
        if let Some((tx, success)) = x402::parse_settlement(&settlement)
            .map(|(tx, ok)| (if x402::is_tx_hash(&tx) { tx } else { String::new() }, ok))
            .filter(|(tx, ok)| !ok || !tx.is_empty())
        {
            let _ = payment::record_settlement(
                chain_id,
                &signed.signature,
                &tx,
                success,
                body.incomplete || body.save_failed,
                // 내용을 끝까지 받아 남겼다는 긍정 신호 — 정산 성공 + 본문 완전 + 저장 성공일 때만.
                // 저장이 늦어 아직 모르면(save_pending) 붙이지 않는다 — 작업 예산의 «같은 결제 10분» 예외가 열리지 않게.
                success && !body.incomplete && !body.save_failed && !body.save_pending,
            );
            recorded = true;
        }
    }
    // 서명은 서버 손에 갔는데 내용을 끝까지 못 받았다(거절·끊김·저장 실패) — 내역에 「내용 못 받음」 표시만 남긴다.
    // 상태는 바꾸지 않는다: 돈이 나갔는지는 GUI 의 체인 확인이 우리 서명으로 정한다.
    if !recorded && (!http_ok || body.incomplete || body.save_failed) {
        let _ = payment::record_settlement(chain_id, &signed.signature, "", false, true, false);
    }
    let (settlement_tx, settlement_explorer) =
        seller_settlement_link(http_ok, &settlement, explorer);

    // 🔴 서명 갈래에서 2xx 인데 정산 증빙이 `success:false` 라고 말하면 「정산 성공」이 아니다 —
    // 예전엔 HTTP 2xx 만 보고 `payment:"confirmed"` 로 말했고, 같은 응답을 GUI 내역은 「정산 실패」로 적었다. 서명은 서버 손에
    // 있으니 나중에 정산될 수도 있다 → `ok=false` 로 두면 `payment_state` 가 「모름」으로 말한다.
    let ok = http_ok && !x402::settlement_says_failed(&settlement);

    let notice = if body.save_failed {
        // 결제 뒤 받은 파일을 못 남겼다 — 바이트는 사라졌고, 다시 받으려면 또 결제된다.
        ts!(
            "결제한 응답을 파일로 저장하지 못했어요(저장 폴더 권한이나 디스크 공간). 다시 요청하면 또 결제돼요 — 사용자에게 알리고, 저장 문제를 고친 뒤에 다시 할지 물어보세요.",
            "The paid response couldn't be saved to a file (folder permission or disk space). Asking again pays again — tell the user and ask before retrying once saving is fixed."
        )
        .to_string()
    } else if body.incomplete {
        // 결제는 됐거나 됐을 수 있는데 응답이 중간에 끊겼다 — 다시 부르면 또 결제된다.
        ts!(
            "결제 뒤 응답을 끝까지 받지 못했어요(끊겼거나 크기 상한을 넘음). 다시 요청하면 또 결제될 수 있어요 — 남긴 부분이 있으면 body·saved_file 에 있어요. 사용자에게 알리세요.",
            "The response after payment didn't arrive in full (cut off, or over the size limit). Asking again may pay again — whatever was kept is in body / saved_file. Tell the user."
        )
        .to_string()
    } else {
        String::new()
    };

    Ok(X402Outcome::Paid {
        notice,
        tx: String::new(),
        explorer: String::new(),
        http_status: paid_status,
        ok,
        amount,
        pay_to,
        resource,
        settlement,
        settlement_tx,
        settlement_explorer,
        body,
    })
}

#[cfg(test)]
mod tests;

/// x402 흐름 future 크기 상한 — src-tauri 승인 future 가 디버그에서 215KB 로 불어 작업 스레드 스택을 넘친 적이 있다.
#[cfg(test)]
mod future_size_guard {
    #[test]
    fn x402_flow_future_stays_small() {
        let req = crate::fetch::FetchRequest::default();
        let f = super::run_x402("http://localhost/", &req, None, None);
        let n = std::mem::size_of_val(&f);
        eprintln!("run_x402 future = {n} bytes");
        assert!(n < 16 * 1024, "run_x402 future = {n} bytes");
    }

    // 나머지 도구 경로도 — 잔액 조회·승인 기다리기.
    #[test]
    fn other_tool_futures_stay_small() {
        let n = std::mem::size_of_val(&crate::wallet::get_balances(""));
        assert!(n < 16 * 1024, "get_balances future = {n} bytes");
        let n = std::mem::size_of_val(&crate::payment::await_result("", std::time::Duration::ZERO));
        assert!(n < 16 * 1024, "await_result future = {n} bytes");
        let n = std::mem::size_of_val(&crate::payment::ensure_app_alive());
        assert!(n < 16 * 1024, "ensure_app_alive future = {n} bytes");
    }
}
