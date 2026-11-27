// 거래 내역 + x402 정산 추적.
//
// 모든 송금/서명 시도(성공·차단·실패)를 ~/.budu/history.json 에 남긴다 (감사 로그).
// x402 정산 tx 해시는 MCP만 안다(GUI가 서명 → MCP가 제출 → 페이실리테이터 온체인 정산 →
// MCP가 PAYMENT-RESPONSE로 받음). MCP가 ~/.budu/x402_settlements.json 에 {nonce, tx, success}를
// 기록하면, GUI 폴링이 읽어 매칭되는 "signed" 내역(detail=nonce)을 "settled"+tx 로 갱신한다.

use serde::Deserialize;
use std::fs;
use std::path::PathBuf;

use crate::chain::chain_file;
use crate::settings::redact_urls;
use crate::store::{budu_dir, now_secs, write_json};
use crate::wallet::{account_file, account_file_name};

/// 🔴 **내역 파일의 읽기-수정-쓰기를 한 줄로 세운다**. 내역은 통째로 읽어 한 줄 넣고
/// 통째로 쓴다 — 두 송금이 거의 같이 끝나면 둘 다 같은 목록을 읽고, 나중에 쓴 쪽이 먼저 쓴 기록을 지운다.
/// 정산 반영(`apply_x402_settlements`)도 같은 파일을 고친다. 내역을 쓰는 건 이 프로세스(GUI)뿐이라
/// 프로세스 안 잠금이면 된다. 독살(poison)돼도 계속 쓴다 — 기록을 멈추는 편이 더 나쁘다.
static HISTORY_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn history_guard() -> std::sync::MutexGuard<'static, ()> {
    HISTORY_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 송금 시도 1건의 기록 — 형식의 정본은 `policy::HistoryEntry`(MCP·CLI 가 같은 타입으로 읽는다).
pub(crate) use crate::policy::HistoryEntry;

/// 내역 본 파일이 품는 최신 기록 수 — 넘치면 보관 파일로 옮긴다(`policy::HISTORY_HOT_CAP`).
const HISTORY_CAP: usize = crate::policy::HISTORY_HOT_CAP;

/// 활성 계정(작업이 고정했으면 그 계정)의 내역 파일 (체인별 + 계정별).
/// 내역은 주소의 것이다 — 계정 2 의 화면에 계정 1 의 송금이 보이면 안 된다.
fn history_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(account_file("history")))
}

/// 특정 계정의 내역 파일 — x402 정산 반영이 모든 계정을 훑을 때 쓴다.
fn history_path_for(index: u32) -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(account_file_name(&chain_file("history"), index)))
}

fn read_history_at(path: &PathBuf) -> Vec<HistoryEntry> {
    fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// 내역 파일이 **있는데** 못 읽거나 깨졌는가 — 없는 것(아직 기록 없음)과 가른다.
fn history_unreadable(path: &PathBuf) -> bool {
    match fs::read_to_string(path) {
        Ok(s) => serde_json::from_str::<Vec<HistoryEntry>>(&s).is_err(),
        Err(e) => e.kind() != std::io::ErrorKind::NotFound,
    }
}

/// 새 기록을 맨 앞에 넣고, cap 을 넘친 오래된 기록을 떼어 돌려준다 (순수 함수 — 파일 I/O 없음, 테스트용).
/// 반환 = (본 파일에 남길 목록, 보관 파일로 옮길 기록 — 최신순).
fn with_entry(
    mut list: Vec<HistoryEntry>,
    entry: HistoryEntry,
    cap: usize,
) -> (Vec<HistoryEntry>, Vec<HistoryEntry>) {
    list.insert(0, entry);
    let evicted = if list.len() > cap {
        list.split_off(cap)
    } else {
        Vec::new()
    };
    (list, evicted)
}

/// 보관 파일에 쓸 줄들 — 넘겨받은 기록(최신순)을 **오래된 것부터** 한 줄씩. 순수 함수(테스트용).
/// `tail` = 보관 파일 끝의 기록들(오래된 순). 쓸 기록의 **앞부분이 보관 파일의 끝부분과 겹치면** 그만큼 뺀다 —
/// 덧붙인 뒤 본 파일을 쓰기 전에 죽으면 그 기록들이 본 파일에도 남아, 다음 번에 또 밀려나며 두 번 적힌다.
/// 🔴 한때 마지막 한 줄만 비교했다 — 보관에 실패해 본 파일이 cap 을 넘긴 뒤 여러 건을 한꺼번에
/// 옮기다 죽으면, 첫 기록이 마지막 줄과 달라 겹친 구간 전체가 다시 적혔다.
/// 개발 71 부터 기록마다 고유 `id` 가 있어 「모든 필드가 같은 두 기록」이 없다 — 같은 초·같은 금액·같은 사유가 연달아도
/// 서로 다른 기록으로 본다. id 가 없는 옛 기록끼리는 여전히 하나로 볼 수 있다.
fn archive_lines(evicted: &[HistoryEntry], tail: &[HistoryEntry]) -> String {
    let oldest_first: Vec<&HistoryEntry> = evicted.iter().rev().collect();
    let overlap = (1..=oldest_first.len().min(tail.len()))
        .rev()
        .find(|&m| {
            tail[tail.len() - m..]
                .iter()
                .zip(&oldest_first[..m])
                .all(|(a, b)| a == *b)
        })
        .unwrap_or(0);
    let mut out = String::new();
    for e in &oldest_first[overlap..] {
        if let Ok(line) = serde_json::to_string(e) {
            out.push_str(&line);
            out.push('\n');
        }
    }
    out
}

/// 보관 파일 끝의 기록들(오래된 순) — 끝의 256KB 만 읽는다(보관 파일은 몇 MB 까지 커질 수 있다).
/// 자른 자리가 한글 글자 중간일 수 있어 손실 허용 변환으로 읽는다(`read_to_string` 은 거기서 통째로 실패했다).
/// 잘린 첫 줄은 JSON 이 아니라 걸러진다.
fn archive_tail(path: &std::path::Path) -> Vec<HistoryEntry> {
    read_archive_tail(path, false)
        .map(|(v, _)| v)
        .unwrap_or_default()
}

/// `archive_tail` + 「없음」과 「있는데 못 읽음」을 가르고, 읽은 끝부분이 **파일 전체였는지**도 돌려준다
/// — 안전장치(자율 결제 중복 검사)는 못 읽었거나 앞부분을 안 본 것을 「없었다」로 치면 안 된다.
/// 없으면 Ok((빈 목록, true)), 있는데 못 읽으면 Err. **깨진 줄이 하나라도 있어도 Err** —
/// 방금 나간 결제의 줄이 깨져 있으면 건너뛴 목록은 「그 결제 없음」이 된다. 단 256KB 로 자른 자리의 첫 줄은
/// 원래 반쪽이라 봐준다(파일 전체를 읽었으면 첫 줄도 온전해야 한다).
fn archive_tail_checked(path: &std::path::Path) -> Result<(Vec<HistoryEntry>, bool), ()> {
    read_archive_tail(path, true)
}

fn read_archive_tail(path: &std::path::Path, strict: bool) -> Result<(Vec<HistoryEntry>, bool), ()> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = match fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok((Vec::new(), true)),
        Err(_) => return Err(()),
    };
    let len = f.metadata().map_err(|_| ())?.len();
    let start = len.saturating_sub(256 * 1024);
    f.seek(SeekFrom::Start(start)).map_err(|_| ())?;
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes).map_err(|_| ())?;
    let text = String::from_utf8_lossy(&bytes);
    let mut list = Vec::new();
    for (i, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str(line) {
            Ok(e) => list.push(e),
            Err(_) if !strict || (i == 0 && start > 0) => {}
            Err(_) => return Err(()),
        }
    }
    Ok((list, start == 0))
}

/// 밀려난 기록을 보관 파일 끝에 덧붙인다. 한 번의 쓰기로(줄 사이에서 끊기지 않게).
/// 앞선 쓰기가 줄 중간에 죽어 파일이 개행으로 안 끝나면 개행부터 — 새 줄이 반쪽 줄에 붙어 같이 버려지지 않게.
fn append_archive(path: &std::path::Path, evicted: &[HistoryEntry]) -> Result<(), String> {
    use std::io::{Read, Seek, SeekFrom, Write};
    let mut body = archive_lines(evicted, &archive_tail(path));
    if body.is_empty() {
        return Ok(());
    }
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .read(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let len = f.metadata().map_err(|e| e.to_string())?.len();
    if len > 0 {
        let mut last = [0u8; 1];
        f.seek(SeekFrom::Start(len - 1))
            .map_err(|e| e.to_string())?;
        f.read_exact(&mut last).map_err(|e| e.to_string())?;
        if last[0] != b'\n' {
            body.insert(0, '\n');
        }
    }
    f.write_all(body.as_bytes()).map_err(|e| e.to_string())?;
    f.sync_all().map_err(|e| e.to_string())
}

/// 송금 시도 1건을 내역에 추가한다. 실패해도 송금 흐름은 막지 않는다(로그는 부가 기능).
///
/// 🔴 두 가지 조용한 유실을 막는다:
/// - **200건 넘친 기록** — 예전엔 잘라 버렸다. 이제 보관 파일에 덧붙이고, 덧붙이기가 실패하면 **자르지 않는다**
///   (본 파일이 잠시 200건을 넘을 뿐, 다음 기록 때 다시 옮긴다).
/// - **깨진 본 파일** — 예전엔 빈 목록으로 읽고 새 한 건으로 덮어써 옛 기록을 통째로 지웠다(개발 69 입금 기록과 같은
///   병). 이제 깨진 파일은 `….broken.<초>.json` 으로 옆에 치우고 새로 시작한다 — 기록은 멈추지 않고 옛 파일은 남는다.
///   치우지도 못하면(권한 등) 이번 기록을 포기한다 — 덮어쓰는 것보다 낫다.
///
/// 기록마다 고유 번호(`id`)와 주인 주소(`from` — 작업이 고정한 계정)를 싣는다.
pub(crate) fn log_attempt(token: &str, to: &str, amount: &str, status: &str, detail: &str) {
    log_entry(token, to, amount, status, detail);
}

/// `log_attempt` + 결제 대상 URL — 승인 창 전 한도 검사가 막은 x402 결제처럼 무엇을 사려 했는지가 남아야 할 때.
pub(crate) fn log_attempt_for(
    token: &str,
    to: &str,
    amount: &str,
    status: &str,
    detail: &str,
    resource: &str,
    job: &str,
) {
    let extra = RowExtra {
        resource,
        job,
        ..Default::default()
    };
    let _ = log_entry_with(token, to, amount, status, detail, false, extra);
}

/// 기록 한 줄에 덧붙는 칸들 — 인자가 계속 늘어 순서 실수가 날 자리라 이름으로 넘긴다.
#[derive(Default, Clone, Copy)]
pub(crate) struct RowExtra<'a> {
    /// 한도를 예약한 날. 0 = 예약 없음.
    pub(crate) reserved_day: u64,
    /// 결제 대상 URL(표시용).
    pub(crate) resource: &'a str,
    /// 작업 이름표.
    pub(crate) job: &'a str,
    /// 서명한 tx 의 마지막 유효 블록 높이. 0 = 서명 아님.
    pub(crate) valid_until: u64,
    /// 블록해시를 받은 slot. 0 = 모름.
    pub(crate) sign_slot: u64,
}

/// x402 서명 기록(「정산 대기」) — 예약한 날을 같이 싣는다. 만료로 확정되면 그 날에만 환불한다.
/// **디스크까지 적는다** — 서명은 내보내면 판매자가 정산할 수 있는 돈이라, 기록 없이 나가면 다시
/// 켠 뒤 같은 결제가 「처음」처럼 보인다. 반환은 `log_pending` 과 같다: `Err(Some)` = 줄은 제자리(디스크까지는 모름),
/// `Err(None)` = 못 적음. 호출자는 어느 쪽이든 서명을 내보내지 않는다.
pub(crate) fn log_signed(
    token: &str,
    to: &str,
    amount: &str,
    nonce: &str,
    extra: RowExtra<'_>,
) -> Result<String, Option<String>> {
    log_entry_with(token, to, amount, "signed", nonce, true, extra)
}

/// 한 줄을 적고 그 id 를 돌려준다(저장 실패면 None).
fn log_entry(token: &str, to: &str, amount: &str, status: &str, detail: &str) -> Option<String> {
    log_entry_with(token, to, amount, status, detail, false, RowExtra::default()).ok()
}

/// `durable` = 디스크까지 내린다 — **제출 직전의 「확인 필요」 줄**(`log_pending`)만 쓴다.
/// 보통 원자 쓰기는 rename 까지라, tx 를 낸 직후 전원이 나가면 이 줄이 사라질 수 있었다 — 체인엔 송금이 있는데
/// 내역·10분 중복 검사엔 없어서 같은 결제가 한 번 더 나갈 수 있다.
/// 디스크까지 못 내렸으면 실패로 돌려 제출하지 않는다. 디렉터리 동기화만 실패한
/// 드문 경우엔 줄이 제자리에 남아 「확인 필요」로 보인다 — 보수 쪽(돈은 안 나갔다).
fn log_entry_with(
    token: &str,
    to: &str,
    amount: &str,
    status: &str,
    detail: &str,
    durable: bool,
    extra: RowExtra<'_>,
) -> Result<String, Option<String>> {
    let entry = HistoryEntry {
        ts: now_secs(),
        token: token.into(),
        to: to.into(),
        amount: amount.into(),
        status: status.into(),
        detail: detail.into(),
        id: new_record_id(),
        from: crate::wallet::active_account()
            .map(|a| a.address)
            .unwrap_or_default(),
        reserved_day: extra.reserved_day,
        resource: crate::policy::clip_resource(extra.resource),
        job: crate::policy::clip_job(extra.job),
        valid_until: extra.valid_until,
        sign_slot: extra.sign_slot,
        ..Default::default()
    };
    // 디스크와 따로 메모리에도 — 저장이 실패해도 10분 중복 검사가 방금 나간 결제를 본다.
    remember_recent(&entry);
    let id = entry.id.clone();
    let _g = history_guard();
    let path = history_path().map_err(|_| None)?;
    match record_at_with(&path, entry, HISTORY_CAP, durable) {
        Ok(()) => Ok(id),
        Err(RecordError::InPlace(e)) => {
            eprintln!("[history] 기록을 디스크까지 못 내림: {e}");
            Err(Some(id))
        }
        Err(RecordError::NotWritten(e)) => {
            eprintln!("[history] 기록 실패: {e}");
            // 먼저 적기(durable)를 못 했으면 호출자는 내지 않는다 — 메모리 표식도 뺀다(안 낸 결제가
            // 10분 동안 「확인 필요」로 남아 같은 자율 결제를 사람에게 돌렸다). 뒤에 적는 기록(`durable` 아님)은 돈이 이미
            // 나간 뒤라 표식을 남긴다.
            if durable {
                forget_recent(&id);
            }
            Err(None)
        }
    }
}

/// `record_at_with` 의 실패 — 줄이 제자리에 갔는지가 뒷수습을 가른다.
enum RecordError {
    NotWritten(String),
    /// 새 내역은 제자리인데 디스크까지 못 내렸다(durable 쓰기만).
    InPlace(String),
}

impl From<String> for RecordError {
    fn from(e: String) -> Self {
        RecordError::NotWritten(e)
    }
}

/// 이 프로세스가 최근에 적으려 한 「나갔거나 나갔을 수 있는」 기록 — (체인, 기록). 10분 창만큼만 든다.
/// 중복 검사는 디스크 내역만 봤다 — 송금 뒤 내역 저장이 실패하면(디스크 꽉 참 등) 방금 나간 결제가 없는 셈이 되어
/// 같은 결제가 다시 자율로 나갔다. 자율 결제는 이 프로세스에서만 나가므로 메모리 사본이면 그 구멍을 닫는다.
static RECENT_PAID: std::sync::Mutex<Vec<(u64, HistoryEntry)>> = std::sync::Mutex::new(Vec::new());

fn remember_recent(entry: &HistoryEntry) {
    if !counts_as_paid(&entry.status) {
        return;
    }
    let chain = crate::chain::active_chain().chain_id;
    let mut list = RECENT_PAID.lock().unwrap_or_else(|e| e.into_inner());
    let now = now_secs();
    list.retain(|(_, e)| now.saturating_sub(e.ts) <= REPEAT_WINDOW_SECS);
    list.push((chain, entry.clone()));
}

fn forget_recent(id: &str) {
    RECENT_PAID
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .retain(|(_, e)| e.id != id);
}

fn recent_in_memory() -> Vec<HistoryEntry> {
    let chain = crate::chain::active_chain().chain_id;
    RECENT_PAID
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .filter(|(c, _)| *c == chain)
        .map(|(_, e)| e.clone())
        .collect()
}

/// 깨진 내역을 `window` 안에 옆으로 치웠나 — 치운 파일에 방금 나간 결제가 있었을 수 있는데
/// 새로 시작한 본 파일엔 없다. 치운 시각은 이름(`<본 이름>.broken.<초>.json`)에 있다.
fn set_aside_recently(hot: &std::path::Path, now: u64, window: u64) -> bool {
    let (Some(dir), Some(stem)) = (hot.parent(), hot.file_stem().and_then(|s| s.to_str())) else {
        return true; // 경로를 못 풀면 증명 못 함 — 사람에게
    };
    let Ok(rd) = fs::read_dir(dir) else {
        return true;
    };
    let prefix = format!("{stem}.broken.");
    rd.flatten().any(|e| {
        let name = e.file_name().to_string_lossy().into_owned();
        name.strip_prefix(&prefix)
            .and_then(|rest| rest.strip_suffix(".json"))
            .and_then(|secs| secs.parse::<u64>().ok())
            .is_some_and(|at| now.saturating_sub(at) <= window)
    })
}

/// 기록 고유 번호 — 나노초 시각 + 프로세스 + 프로세스 안 순번(같은 나노초·재시작에도 겹치지 않게).
fn new_record_id() -> String {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{nanos:x}-{:x}-{n:x}", std::process::id())
}

/// `log_attempt` 의 본체 — 경로를 받는다(테스트가 실지갑을 안 건드리게).
#[cfg(test)]
fn record_at(path: &PathBuf, entry: HistoryEntry, cap: usize) -> Result<(), String> {
    record_at_with(path, entry, cap, false).map_err(|e| match e {
        RecordError::NotWritten(m) | RecordError::InPlace(m) => m,
    })
}

fn record_at_with(
    path: &PathBuf,
    entry: HistoryEntry,
    cap: usize,
    durable: bool,
) -> Result<(), RecordError> {
    if history_unreadable(path) {
        let aside = path.with_extension(format!("broken.{}.json", now_secs()));
        fs::rename(path, &aside).map_err(|e| e.to_string())?;
    }
    let (mut list, evicted) = with_entry(read_history_at(path), entry, cap);
    if !evicted.is_empty()
        && append_archive(&crate::policy::history_archive_path(path), &evicted).is_err()
    {
        list.extend(evicted); // 옮기지 못했으면 자르지 않는다
    }
    if !durable {
        return Ok(write_json(path.clone(), &list)?);
    }
    let json = serde_json::to_string_pretty(&list).map_err(|e| e.to_string())?;
    crate::store::write_atomic_durable(path, json.as_bytes()).map_err(|e| match e {
        crate::store::DurableError::NotWritten(m) => RecordError::NotWritten(m),
        crate::store::DurableError::NotDurable(m) => RecordError::InPlace(m),
    })
}

/// detail 의 URL 을 가린다(출력용). 이번 패치 이전이 기록한 비redact 에러에 RPC URL·키가
/// 들어 있어도 GUI 로 다시 새지 않게 — 순수 함수라 파일 I/O 없이 검증 가능.
fn redact_details(mut list: Vec<HistoryEntry>) -> Vec<HistoryEntry> {
    for e in &mut list {
        e.detail = redact_urls(&e.detail);
    }
    list
}

/// 거래 내역을 최신순으로 돌려준다 — 보낸 기록에 **입금 기록**을 시각순으로 섞는다
/// (`policy::merge_received` — MCP·CLI 와 같은 함수).
///
/// `limit` = 돌려줄 줄 수(화면의 「더 보기」가 늘린다). 보낸 기록은 본 파일 다음에 보관 파일까지 읽는다.
#[tauri::command]
pub(crate) fn get_history(limit: Option<usize>) -> Vec<HistoryEntry> {
    let limit = limit.unwrap_or(HISTORY_CAP);
    // 주인이 다른 기록은 뺀다 — 지갑을 지우고 다른 시드를 가져오면 같은 파일 이름에 옛 기록이 남는다.
    let owner = crate::wallet::active_account()
        .map(|a| a.address)
        .unwrap_or_default();
    let sent = history_path()
        .map(|p| crate::policy::read_sent_history(&p, limit, &owner))
        .unwrap_or_default();
    let mut list = redact_details(sent);
    list.truncate(limit);
    list
}

// ── 같은 결제를 짧은 시간에 두 번 ────────────────────────────────────────────────────

/// 자율 결제를 사람 앞으로 돌리는 창 — 같은 받는 곳·같은 금액이 이 안에 이미 나갔으면(10분).
pub(crate) const REPEAT_WINDOW_SECS: u64 = 600;

/// 「돈이 나갔거나 나갔을 수 있는」 기록인가 — 차단·실패·되돌려짐·만료는 아니다.
fn counts_as_paid(status: &str) -> bool {
    // 「정산 실패」도 넣는다 — 서버 말만으로는 서명이 죽지 않는다. 체인 확인이 결말을 적을 때까지는 나갔을 수 있다.
    matches!(status, "sent" | "signed" | "settled" | "unknown" | "settle_failed")
}

/// 같은 토큰·받는 곳·금액의 결제가 `window` 안에 있었으면 몇 초 전인지 (순수 함수 — 테스트용).
/// 금액은 숫자로 비교한다("0.01" 과 "0.010" 은 같다). 기록 쪽 금액을 `parse` 가 못 읽으면 **같은 것으로 본다**
/// — 「다르다」로 치면 깨진 금액 한 칸이 중복 검사를 연다. 모르면 사람에게.
#[allow(clippy::too_many_arguments)]
fn last_same_payment(
    list: &[HistoryEntry],
    token: &str,
    to: &str,
    amount: &str,
    now: u64,
    window: u64,
    resource: Option<&str>,
    parse: impl Fn(&str) -> Option<u128>,
) -> Option<u64> {
    let want = parse(amount)?;
    // 결제 대상 URL 까지 같아야 「같은 결제」 로 보는 경우(작업 예산). 기록 쪽은 `clip_resource` 로 잘려 있다.
    let job_path = resource.is_some();
    let resource = resource.map(crate::policy::clip_resource);
    list.iter()
        .filter(|e| counts_as_paid(&e.status))
        // 작업 예산: 앞의 같은 결제가 **정산됐고 내용도 받았으면** 재시도가 아니라 새로 사는 것이다.
        // «받았다» 는 MCP 의 긍정 신호(`delivered`)로만 안다 — 표시가 없는 걸 받음으로 읽지 않는다.
        // (입항 직전 /wave 를 5분마다 새로 사는 정상 반복). 그건 창 없이 통과 — 무한 반복은 작업 예산 상한이 막는다(시간 제한은 마지막 사용부터라 꾸준한 반복엔 안 걸린다).
        // 내용 못 받음·정산 대기(서명만)·정산 실패·결과 모름은 «아까 실패했다» 고 믿은 AI 의 재시도일 수 있어 지금처럼 사람에게.
        // 자율 결제(resource 없음)는 이 예외가 없다 — 그쪽은 사람이 금액 상한만 정했지 «이 일» 을 승인한 게 아니다.
        .filter(|e| !(job_path && e.status == "settled" && e.delivered && !e.undelivered))
        .filter(|e| resource.as_deref().is_none_or(|r| e.resource == r))
        // 받는 주소는 정확히 같아야 같은 결제다 — base58 은 대소문자가 다르면 다른 주소.
        .filter(|e| e.token == token && e.to.trim() == to.trim())
        .filter(|e| e.ts <= now && now - e.ts <= window)
        .filter(|e| parse(&e.amount).is_none_or(|a| a == want))
        .map(|e| now - e.ts)
        .min()
}

/// 지금 계정(작업이 고정했으면 그 계정)의 기록 `list` 에서 `last_same_payment`.
fn same_payment_in(
    mut list: Vec<HistoryEntry>,
    token: &str,
    to: &str,
    amount: &str,
    resource: Option<&str>,
) -> Option<u64> {
    let owner = crate::wallet::active_account()
        .map(|a| a.address)
        .unwrap_or_default();
    list.retain(|e| crate::policy::history_owned_by(e, &owner));
    let dec = crate::chain::active_chain().usdc_decimals;
    last_same_payment(
        &list,
        token,
        to,
        amount,
        now_secs(),
        REPEAT_WINDOW_SECS,
        resource,
        |a| crate::limits::parse_usdc_nonneg(a, dec).ok(),
    )
}

/// 본 파일 + **보관 파일 끝부분**에서 같은 결제(승인 창의 「N분 전에…」 줄 — 보여 주기용이라 못 읽으면 없음).
/// 「10분 창이면 본 파일(최신 200건)로 충분하다」는 가정이었다 — AI 가 차단당하는 시도를
/// 10분 안에 200번 넘게 쌓으면 방금 나간 결제가 보관 파일로 밀려나, 같은 결제가 다시 자율로 나갔다.
pub(crate) fn recent_same_payment(token: &str, to: &str, amount: &str) -> Option<u64> {
    let hot = history_path().ok()?;
    let mut list = read_history_at(&hot);
    list.extend(archive_tail(&crate::policy::history_archive_path(&hot)));
    same_payment_in(list, token, to, amount, None)
}

/// 읽은 보관 끝부분이 10분 창을 다 덮는가 (순수 함수 — 테스트용). 파일 전체를 읽었으면 덮는다.
/// 끝부분만 읽었으면 그 가장 오래된 기록이 창 밖이어야 덮는다 — 보관 파일은 밀려난 순서(오래된 순)로 쌓이니
/// 그보다 앞은 더 오래됐다. 끝부분에 기록이 하나도 없으면 증명할 수 없다.
/// 창 밖은 **엄격히 넘어야**(`>`) 한다 — `last_same_payment` 는 정확히 창 길이 전도 창 안으로 세서, 가장 오래된 게
/// 딱 그 초면 잘린 앞쪽에 같은 초의 기록이 더 있을 수 있다.
fn archive_covers_window(tail: &[HistoryEntry], whole: bool, now: u64, window: u64) -> bool {
    whole
        || tail
            .iter()
            .map(|e| e.ts)
            .min()
            .is_some_and(|oldest| now.saturating_sub(oldest) > window)
}

/// 자율 결제의 문 — 같은 결제가 10분 안에 있었거나, **그걸 확인할 수 없으면** 사람에게 넘긴다.
/// 확인할 수 없는 셋: 본 파일이 있는데 못 읽음 · 보관 파일이 있는데 못 읽음 ·
/// 보관 파일이 커서 끝 256KB 에 10분 창이 다 안 들어옴.
/// 못 읽는 내역을 「중복 없음」으로 치면 안전장치가 조용히 열린다(「못 읽으면 빈 값」).
pub(crate) fn autopay_needs_human_for_repeat(token: &str, to: &str, amount: &str) -> bool {
    autopay_needs_human_for_repeat_of(token, to, amount, None)
}

/// `autopay_needs_human_for_repeat` + `resource` 가 있으면 결제 대상 URL 까지 같은 기록만 「같은 결제」 로 본다(작업 예산).
pub(crate) fn autopay_needs_human_for_repeat_of(
    token: &str,
    to: &str,
    amount: &str,
    resource: Option<&str>,
) -> bool {
    let Ok(hot) = history_path() else {
        return true;
    };
    if history_unreadable(&hot) || set_aside_recently(&hot, now_secs(), REPEAT_WINDOW_SECS) {
        return true;
    }
    let Ok((tail, whole)) = archive_tail_checked(&crate::policy::history_archive_path(&hot)) else {
        return true;
    };
    if !archive_covers_window(&tail, whole, now_secs(), REPEAT_WINDOW_SECS) {
        return true;
    }
    let mut list = read_history_at(&hot);
    list.extend(tail);
    // 메모리 사본은 서명 순간의 «정산 대기» 그대로다 — 디스크 기록이 정산·내용 받음으로 바뀌어도 사본은 안 바뀐다.
    // 작업 예산은 그 결말을 보고 판정하므로 디스크에 같은 id 가 있으면 디스크 쪽을 믿는다. 디스크에 없으면
    // (내역 저장 실패 — 사본이 있는 이유) 사본이 남아 사람에게 간다.
    let memory = recent_in_memory();
    if resource.is_some() {
        let on_disk: std::collections::HashSet<String> =
            list.iter().filter(|e| !e.id.is_empty()).map(|e| e.id.clone()).collect();
        list.extend(memory.into_iter().filter(|e| e.id.is_empty() || !on_disk.contains(&e.id)));
    } else {
        list.extend(memory);
    }
    same_payment_in(list, token, to, amount, resource).is_some()
}

/// 승인 창이 묻는다 — 「방금 같은 결제가 나갔나」(몇 초 전). 사람이 중복 결제를 알아보게 한 줄을 띄운다.
#[tauri::command]
pub(crate) fn recent_same_payment_secs(token: String, to: String, amount: String) -> Option<u64> {
    recent_same_payment(&token, &to, &amount)
}

// ── 체인 확인 결과 반영 (confirm.rs) ────────────────────────────────────────────────────

/// 보관 파일의 한 줄 — (원래 바이트, 풀린 기록). 못 읽는 줄은 기록이 None.
type ArchiveRow = (Vec<u8>, Option<HistoryEntry>);

/// 보관 파일의 줄들 — (원래 바이트, 풀린 기록). **바이트로** 읽는다: 덧붙이다 끊겨 한글이 잘린 줄이
/// 하나라도 있으면 `read_to_string` 이 통째로 실패해, 그 뒤의 멀쩡한 기록에 결말을 영영 못 적었다. 못 읽는 줄은 바이트 그대로
/// 두고 다시 쓴다. 없는 파일은 빈 목록.
fn read_archive_rows(path: &std::path::Path) -> Result<Vec<ArchiveRow>, String> {
    let raw = match fs::read(path) {
        Ok(r) => r,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    Ok(raw
        .split(|b| *b == b'\n')
        .filter(|l| !l.is_empty())
        .map(|l| {
            let e = std::str::from_utf8(l)
                .ok()
                .and_then(|t| serde_json::from_str(t).ok());
            (l.to_vec(), e)
        })
        .collect())
}

/// 우리 서명이 detail 에 든 x402 서명 기록의 상태들 — 정산 대기·정산·정산 실패·만료·체인 실패.
pub(crate) fn is_signature_row(status: &str) -> bool {
    matches!(status, "signed" | "settled" | "settle_failed" | "expired" | "reverted")
}

/// MCP가 기록한 정산 결과 1건. nonce 로 "signed" 내역과 매칭한다.
#[derive(Deserialize)]
struct Settlement {
    /// 서명 인가의 nonce("0x..") — history 의 detail 과 일치해야 매칭.
    nonce: String,
    /// 온체인 정산 tx 해시.
    tx: String,
    /// 페이실리테이터 정산 성공 여부.
    success: bool,
    /// 응답 내용을 끝까지 못 받았다. `tx` 가 비고 `success` 가 거짓이면 **표시만** 하는 기록 — 상태는 그대로 둔다.
    #[serde(default)]
    undelivered: bool,
    /// 내용을 끝까지 받았다 — MCP 가 성공 정산과 함께만 싣는 긍정 신호. 옛 MCP 는 안 실어 false.
    #[serde(default)]
    delivered: bool,
}

impl Settlement {
    /// 이 정산이 «내용 받음» 을 말하는가 — 받음과 못 받음이 같이 오면 못 받음이 이긴다.
    fn says_delivered(&self) -> bool {
        self.success && self.delivered && !self.undelivered
    }

    /// 상태는 건드리지 않고 「내용 못 받음」 만 붙이는 기록인가.
    fn mark_only(&self) -> bool {
        self.undelivered && self.tx.is_empty() && !self.success
    }
}

fn settlements_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(chain_file("x402_settlements")))
}

/// 정산 1건을 내역 목록에 적용한다 (순수 함수 — 테스트용).
/// nonce 가 일치하고 아직 "signed" 인 첫 항목을 status="settled"/"settle_failed" + settle_tx 로 갱신.
/// 적용됐으면 true.
///
/// 체인 확인(`confirm.rs`)이 먼저 「settled」 로 바꿔 둔 기록(정산 tx 모름)에도 늦게 온 성공 정산의 tx 를 채운다.
fn apply_settlement(list: &mut [HistoryEntry], s: &Settlement) -> bool {
    if s.mark_only() {
        // 우리 서명(detail)이 같은 서명 기록 아무거나 — 결말이 이미 적혔어도(만료 등) 표시는 남긴다(화면이 상태 보고 고른다).
        return match list.iter_mut().find(|e| e.detail == s.nonce && is_signature_row(&e.status)) {
            Some(e) => {
                e.undelivered = true;
                true
            }
            None => false,
        };
    }
    for e in list.iter_mut() {
        if s.success && e.status == "settled" && e.settle_tx.is_empty() && e.detail == s.nonce {
            e.settle_tx = s.tx.clone();
            e.undelivered |= s.undelivered;
            e.delivered = (e.delivered || s.says_delivered()) && !e.undelivered;
            return true;
        }
        // 체인 확인이 정산 tx 까지 먼저 채워 둔 기록 — 상태·tx 는 체인이 정했으니 그대로 두고 MCP 가 아는 것
        // (내용을 받았나)만 얹는다. 예전엔 위·아래 두 갈래 어디에도 안 걸려 「내용 못 받음」 이 버려지고 기록이 «받음» 처럼 남았다.
        // 바뀐 게 없으면 false — 같은 정산을 두 번 적용해도 «반영됨» 으로 세지 않는다(중복 방지 테스트).
        if e.status == "settled" && !e.settle_tx.is_empty() && e.detail == s.nonce {
            let before = (e.undelivered, e.delivered);
            e.undelivered |= s.undelivered;
            e.delivered = (e.delivered || s.says_delivered()) && !e.undelivered;
            return (e.undelivered, e.delivered) != before;
        }
        if e.status == "signed" && e.detail == s.nonce {
            e.undelivered |= s.undelivered;
            e.delivered = s.says_delivered() && !e.undelivered;
            e.status = if s.success {
                "settled"
            } else {
                "settle_failed"
            }
            .into();
            e.settle_tx = s.tx.clone();
            return true;
        }
    }
    false
}

/// 정산 파일을 고유한 이름으로 가져와(rename) 가져온 묶음 전부를 읽는다 — 전에 읽다 실패해 남겨 둔 묶음 포함.
/// 반환 = (정산들, 다 읽어서 지워도 되는 파일들). **경로를 받는다** — 테스트가 실지갑을 안 건드리게.
fn claim_settlements(path: &std::path::Path) -> (Vec<Settlement>, Vec<PathBuf>) {
    let Some(dir) = path.parent().map(PathBuf::from) else {
        return (Vec::new(), Vec::new());
    };
    let stem = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let claim_prefix = format!(".{stem}.claimed.");
    if fs::metadata(path).is_ok() {
        // 이름은 가져올 때마다 다르다(초 단위로 지으면 같은 초의 두 번째가 첫 번째를 덮었다).
        static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        // 프로세스 id 도 싣는다 — 순번은 프로세스마다 0 부터라, 같은 초에 앱이 재시작하면 남겨 둔 묶음을 덮는다.
        let name = format!("{claim_prefix}{}.{}.{n}", now_secs(), std::process::id());
        let _ = fs::rename(path, dir.join(name));
    }
    // 가져온 묶음 전부 — 방금 것과, 전에 **읽다 실패해 남겨 둔** 것(못 읽었다고 지우면 영영 잃는다).
    let mut settlements: Vec<Settlement> = Vec::new();
    let mut consumed: Vec<PathBuf> = Vec::new();
    for e in fs::read_dir(&dir).into_iter().flatten().flatten() {
        if !e.file_name().to_string_lossy().starts_with(&claim_prefix) {
            continue;
        }
        let Ok(raw) = fs::read_to_string(e.path()) else {
            continue; // 다음 폴링에 다시
        };
        // 못 읽는 JSON 도 바로 지우지 않는다 — MCP 가 가져가기 직전에 열어 둔 파일에 아직
        // 쓰는 중이면 반쪽이 읽힌다. 남겨 두면 다 쓰인 뒤 다음 폴링에 읽힌다. 단 마지막 수정이 60초를 넘었는데도
        // 못 읽으면 쓰기가 끝내 멈춘 것이라 지운다.
        let Ok(batch) = serde_json::from_str::<Vec<Settlement>>(&raw) else {
            let stale = e
                .metadata()
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_some_and(|age| age > std::time::Duration::from_secs(60));
            if stale {
                let _ = fs::remove_file(e.path());
            }
            continue;
        };
        settlements.extend(batch);
        consumed.push(e.path());
    }
    (settlements, consumed)
}

/// MCP가 남긴 x402 정산 결과를 읽어 내역에 반영한다 (GUI 1초 폴링). 반영 건수를 돌려준다.
/// 처리 후 정산 파일을 비운다(중복 적용 방지). 매칭 안 되는 건 그냥 버린다.
///
/// 정산 파일은 체인별 하나(계정 공용)인데 내역은 계정별이다. 서명한 계정과 정산이
/// 도착했을 때의 활성 계정이 다를 수 있으므로(서명 → 사용자가 계정 전환 → 페이실리테이터 정산),
/// 활성 계정부터 보고 안 맞으면 **나머지 계정의 내역까지 훑는다** — 안 그러면 그 「signed」 항목이
/// 영영 정산 대기로 남는다.
#[tauri::command]
pub(crate) fn apply_x402_settlements() -> u32 {
    let path = match settlements_path() {
        Ok(p) => p,
        Err(_) => return 0,
    };
    // 🔴 **읽기 전에 파일을 통째로 가져온다(rename)**. 예전엔 읽고 → 반영하고 → 지웠다.
    // 그 사이 MCP 가 새 정산을 덧붙이면 **읽지 않은 그 건까지 지웠고**, 그 내역은 영영 「정산 대기」로 남았다.
    // rename 은 원자적이다 — 가져간 뒤 MCP 가 쓰는 건 새 파일로 가서 다음 폴링에 반영된다.
    // (MCP 가 가져가기 전 목록을 읽고 가져간 뒤에 쓰면 옛 건이 새 파일에 한 번 더 실리는데, 이미 「signed」가
    // 아니라서 다시 매칭되지 않는다 — 두 번 반영되지 않는다.)
    // 잠금은 **가져오기 전에** 잡는다 — 두 폴링이 겹쳐도 가져오기·읽기·반영이 한 줄로 선다.
    let _g = history_guard();
    let (settlements, consumed) = claim_settlements(&path);
    if consumed.is_empty() {
        return 0; // 처리할 정산 없음 (대부분의 폴링)
    }
    // 활성 계정 먼저, 그다음 나머지 — 대부분은 첫 파일에서 끝난다.
    let active = crate::wallet::active_account_index();
    let mut indices: Vec<u32> = vec![active];
    if let Ok(w) = crate::wallet::read_encrypted() {
        indices.extend(
            w.accounts()
                .iter()
                .map(|a| a.index)
                .filter(|i| *i != active),
        );
    }
    let all_indices = indices.clone();
    let mut pending: Vec<&Settlement> = settlements.iter().collect();
    let mut applied = 0u32;
    // 내역 저장이 하나라도 실패하면 묶음을 지우지 않는다 — 지우면 그 「signed」 는 영영
    // 정산 대기로 남는다. 남긴 묶음은 다음 폴링에 다시 반영된다(저장된 건은 이미 signed 가 아니라 다시 안 맞는다).
    let mut write_failed = false;
    for index in indices {
        if pending.is_empty() {
            break;
        }
        let Ok(hp) = history_path_for(index) else {
            continue;
        };
        let mut list = read_history_at(&hp);
        if list.is_empty() {
            // 파일이 있는데 못 읽은 것이면 묶음을 남긴다 — 빈 목록으로 보고 넘어가면 아래에서
            // 묶음을 지워, 그 계정의 「signed」 가 파일이 되살아나도 영영 정산 대기로 남는다.
            if history_unreadable(&hp) {
                write_failed = true;
            }
            continue;
        }
        let before = pending.len();
        pending.retain(|s| !apply_settlement(&mut list, s));
        let hit = (before - pending.len()) as u32;
        if hit > 0 {
            applied += hit;
            if write_json(hp, &list).is_err() {
                write_failed = true;
            }
        }
    }
    // 본 파일에서 못 찾은 정산은 **보관 파일**에서 찾는다 — 정산이 늦게 오는 사이 시도가 200건을
    // 넘게 쌓이면 그 「signed」 는 보관 파일로 밀려나 있다. 드문 경로라 보관 파일을 통째로 다시 써도 된다.
    if !pending.is_empty() {
        for index in all_indices {
            if pending.is_empty() {
                break;
            }
            let Ok(hp) = history_path_for(index) else {
                continue;
            };
            match settle_in_archive(&crate::policy::history_archive_path(&hp), &mut pending) {
                Ok(hit) => applied += hit,
                Err(_) => write_failed = true,
            }
        }
    }
    if write_failed {
        return applied;
    }
    // 읽은 묶음은 지운다 — 매칭 실패분은 버린다(예전과 같다).
    for p in consumed {
        let _ = fs::remove_file(p);
    }
    applied
}

// ── 체인 확인 (confirm.rs) ───────────────────────────────────────────────────────────────

/// 체인에서 결말을 찾아야 하는 서명 기록인가 — 정산 대기·정산(판매자 말뿐)·정산 실패 가운데 아직 확인 안 했고,
/// 우리 서명과 마지막 유효 블록 높이가 있는 것. 옛 기록(높이 0)은 판정할 수 없어 건드리지 않는다.
pub(crate) fn needs_chain_check(e: &HistoryEntry) -> bool {
    matches!(e.status.as_str(), "signed" | "settled" | "settle_failed")
        && !e.checked
        && e.valid_until > 0
        && crate::solana::is_signature(&e.detail)
}

/// 확인할 기록들 — (계정 번호, 기록). 활성 체인의 모든 계정 본 파일에서(보관 파일로 밀려난 건 드물어 안 본다).
pub(crate) fn chain_candidates() -> Vec<(u32, HistoryEntry)> {
    let mut indices = vec![crate::wallet::active_account_index()];
    if let Ok(w) = crate::wallet::read_encrypted() {
        for a in w.accounts() {
            if !indices.contains(&a.index) {
                indices.push(a.index);
            }
        }
    }
    let mut out = Vec::new();
    for i in indices {
        let Ok(p) = history_path_for(i) else { continue };
        out.extend(read_history_at(&p).into_iter().filter(needs_chain_check).map(|e| (i, e)));
    }
    out
}

/// 체인이 정한 결말 하나를 목록에 적는다 (순수 — 테스트용). 바뀐 기록(바뀐 뒤 모양)을 돌려준다.
/// 그 사이 다른 길(정산 반영 등)이 이미 확인했으면 손대지 않는다.
fn apply_verdict(list: &mut [HistoryEntry], sig: &str, v: &crate::confirm::Verdict) -> Option<HistoryEntry> {
    use crate::confirm::Verdict;
    let e = list.iter_mut().find(|e| e.detail == sig && needs_chain_check(e))?;
    match v {
        Verdict::Settled { tx } => {
            e.status = "settled".into();
            e.settle_tx = tx.clone();
        }
        Verdict::Reverted { tx } => {
            e.status = "reverted".into();
            e.settle_tx = tx.clone();
        }
        Verdict::Expired => e.status = "expired".into(),
    }
    e.checked = true;
    Some(e.clone())
}

/// 계정 `index` 의 본 파일에 결말들을 적는다. 바뀐 기록을 돌려준다(호출자가 「안 나감」 이면 한도를 돌려준다).
/// 못 썼으면 빈 목록 — 다음 확인 때 다시 판정한다(환불은 적은 뒤에만 한다).
pub(crate) fn apply_chain_verdicts(
    index: u32,
    verdicts: &[(String, crate::confirm::Verdict)],
) -> Vec<HistoryEntry> {
    let _g = history_guard();
    let Ok(p) = history_path_for(index) else {
        return Vec::new();
    };
    if history_unreadable(&p) {
        return Vec::new();
    }
    let mut list = read_history_at(&p);
    let changed: Vec<HistoryEntry> = verdicts
        .iter()
        .filter_map(|(sig, v)| apply_verdict(&mut list, sig, v))
        .collect();
    if changed.is_empty() || write_json(p, &list).is_err() {
        return Vec::new();
    }
    changed
}

/// 보관 파일에서 정산을 찾아 반영한다 — 맞은 게 있으면 파일을 통째로 원자 교체한다. 반환 = 반영 건수.
/// 없는 파일은 0. 못 읽는 줄은 그대로 둔다(버리면 다시 쓸 때 사라진다).
fn settle_in_archive(path: &PathBuf, pending: &mut Vec<&Settlement>) -> Result<u32, String> {
    // 바이트로 읽는다 — 한글이 잘린 줄 하나가 파일 전체 읽기를 실패시켜 보관된 signed 에 정산이 영영 안 붙었다.
    let mut rows = read_archive_rows(path)?;
    let mut hit = 0u32;
    pending.retain(|s| {
        for (raw, entry) in rows.iter_mut() {
            let Some(e) = entry else { continue };
            if apply_settlement(std::slice::from_mut(e), s) {
                if let Ok(new_line) = serde_json::to_vec(e) {
                    *raw = new_line;
                }
                hit += 1;
                return false;
            }
        }
        true
    });
    if hit > 0 {
        let mut body = Vec::new();
        for (raw, _) in &rows {
            body.extend_from_slice(raw);
            body.push(b'\n');
        }
        crate::store::write_atomic(path, &body)?;
    }
    Ok(hit)
}

#[cfg(test)]
mod tests;
