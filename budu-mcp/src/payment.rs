// 결제 요청 IPC — MCP가 결제를 "요청"하고 GUI 승인을 기다린다.
//
// 비번은 절대 여기로 들어오지 않는다. MCP는 ~/.budu에 요청 파일만 쓰고, 실제 서명·전송은
// GUI 앱(src-tauri)이 한다. 흐름:
//   write_x402_request() → GUI가 팝업으로 사람 승인 → GUI가 결과 파일 작성 → await_result()가 읽어 반환.
//
// single-flight: 한 번에 대기 요청 1건. 앱이 안 켜져 있으면(하트비트 신선도) 즉시 안내.

use crate::wallet::budu_dir;
use crate::{tf, ts};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// GUI가 살아있다고 볼 하트비트 최대 나이(초). 이보다 오래되면 앱이 꺼진 것으로 본다.
const ALIVE_SECS: u64 = 10;
/// 사용자 승인 대기 최대 시간(초). GUI 팝업 카운트다운과 일치(5분).
pub const APPROVAL_TIMEOUT: Duration = Duration::from_secs(300);

/// 🔴 **승인 대기가 끝났는데 GUI 가 아직 전송 중일 때 더 기다리는 상한**.
///
/// GUI 의 전송은 개발 66 부터 단계마다 상한이 있다(채우기 30 + 제출 30 + 되묻기 10 + 재제출 30 ≈ 100초,
/// 여기에 비번 복호화·자율 경로의 잔액 조회 몇 초). 그 합을 덮는 값이다. 이걸 넘기면 「모른다」로 답한다 —
/// 「응답 없음」이 아니다(사람은 이미 승인했고 돈이 나가는 중일 수 있다).
pub const LATE_SEND_WAIT: Duration = Duration::from_secs(150);

/// 에이전트가 보내는 결제 요청. 비밀은 없다(비번은 GUI에서만).
#[derive(Serialize, Deserialize, Clone)]
pub struct PaymentRequest {
    pub id: String,
    pub token: String,
    pub to: String,
    pub amount: String,
    pub memo: String,
    pub created: u64,
    /// 요청 종류 — 지금은 "x402" 하나다.
    pub kind: String,
    /// x402일 때 결제 대상 리소스 URL (사용자가 팝업에서 본다). transfer면 빈 문자열.
    #[serde(default)]
    pub resource: String,
    /// 요청 생성 시점의 활성 체인 ID — GUI가 승인 시 현재 체인과 다르면 거부한다.
    #[serde(default)]
    pub chain_id: u64,
    /// 요청 생성 시점의 **활성 계정** — 파생 인덱스 + 주소. GUI 가 승인 시 지금 계정과
    /// 대조해 다르면 거부하고, 같으면 승인 작업 전체를 이 계정으로 고정한다(chain_id 와 같은 처방).
    #[serde(default)]
    pub account: u32,
    #[serde(default)]
    pub from: String,
    /// **임대를 갱신하는 요청인가**. 새 MCP 는 기다리는 동안 요청 파일의 수정 시각을
    /// 주기적으로 갱신한다 — 멈추면(프로세스가 SIGKILL 로 죽어 `CancelOnDrop` 도 못 돈 경우) GUI 는
    /// `REQUEST_LEASE_SECS` 뒤부터 그 요청을 보여주지도 승인하지도 않는다. 옛 MCP 의 요청엔 이 필드가
    /// 없다(false) → 예전 규칙(5분+유예) 그대로다.
    #[serde(default)]
    pub lease: bool,
    /// x402 솔라나 결제 조건 — GUI 가 이 값으로 트랜잭션을 조립하고, 요청의 `to`·`amount` 와 다시 대조한다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub svm: Option<crate::policy::SvmTerms>,
    /// 작업 이름표 — AI 가 준 `job` 을 `policy::clip_job` 으로 정리한 값. 승인 창이 보여 주고, 사람이 그 작업에
    /// 예산을 열면 같은 이름표의 다음 결제가 그 예산 안에서 비번 없이 나간다. 빈 값 = 작업 없음(칸째 안 쓴다).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub job: String,
}

/// GUI가 쓰는 처리 결과.
#[derive(Deserialize)]
pub struct PaymentResult {
    pub id: String,
    pub status: String,
    pub tx_hash: String,
    pub detail: String,
    /// x402 승인일 때 GUI 가 부분 서명해 돌려준 트랜잭션. 그 밖엔 None.
    #[serde(default)]
    pub x402: Option<crate::policy::SvmSigned>,
}

impl PaymentResult {
    /// GUI 의 결제 시도 기록에서 결과를 되살린다 — 요청을 거둔 뒤라 결과 파일이 안 온 경우.
    fn from_record(r: crate::policy::AttemptRecord) -> Self {
        PaymentResult {
            id: r.id,
            status: r.status,
            tx_hash: r.tx_hash,
            detail: r.detail,
            x402: r.x402.and_then(|v| serde_json::from_value(v).ok()),
        }
    }

    /// 승인 대기도, 늦은 전송 대기도 끝났는데 GUI 가 아직 처리 중일 때의 결과 — **불명**.
    fn still_sending(id: &str, rec: Option<crate::policy::AttemptRecord>) -> Self {
        PaymentResult {
            id: id.to_string(),
            status: "unknown".into(),
            tx_hash: rec.map(|r| r.tx_hash).unwrap_or_default(),
            detail: ts!(
                "사용자가 승인해서 지갑이 결제를 처리하는 중이었는데, 끝나는 걸 기다리지 못했어요. 나갔을 수 있으니 다시 요청하기 전에 사용자에게 알리고 지갑 내역을 확인하세요.",
                "The user approved and the wallet was still processing the payment when this call stopped waiting. It may have gone through — tell the user and check the wallet history before asking again."
            )
            .into(),
            x402: None,
        }
    }
}

#[derive(Deserialize)]
struct Heartbeat {
    ts: u64,
    /// **승인 창을 실제로 띄울 수 있나**. GUI 의 러스트 스레드는 프로세스가 살아 있는
    /// 한 하트비트를 찍으므로, WebView 만 죽으면 「살아 있다」고 말하면서 창은 안 뜬다.
    /// 그쪽이 창을 여러 번 깨워 보고도 안 되면 이 값을 false 로 내린다 → 즉시 정직하게 거절한다.
    /// 없으면 true (이 필드가 없던 옛 앱과의 호환 — 예전과 똑같이 동작).
    #[serde(default = "ui_ok_default")]
    ui_ok: bool,
    /// **앱이 처리할 수 있는 결제 방식**. 새 방식을 옛 앱에 보내지 않으려고 본다 —
    /// 옛 앱은 모르는 kind 를 **평범한 송금으로 처리**했다(돈은 나가고 결제는 성립하지 않는다).
    /// 필드가 없으면 그 시절 앱이므로 송금·x402 서명 둘만 할 수 있는 것으로 본다.
    #[serde(default = "kinds_default")]
    kinds: Vec<String>,
}

fn ui_ok_default() -> bool {
    true
}

fn kinds_default() -> Vec<String> {
    crate::policy::kinds_legacy()
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

fn mcp_alive_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("mcp_alive.json"))
}

/// 정산 기록은 **결제한 체인**의 파일에 — 응답을 기다리는 사이 사용자가 체인을 바꿔도 지금 체인으로 가지 않게.
fn settlements_path(chain_id: u64) -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(crate::policy::chain_file_name(chain_id, "x402_settlements")))
}

/// x402 정산 결과 1건 — GUI가 읽어 내역의 "signed"(nonce 매칭)을 "settled"+tx 로 갱신한다.
#[derive(Serialize, Deserialize)]
struct Settlement {
    nonce: String,
    tx: String,
    success: bool,
    /// 돈은 나갔거나 나갔을 수 있는데 응답 내용을 끝까지 못 받았다. `tx` 가 비고 `success` 가 거짓이면서 이 값이
    /// 참이면 **표시만** 하는 기록이다 — GUI 는 상태를 바꾸지 않고 「내용 못 받음」 만 붙인다(결말은 체인 확인이 정한다).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    undelivered: bool,
    /// 정산된 응답의 내용을 끝까지 받아 저장까지 했다 — 앱의 작업 예산이 «같은 결제 10분» 예외를 이 긍정 신호로만 연다.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    delivered: bool,
}

/// 원자 쓰기의 임시 파일 경로 — **작성자마다 다르다**.
///
/// 예전엔 `path.with_extension("tmp")` 하나를 모두가 같이 썼다. 같은 파일을 두 작성자가 동시에 쓰면
/// (예: 세션 잠금 해제의 KDF 업그레이드와 계정 이름 바꾸기가 둘 다 wallet.enc 를) 그 한 임시 파일을 번갈아
/// truncate·쓰기 하다 **섞인 내용을 rename** 했다 — 재현 테스트로 확인(256KB·64KB 두 스레드, 40회 중 발생).
/// 프로세스 id + 프로세스 안 순번이면 두 프로세스(GUI·MCP) 사이에서도 겹치지 않는다. 결과는 「마지막
/// 작성자가 이긴다」 — 내용이 섞이는 일은 없다.
fn unique_tmp(path: &Path) -> PathBuf {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

/// 임시 파일에 쓴 뒤 rename 으로 원자 교체 — GUI(별도 프로세스)가 폴링으로 읽는 파일들이라,
/// 쓰는 도중의 절반 써진 내용을 GUI가 읽는 일이 없게 한다.
/// 권한은 src-tauri 의 store::write_atomic 과 동일하게 디렉터리 0700 / 파일 0600 으로 맞춘다
/// (모든 ~/.budu 파일 권한 일관 적용 불변식 — MCP 가 먼저 파일을 만들어도 넓게 노출되지 않게).
fn write_atomic(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)
            .map_err(|e| tf!("디렉터리 생성 실패: {e}", "Couldn't create the folder: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o700));
        }
    }
    let tmp = unique_tmp(path);
    write_file_private(&tmp, bytes)?;
    fs::rename(&tmp, path).map_err(|e| tf!("파일 교체 실패: {e}", "Couldn't replace the file: {e}"))
}

/// 파일을 0600 으로 생성해 내용을 쓴다 (생성 후 chmod 사이의 노출 창 제거).
#[cfg(unix)]
fn write_file_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))?;
    let _ = f.set_permissions(fs::Permissions::from_mode(0o600));
    f.write_all(bytes)
        .map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))
}

#[cfg(not(unix))]
fn write_file_private(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    fs::write(path, bytes).map_err(|e| tf!("파일 저장 실패: {e}", "Couldn't save the file: {e}"))
}

/// 정산 결과를 ~/.budu/x402_settlements.json 에 추가한다(append). 실패해도 결제 흐름은 안 막는다.
/// nonce = 서명 인가의 nonce(GUI 내역 detail 과 매칭). 비밀 아님(공개 결제 증빙).
pub fn record_settlement(
    chain_id: u64,
    nonce: &str,
    tx: &str,
    success: bool,
    undelivered: bool,
    delivered: bool,
) -> Result<(), String> {
    let path = settlements_path(chain_id)?;
    // 🔴 **읽기-덧붙이기-쓰기를 프로세스 사이에서 한 줄로**. AI 클라이언트마다 MCP
    // 프로세스가 따로 떠서 두 결제가 거의 같이 정산되면, 둘 다 같은 목록을 읽고 나중에 쓴 쪽이 먼저 쓴 정산을
    // 지운다 — 그 내역은 영영 「정산 대기」다. 잠금 파일에 OS 배타 잠금(flock)을 건다; 프로세스가 죽으면 풀린다.
    let lock_path = path.with_extension("lock");
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .map_err(|e| {
            tf!(
                "정산 기록 잠금 실패: {e}",
                "Couldn't lock the settlement record: {e}"
            )
        })?;
    lock.lock().map_err(|e| {
        tf!(
            "정산 기록 잠금 실패: {e}",
            "Couldn't lock the settlement record: {e}"
        )
    })?;
    // 있는데 못 읽는 파일은 빈 목록으로 덮지 않는다 — GUI 가 아직 안 가져간 정산(nonce·tx)이 사라진다.
    // 옆으로 치워 바이트를 남기고 새로 시작한다(서명의 결말은 GUI 체인 확인이 따로 찾는다). 치우지도 못하면 기록을 포기한다.
    let mut list: Vec<Settlement> = match fs::read_to_string(&path) {
        Ok(text) => match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(_) => {
                set_aside(&path)?;
                Vec::new()
            }
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(_) => {
            set_aside(&path)?;
            Vec::new()
        }
    };
    list.push(Settlement {
        nonce: nonce.to_string(),
        tx: tx.to_string(),
        success,
        undelivered,
        delivered,
    });
    let json = serde_json::to_string(&list).map_err(|e| {
        tf!(
            "정산 기록 직렬화 실패: {e}",
            "Couldn't serialize the settlement record: {e}"
        )
    })?;
    write_atomic(&path, json.as_bytes())
}

/// 못 읽는 정산 파일을 `<이름>.broken.<초>.<pid>` 로 치운다 — GUI 는 이 이름을 안 가져간다.
fn set_aside(path: &Path) -> Result<(), String> {
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let aside = path.with_file_name(format!("{name}.broken.{}.{}", now_secs(), std::process::id()));
    fs::rename(path, aside).map_err(|e| {
        tf!(
            "정산 기록을 읽지 못했고 옆으로 치우지도 못했어요: {e}",
            "Couldn't read the settlement record or set it aside: {e}"
        )
    })
}

/// MCP(=AI 클라이언트) 생존 표시를 쓴다. GUI가 이걸 보고 "AI 연결됨" 배지를 띄운다.
/// client = 연결한 클라이언트 이름(예: "claude-code"). 빈 문자열이면 GUI가 일반 표기.
pub fn write_mcp_heartbeat(client: &str) -> Result<(), String> {
    let body = serde_json::json!({ "ts": now_secs(), "client": client });
    write_atomic(&mcp_alive_path()?, body.to_string().as_bytes())
}

/// MCP 종료 시 하트비트를 지운다 → GUI가 즉시 "연결 안 됨"으로 본다.
pub fn clear_mcp_heartbeat() {
    if let Ok(p) = mcp_alive_path() {
        let _ = fs::remove_file(p);
    }
}

pub fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// 하트비트가 신선한지(순수 함수 — 테스트용).
fn is_fresh(now: u64, beat: u64) -> bool {
    now.saturating_sub(beat) <= ALIVE_SECS
}

/// GUI 앱이 최근에 살아있었는지. 결제 요청을 띄울 사람이 있는지 확인용.
/// 「살아 있다」 = 프로세스가 있다가 아니라 **여기서 사람이 승인까지 할 수 있다** → 화면이
/// 죽은 상태(`ui_ok:false`)는 살아 있는 걸로 치지 않는다.
pub fn app_alive() -> bool {
    read_heartbeat().is_some_and(|h| h.ui_ok)
}

/// 신선한 하트비트(없거나 낡았으면 None). 이유를 갈라 안내하려고 `ui_ok` 까지 돌려준다.
fn read_heartbeat() -> Option<Heartbeat> {
    let h: Heartbeat = heartbeat_path()
        .ok()
        .and_then(|p| fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())?;
    is_fresh(now_secs(), h.ts).then_some(h)
}

/// **지금 켜져 있는 앱이 이 결제 방식을 아는가**. 모르면 요청을 아예 쓰지 않는다 —
/// 옛 앱에 `x402-direct` 를 보내면 그쪽은 그걸 평범한 송금으로 처리한다(서버가 알아볼 수 없는
/// 전송이 나가고 돈만 없어진다). 하트비트가 없으면(앱 꺼짐) false — 그 경우는 호출자가 이미
/// `app_alive()` 로 갈라 안내한다.
pub fn app_supports(kind: &str) -> bool {
    read_heartbeat().is_some_and(|h| h.kinds.iter().any(|k| k == kind))
}

/// 앱은 떠 있는데 **화면(WebView)이 죽어** 승인 창을 못 띄우는 상태인가.
/// 「앱을 켜세요」와 「앱을 다시 시작하세요」는 사용자가 할 일이 다르므로 갈라서 안내한다.
pub fn ui_stalled() -> bool {
    read_heartbeat().is_some_and(|h| !h.ui_ok)
}

/// 이 실행 파일이 든 앱 번들 — `…/Budu.app/Contents/MacOS/budu-mcp` 면 `…/Budu.app`.
/// 번들 밖(dev 의 `target/debug`, 손으로 옮긴 바이너리)이면 None — 깨울 앱을 모른다.
fn bundle_of(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    let is_bundle = macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension().is_some_and(|e| e == "app");
    is_bundle.then(|| app.to_path_buf())
}

/// 앱을 깨운 뒤 하트비트를 기다리는 상한. 앱은 뒤쪽(`-g`)에 큰 창을 띄우며 뜨고 러스트 감시 스레드가 1초마다
/// 하트비트를 찍는다(WebView 를 기다리지 않는다) — 콜드 스타트가 몇 초면 끝나니 넉넉히 잡은 값이다.
const WAKE_WAIT: Duration = Duration::from_secs(20);

/// 🔴 **승인할 앱이 꺼져 있으면 깨운다**.
///
/// MCP 엔 상주 연결이 없다 — Claude 가 켤 때 우리를 띄운다. 그런데 결제는 앱(승인 창)이 있어야 한다.
/// 예전엔 앱이 꺼져 있으면 「앱을 켠 뒤 다시 시도하세요」로 끝났고, 사람은 Budu 를 켜고 AI 에게 다시
/// 말해야 했다. 자동 시작을 꺼 둔 사람·종료해 둔 사람에게 「연결」은 매번 끊겨 있는 셈이었다.
/// 이제는 **우리가 든 그 번들**을 `open -g`(앞으로 안 가져옴)로 띄우고 하트비트를 기다린다.
///
/// 깨우지 않는 경우 — 깨워 봐야 승인할 수 없거나, 깨울 대상을 모른다:
///   · 이미 살아 있다 → 할 일 없음.
///   · 화면이 죽었다(`ui_stalled`) → 이미 떠 있는 앱이다. `open` 은 그걸 고치지 못한다.
///   · 지갑이 아직 없다(none·legacy) → 앱은 하트비트를 일부러 안 찍는다 — 20초를 헛기다린다.
///   · 번들 밖에서 돈다(dev) → 어느 앱을 띄울지 모른다. 엉뚱한 설치본을 깨우지 않는다.
///
/// 돌려주는 값 = 지금 승인할 앱이 있는가(`app_alive`). false 면 호출자가 `app_unavailable` 로 안내한다.
pub async fn ensure_app_alive() -> bool {
    if app_alive() {
        return true;
    }
    if ui_stalled() {
        return false;
    }
    if !crate::wallet::wallet_status().is_ok_and(|s| s.state == "encrypted") {
        return false;
    }
    let Some(app) = std::env::current_exe().ok().as_deref().and_then(bundle_of) else {
        return false;
    };
    // /usr/bin/open 절대경로 — PATH 의 가짜 open 을 타지 않게(mcpb 런처와 같은 원칙). stdout 은 MCP 의
    // stdio 채널이라 자식에게 물려주면 JSON-RPC 가 깨진다 → 셋 다 닫는다.
    let spawned = std::process::Command::new("/usr/bin/open")
        .arg("-g")
        .arg(&app)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !spawned {
        return false;
    }
    eprintln!("[budu] 지갑 앱이 꺼져 있어 깨웠어요: {}", app.display());
    let deadline = std::time::Instant::now() + WAKE_WAIT;
    while std::time::Instant::now() < deadline {
        if app_alive() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    app_alive()
}

/// 이미 대기 중인 요청이 있는지 (single-flight 가드).
pub fn has_pending() -> bool {
    request_path().map(|p| p.exists()).unwrap_or(false)
}

/// 새 요청 id — 유닉스 나노초. 로컬 single-flight 환경엔 충분히 고유하다.
pub fn new_id() -> String {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
        .to_string()
}

/// x402 솔라나 결제 요청을 파일에 쓴다 (kind=`x402-svm`, USDC 고정).
/// amount 는 십진 USDC 문자열(승인 창 표시), resource 는 결제 대상 URL, terms 는 GUI 가 조립할 결제 조건.
pub fn write_x402_request(
    to: &str,
    amount: &str,
    memo: &str,
    resource: &str,
    picked_chain: u64,
    terms: crate::policy::SvmTerms,
    job: &str,
) -> Result<String, String> {
    write_request(
        "USDC",
        to,
        amount,
        memo,
        crate::policy::KIND_X402,
        resource,
        Some(picked_chain),
        Some(terms),
        job,
    )
}

/// 요청 파일 작성기 — single-flight 로 요청 하나를 내건다. 반환 = 요청 id.
/// `pin` = 요구를 고른 체인. 지금 활성 체인과 다르면 쓰지 않는다.
#[allow(clippy::too_many_arguments)]
fn write_request(
    token: &str,
    to: &str,
    amount: &str,
    memo: &str,
    kind: &str,
    resource: &str,
    pin: Option<u64>,
    svm: Option<crate::policy::SvmTerms>,
    job: &str,
) -> Result<String, String> {
    let id = new_id();
    let chain = crate::chain::active_chain();
    let chain_id = chain.chain_id;
    // 🔴 x402 는 **요구를 고른 체인**과 지금 각인할 체인이 같아야 한다 — 각인하는 값과 대조하는 값이
    // 같은 한 번의 읽기다. 호출자가 따로 대조하고 여기서 다시 읽으면, 그 사이 전환된 체인이 각인되어 테스트넷 요구가
    // 메인넷 요청으로 나갈 수 있었다.
    if pin.is_some_and(|p| p != chain_id) {
        return Err(ts!(
            "결제를 준비하는 사이 네트워크가 바뀌었어요. 아무것도 결제하지 않았습니다 — 다시 시도하세요.",
            "The network changed while this payment was being prepared. Nothing was paid — try again."
        )
        .into());
    }
    // 활성 계정 각인. 여기서 못 읽으면(지갑 파일 없음·깨짐) 요청을 만들지 않는다 —
    // 어느 계정에서 나갈지 모르는 결제를 사람 앞에 띄우지 않는다.
    let account = crate::wallet::active_account()?;
    let req = PaymentRequest {
        id: id.clone(),
        token: token.to_string(),
        to: to.to_string(),
        amount: amount.to_string(),
        memo: memo.to_string(),
        created: now_secs(),
        kind: kind.to_string(),
        resource: resource.to_string(),
        chain_id,               // 요청 시점 활성 체인 각인(승인 시 GUI가 대조)
        account: account.index, // 요청 시점 활성 계정 각인(승인 시 GUI가 대조)
        from: account.address,
        lease: true,
        svm,
        job: crate::policy::clip_job(job),
    };
    let json = serde_json::to_string_pretty(&req)
        .map_err(|e| tf!("직렬화 실패: {e}", "Couldn't serialize the request: {e}"))?;

    // 원자적 single-flight 획득: 요청 파일을 create_new(O_EXCL)로 만든다. 이미 있으면(대기 중) 거절.
    // has_pending() 사전검사와 파일 쓰기 사이의 경합(동시 호출 둘 다 통과해 한쪽 유실)을 닫는다.
    claim_request_file(&request_path()?, json.as_bytes())?;

    // 슬롯을 확보한 뒤에야 이전 요청의 결과 파일 잔재를 치운다(새 폴링이 옛 결과를 잡지 않게).
    if let Ok(p) = result_path() {
        let _ = fs::remove_file(p);
    }
    Ok(id)
}

/// 요청 파일을 create_new(O_EXCL)로 원자적으로 만들어 single-flight 슬롯을 획득한다.
/// 이미 존재하면(다른 요청이 대기 중) AlreadyExists → 사용자에게 안내.
fn claim_request_file(path: &PathBuf, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    // 🔴 **다 쓴 뒤에 등록한다**. 예전엔 자리(create_new)를 먼저 잡고 그 안에 썼다 — 쓰는 도중
    // 강제 종료되면 반쪽 파일이 자리에 남아, MCP 는 「대기 중」이라 새 요청을 계속 거절하고 GUI 는 못 읽어 안 치웠다.
    // 이제 옆의 임시 파일에 다 쓰고 `hard_link` 로 자리에 건다 — 링크는 자리가 차 있으면 실패하니(O_EXCL 과 같다)
    // single-flight 는 그대로이고, 자리에 놓이는 건 언제나 완성된 파일이다.
    let tmp = path.with_file_name(format!(
        ".{}.{}.{}.tmp",
        path.file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default(),
        std::process::id(),
        new_id()
    ));
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let written = opts.open(&tmp).and_then(|mut f| {
        f.write_all(bytes)?;
        f.sync_all()
    });
    if let Err(e) = written {
        let _ = fs::remove_file(&tmp);
        return Err(tf!(
            "요청 파일 저장 실패: {e}",
            "Couldn't write the request file: {e}"
        ));
    }
    let linked = fs::hard_link(&tmp, path);
    let _ = fs::remove_file(&tmp);
    match linked {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(ts!("이미 승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 다시 요청하세요.", "A payment is already waiting for approval. Let the user handle it, then ask again.").into())
        }
        Err(e) => Err(tf!(
            "요청 파일 생성 실패: {e}",
            "Couldn't create the request file: {e}"
        )),
    }
}

/// 내 요청 파일을 거둔다(파일 안의 id 가 내 것일 때만) — 경로를 받는다. **경로를 안에서 구하면 테스트가 실지갑
/// (`~/.budu`)을 건드리게 되므로** 여기를 갈라 두고 테스트는 임시 폴더를 넘긴다
/// (`claim_request_file` 과 같은 이음매). 파일 안의 id 가 내 것일 때만 지운다.
///
/// 확인과 삭제를 한 걸음으로(`policy::remove_request_if_mine`) — 둘 사이에 남의 새 요청이 들어와도 안 지운다.
fn cancel_request_at(path: &Path, id: &str) {
    crate::policy::remove_request_if_mine(path, id);
}

/// 대기 중에 이 프로세스가 사라지면 내 요청을 거둔다.
///
/// AI 클라이언트(Claude 등)가 승인 대기 중에 죽으면 stdin 이 닫혀 MCP 서버도 따라 죽는다
///. 그때 요청 파일을 그냥 두면 **기다리는 사람이 아무도 없는 승인 창**이
/// 최대 6분(GUI `is_stale` 의 5분 + 유예 60초) 화면에 남는다. 사용자가 그걸 승인하면 돈은 나가는데
/// 결과를 받을 상대가 없고, 다시 켠 AI 는 「아까 결제가 실패했다」고 알고 있어 한 번 더 요청한다
/// → 이중 결제. 요청을 거두면 승인 창이 곧바로 닫히고(GUI 의 `live_request()`
/// 가 파일에서 파생된다) 아무 일도 안 난다.
///
/// 시간 초과 때 `await_result` 가 하는 것과 **같은 함수**를 부른다 — `cancel_request_at` 은 파일 안의 id 가
/// 내 것일 때만 지우므로 남의 요청은 건드리지 않는다.
///
/// **거두기가 못 하는 일과, 그래도 안전한 이유**:
/// GUI 가 이미 `begin_approval` 을 지나 서명·전송 중이면 이 삭제는 그 결제를 멈추지 못한다 —
/// 돈은 나가고 GUI 의 `resolve_request` 는 요청이 사라진 걸 보고 결과를 조용히 버린다(죽은
/// 클라이언트는 어느 쪽이든 결과를 못 받는다). 문제는 그 사이 슬롯이 비어 **새 요청 B 가
/// 생길 수 있다**는 것이었다(전송은 RPC 가 느리면 30초까지). 개발 59 엔 GUI 가 B 의 승인을
/// A 와 겹쳐 시작할 수 있어 자율 승인이면 사람 없이 두 건이 나갔다 → 되돌렸다. 개발 63 부터
/// GUI 의 `begin_approval` 이 **진행 중인 승인이 있으면 두 번째 승인을 거절**하므로(ipc.rs),
/// B 는 A 가 끝날 때까지 자율로도 수동으로도 시작되지 않는다.
///
/// ⚠️ 닫히는 것은 **겹치는 창**뿐이다. A 가 끝난 **뒤** 다시 켜진 AI 가 같은 결제를 또
/// 요청하면(순차 재시도) 그건 한도·신뢰 주소·승인 창이 막는 몫이고, 이 가드와 무관하다.
///
/// SIGKILL 로 이 프로세스가 죽으면 Drop 은 안 돈다 — 그건 원래도 그랬고 막을 방법이 없다.
/// 그렇게 남은 고아 요청은 `has_pending()` 이 **존재만** 보므로 그 뒤 모든 결제를 막는데,
/// 개발 63 부터 **GUI 의 감시 스레드가 승인 창 시간(5분+유예)을 넘긴 요청 파일을 지운다**
/// (`src-tauri/src/ipc.rs` watchdog). 개발 63 이전엔 그 청소가 어디에도 없어서 — `is_stale`
/// 은 보여줄지 말지를 거르기만 한다 — 터미널에서 손으로 지우는 수밖에 없었다.
struct CancelOnDrop<'a> {
    id: &'a str,
    /// 거둘 요청 파일. 경로를 들고 있어야 테스트가 임시 폴더로 이 가드를 그대로 시험할 수
    /// 있다(`cancel_request_at` 과 같은 이음매). 실제 경로는 `await_result` 가 넘긴다.
    path: &'a Path,
    /// 정상 종료(결과 수신·시간 초과)면 내려서 Drop 이 아무것도 안 하게 한다.
    armed: bool,
}

impl Drop for CancelOnDrop<'_> {
    fn drop(&mut self) {
        if self.armed {
            cancel_request_at(self.path, self.id);
        }
    }
}

/// 결과를 timeout까지 폴링한다. 오면 Some(소비 후 파일 정리), 끝내 아무것도 안 나갔으면 None.
///
/// 🔴 **시간 초과 = 「응답 없음」이 아니다**. 사람이 5분이 끝나 갈 무렵
/// 비번을 넣으면 GUI 는 전송 중인데, 예전엔 여기서 요청을 거두고 None 을 돌려 「사용자가 응답하지
/// 않았어요」가 나갔다 — 돈은 나가고 AI 는 재시도한다. 이제 시간이 다 되면:
///   ① 요청을 **먼저** 거두고(더는 새 승인이 시작되지 않는다) ② GUI 의 결제 시도 기록을 읽는다.
/// 순서가 중요하다 — GUI 는 기록을 먼저 쓰고 요청을 보므로(shared/policy.rs 규약), 전송을 시작한 승인은
/// 반드시 기록으로 보인다. 기록이 「처리 중」이면 `LATE_SEND_WAIT` 까지 더 기다리고, 그래도 안 끝나면
/// status "unknown" 으로 돌려준다. None 은 **아무것도 안 나갔을 때만** 이다(시작된 적 없음·확실한 실패).
///
/// 이 함수의 future 가 **완료되기 전에 버려지면**(= 클라이언트가 죽어 런타임이 내려가면)
/// 대기 중이던 요청을 거둔다 — 위 `CancelOnDrop` 참고.
/// 돌려주는 값 = (결과, 앱이 꺼져서 일찍 거뒀나). 둘째 값으로 «사용자가 응답 안 함» 과 «앱이 꺼짐» 을 가른다.
pub async fn await_result(id: &str, timeout: Duration) -> (Option<PaymentResult>, bool) {
    // 경로를 못 구하는 상황(홈 디렉터리 없음)은 애초에 요청도 못 썼다는 뜻이라, 거둘 것도 없다.
    let dir = budu_dir().unwrap_or_default();
    let gone = std::sync::atomic::AtomicBool::new(false);
    let r = await_result_in(&dir, id, timeout, LATE_SEND_WAIT, &|| !app_alive(), APP_GONE_GRACE, &gone).await;
    (r, gone.load(std::sync::atomic::Ordering::Relaxed))
}

/// 기다리는 동안 앱 하트비트가(신선도 10초가 지난 뒤로) 이만큼 **더** 끊겨 있으면 — 실제로는 앱이 죽고 약 30초 뒤 — 5분을 다 채우지 않고 거둔다(실측 —
/// 승인 직후 앱이 죽었는데 5분 뒤 «사용자가 응답하지 않았어요» 로 답했다). 하트비트 신선도(10초)보다 넉넉히.
pub const APP_GONE_GRACE: Duration = Duration::from_secs(20);

/// `await_result` 의 속알맹이 — 데이터 디렉터리와 늦은 전송 대기를 받는다. **경로를 안에서 구하면 테스트가
/// 실지갑(`~/.budu`)을 건드리게 되므로** 갈라 둔다(`cancel_request_at` 과 같은 이음매).
async fn await_result_in(
    dir: &Path,
    id: &str,
    timeout: Duration,
    late_wait: Duration,
    app_gone: &(dyn Fn() -> bool + Sync),
    gone_grace: Duration,
    withdrew_for_gone: &std::sync::atomic::AtomicBool,
) -> Option<PaymentResult> {
    let req_path = dir.join("payment_request.json");
    let res_path = dir.join("payment_result.json");
    let mut guard = CancelOnDrop {
        id,
        path: &req_path,
        armed: true,
    };
    let start = SystemTime::now();
    let mut last_touch = start;
    let mut gone_since: Option<SystemTime> = None;
    loop {
        if let Some(r) = take_result(&res_path, id) {
            guard.armed = false;
            return Some(r);
        }
        let elapsed = SystemTime::now().duration_since(start).unwrap_or(timeout);
        if elapsed >= timeout {
            break;
        }
        // 앱이 죽었으면 일찍 거둔다 — 거두는 길은 시간 초과와 **똑같다**(요청을 먼저 지우고 결제 시도 기록을 본다).
        // 그래서 앱이 전송 중에 죽었으면(`sending`) 「안 나감」 이 아니라 늦은 대기 끝에 「불명」 으로 답한다.
        if app_gone() {
            let since = *gone_since.get_or_insert_with(SystemTime::now);
            if SystemTime::now().duration_since(since).unwrap_or_default() >= gone_grace {
                eprintln!("[budu] 요청 {id}: 지갑 앱 하트비트가 끊겼다 — 일찍 거둔다");
                withdrew_for_gone.store(true, std::sync::atomic::Ordering::Relaxed);
                break;
            }
        } else {
            gone_since = None;
        }
        // 임대 갱신 — 「아직 기다리고 있다」를 GUI 에 알린다. 이 프로세스가 SIGKILL 로 죽으면 갱신이
        // 멎고, GUI 는 곧 그 요청을 승인하지 못하게 된다(`policy::lease_lapsed`).
        let since_touch = SystemTime::now()
            .duration_since(last_touch)
            .unwrap_or_default();
        if since_touch >= Duration::from_secs(crate::policy::LEASE_TOUCH_SECS) {
            last_touch = SystemTime::now();
            touch_if_mine(&req_path, id);
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
    // ① 거두기 — 여기서부터 GUI 는 이 요청의 새 승인을 시작하지 못한다.
    cancel_request_at(&req_path, id);
    guard.armed = false;
    // 어느 갈래로 끝났는지 남긴다. stdout 은 MCP 프로토콜이라 stderr.
    eprintln!("[budu] 요청 {id}: 승인 대기 시간이 다 됐다 — 거두고 결제 시도 기록을 본다");
    // ② 기록 읽기. 거두기 직전에 결과가 들어왔을 수도 있으니 결과 파일도 같이 본다.
    let late_start = SystemTime::now();
    loop {
        if let Some(r) = take_result(&res_path, id) {
            eprintln!("[budu] 요청 {id}: 거두기 직전에 온 결과 파일로 끝남");
            return Some(r);
        }
        // 기록이 있는데 못 읽으면 「아무것도 안 나감」이 아니라 「아직 처리 중」으로 본다 —
        // 깨진 게 `sending`·`done` 이었을 수 있다. 끝내 못 읽으면 늦은 대기 끝에 불명으로 답한다.
        let (rec, verdict) = match crate::policy::read_attempt_at(dir, id) {
            Ok(rec) => {
                let v = crate::policy::after_timeout(rec.as_ref());
                (rec, v)
            }
            Err(_) => (None, crate::policy::AfterTimeout::StillSending),
        };
        match verdict {
            crate::policy::AfterTimeout::NothingSent => {
                eprintln!("[budu] 요청 {id}: 아무것도 안 나감으로 끝남");
                return None;
            }
            crate::policy::AfterTimeout::Finished => {
                eprintln!("[budu] 요청 {id}: 결제 시도 기록에서 결말을 받음(늦은 전송 갈래)");
                return rec.map(PaymentResult::from_record);
            }
            crate::policy::AfterTimeout::StillSending => {
                let waited = SystemTime::now()
                    .duration_since(late_start)
                    .unwrap_or(late_wait);
                if waited >= late_wait {
                    eprintln!("[budu] 요청 {id}: 늦은 전송을 기다리다 끝남 — 불명");
                    return Some(PaymentResult::still_sending(id, rec));
                }
            }
        }
        tokio::time::sleep(Duration::from_millis(700)).await;
    }
}

/// 요청 파일이 **내 것일 때만** 수정 시각을 지금으로 바꾼다. 없는 파일은 만들지 않는다 — 결과가 나와 GUI 가
/// 지운 요청을 되살리면 single-flight 가 막힌다. (읽기와 갱신 사이에 남의 요청으로 바뀌면 그 요청의 임대를
/// 한 번 늘려 줄 뿐이다 — 그쪽도 살아 있는 요청이라 해가 없다.)
fn touch_if_mine(path: &Path, id: &str) {
    let mine = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<PaymentRequest>(&s).ok())
        .is_some_and(|r| r.id == id);
    if mine {
        if let Ok(f) = fs::OpenOptions::new().write(true).open(path) {
            let _ = f.set_modified(SystemTime::now());
        }
    }
}

/// 내 결과가 와 있으면 가져가고 결과 파일을 치운다.
fn take_result(path: &Path, id: &str) -> Option<PaymentResult> {
    let r = fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str::<PaymentResult>(&s).ok())
        .filter(|r| r.id == id)?;
    let _ = fs::remove_file(path);
    Some(r)
}


#[cfg(test)]
mod tests;
