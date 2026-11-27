// 거래 한도 — 단일/일일 누적 한도 검사 + 오늘 사용액 장부(~/.budu/spend.json).

use crate::i18n::{tf, ts};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::chain::{active_chain, chain_file};
use crate::store::{current_day, budu_dir, write_json};

/// 일일 한도 장부(spend.json) 직렬화 락. read-check-write 를 하나의 임계영역으로 묶어, 동시 결제가
/// 같은 누적액을 보고 둘 다 통과(일일 한도 우회)하는 경합을 막는다. **락은 빠른 파일 I/O 구간만**
/// 잡는다 — 느린/멈춘 RPC 가 모든 결제를 전역 정지시키지 않게, 네트워크 전송은 락 밖에서.
/// std Mutex 는 .await 를 못 넘으므로 tokio 비동기 Mutex 를 쓴다.
static SPEND_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// 오늘 장부에서 누적액(base unit)을 읽는다. 부두가 내보내는 돈은 USDC 하나다 — `token` 은 부르는 쪽의 표시용
///.
fn spent_of(spend: &Spend, _token: &str) -> u128 {
    spend.usdc.parse().unwrap_or(0)
}

/// 누적액을 설정한다.
fn set_spent(spend: &mut Spend, _token: &str, v: u128) {
    spend.usdc = v.to_string();
}

/// 일일 장부에 사용액을 **예약**한다(낙관적 선반영) — 락 안에서 read-check-write 를 빠른 I/O 로만.
/// 한도 통과 + 디스크 기록 성공 시 **예약한 날(UTC 에포크일)** 을 돌려준다(환불이 같은 날에만 적용되게).
/// 한도 초과/기록 실패면 Err — 기록이 안 됐는데 Ok 를 주면 나중에 헛환불로 다른 사용액을 깎는다.
/// 네트워크 전송은 호출자가 락 밖에서 수행하고, 실패하면 refund_spend 로 되돌린다(reserve-then-execute).
pub(crate) async fn reserve_spend(
    token: &str,
    value: u128,
    single: u128,
    daily: u128,
    decimals: u8,
) -> Result<u64, String> {
    // 같은 지갑 폴더의 앱이 하나뿐임을 쥐지 못했으면 돈을 내보내지 않는다 — 이 잠금(SPEND_LOCK 등)은
    // 프로세스 안에서만 통해, 잠금 파일을 못 만든 채(권한 등) 앱이 둘 뜨면 장부를 서로 덮고 같은 요청을 두 번 보낼 수 있다.
    if !crate::store::app_lock_held() {
        return Err(ts!(
            "다른 Budu 가 같은 지갑을 쓰고 있지 않은지 확인하지 못해 결제를 막았어요. ~/.budu/app.lock 을 만들 수 있는지(권한) 확인하고 앱을 다시 시작해 주세요.",
            "Payments are blocked because Budu couldn't confirm it's the only copy using this wallet. Check that ~/.budu/app.lock can be created (permissions), then restart the app."
        )
        .into());
    }
    let _g = SPEND_LOCK.lock().await;
    let mut spend = spend_for_reserve().ok_or_else(|| {
        ts!(
            "오늘 쓴 금액 장부를 읽지 못해 결제를 막았어요. 한도를 지킬 수 없어서예요 — 내일(UTC) 새 장부로 풀리거나, 파일을 고치면 됩니다.",
            "Couldn't read today's spending ledger, so the payment was blocked — the limits can't be enforced without it. It clears with a fresh ledger tomorrow (UTC), or once the file is fixed."
        )
        .to_string()
    })?;
    let spent = spent_of(&spend, token);
    enforce_caps(value, single, spent, daily, token, decimals)?;
    let before = serde_json::to_string_pretty(&spend).map_err(|e| e.to_string())?;
    set_spent(&mut spend, token, spent.saturating_add(value));
    // 기록 실패면 예약도 실패(헛환불 방지). 디스크까지 내린다 — 이 뒤에 돈이 나간다.
    let json = serde_json::to_string_pretty(&spend).map_err(|e| e.to_string())?;
    commit_reservation(&spend_path()?, &before, &json).map(|()| spend.day)
}

/// 예약한 장부를 디스크까지 적는다. 실패면 Err — 호출자는 돈을 내보내지 않는다.
fn commit_reservation(path: &PathBuf, before: &str, after: &str) -> Result<(), String> {
    match crate::store::write_atomic_durable(path, after.as_bytes()) {
        Ok(()) => Ok(()),
        Err(crate::store::DurableError::NotWritten(e)) => Err(e),
        // 새 장부는 제자리인데 디스크까지 못 내렸다 — 예약 전으로 되돌리고 이 결제는 거절한다.
        // 되돌리기마저 실패하면 한도만 깎인 채 남는다(사용자에게 불리할 뿐 돈은 안 나간다).
        Err(crate::store::DurableError::NotDurable(e)) => {
            let _ = crate::store::write_atomic(path, before.as_bytes());
            Err(e)
        }
    }
}

/// 전송/서명 실패 시 reserve_spend 로 선반영했던 사용액을 되돌린다(환불). 락 안에서.
/// **예약한 그 날에만** 차감한다 — 자정을 넘겨 장부가 리셋된 뒤 환불하면 새 날의 정상 사용액을
/// 깎아 일일 한도가 우회될 수 있으므로, 날이 다르면 그 예약은 이미 사라진 것으로 보고 no-op.
pub(crate) async fn refund_spend(token: &str, value: u128, reserved_day: u64) {
    let _g = SPEND_LOCK.lock().await;
    // 못 읽는 장부엔 환불하지 않는다 — 0 에서 빼 봐야 0 이고, 새로 쓰면 깨진 파일(오늘 쓴 기록)을 덮는다.
    let Some(mut spend) = spend_for_reserve() else {
        return;
    };
    if spend.day != reserved_day {
        return; // 다른 날 → 예약분은 이미 리셋됨, 건드리지 않는다
    }
    let spent = spent_of(&spend, token);
    set_spent(&mut spend, token, spent.saturating_sub(value));
    let _ = spend_path().and_then(|p| write_json(p, &spend));
}

/// 체인 확인이 확정한 「돈이 안 나간 결제」의 한도를 **기록 하나당 한 번만** 돌려준다.
/// `key` = 내역 기록의 열쇠(고유 id, 옛 기록은 시각·해시). 오늘 장부가 아니거나(예약한 날이 지났다) 이미 돌려준 열쇠면 아무것도 안 한다.
/// 못 읽는 장부엔 손대지 않는다. 돌려줬으면 true.
/// 부르는 곳 = 솔라나 정산 확정.
pub(crate) async fn refund_once(token: &str, value: u128, day: u64, key: &str) -> bool {
    let _g = SPEND_LOCK.lock().await;
    let Some(mut spend) = spend_for_reserve() else {
        return false;
    };
    if !apply_refund_once(&mut spend, token, value, day, key) {
        return false;
    }
    spend_path().and_then(|p| write_json(p, &spend)).is_ok()
}

/// `refund_once` 의 판단 (순수 — 테스트용).
fn apply_refund_once(spend: &mut Spend, token: &str, value: u128, day: u64, key: &str) -> bool {
    if spend.day != day || key.is_empty() || spend.refunded.iter().any(|k| k == key) {
        return false;
    }
    let spent = spent_of(spend, token);
    set_spent(spend, token, spent.saturating_sub(value));
    spend.refunded.push(key.to_string());
    true
}

/// 설정에서 한도를 확인하지 못했다는 표시 — `settings::salvage_limits` 가 못 살린 칸에 넣는다. 빈 값이라 설정 화면의
/// 칸이 비어 보이고, 저장하려면 숫자를 넣어야 한다(검증이 빈 값을 거부한다).
pub(crate) const LIMIT_UNKNOWN: &str = "";

/// 결제 직전 한도 읽기 — 「모름」이면 막는다. 금액 파서와 따로 두는 이유: 빈 **금액**은 형식 오류지만 빈 **한도**는
/// 「설정을 못 읽었다」라 사람이 할 일이 다르다.
pub(crate) fn parse_limit_usdc(s: &str, dec: u8) -> Result<u128, String> {
    limit_known(s)?;
    parse_usdc_nonneg(s, dec)
}

fn limit_known(s: &str) -> Result<(), String> {
    if s.trim() == LIMIT_UNKNOWN {
        return Err(ts!(
            "설정에서 한도를 읽지 못해 결제를 막았어요. 설정에서 한도를 다시 저장해 주세요.",
            "Payments are blocked because your limits couldn't be read. Save your limits again in Settings."
        )
        .into());
    }
    Ok(())
}

/// 토큰 자릿수보다 긴 소수는 거부한다. alloy `parse_units` 는 넘친 자리를 **조용히 버려서**
/// 한도 「0.0000001 USDC」가 0 = 무제한이 됐고, 금액 「1.1234567」은 1.123456 으로 나갔다. 끝의 0 은 자리로 안 센다.
fn check_decimals(s: &str, dec: u8) -> Result<(), String> {
    let t = s.trim();
    // 십진수 꼴만 받는다 — 부호 하나·숫자·점 하나, 숫자는 적어도 한 자리. alloy 는 "." 를 0 으로 읽어
    // 한도 칸의 「.」 이 0 = 무제한이 됐다. `_` 구분자·지수 표기도 여기서 막는다.
    let body = t.strip_prefix(['+', '-']).unwrap_or(t);
    if !body.bytes().any(|b| b.is_ascii_digit())
        || !body.bytes().all(|b| b.is_ascii_digit() || b == b'.')
        || body.matches('.').count() > 1
    {
        return Err(ts!("숫자로 입력해 주세요", "Enter a number").into());
    }
    let frac = t.split_once('.').map(|(_, f)| f.trim_end_matches('0')).unwrap_or("");
    if frac.len() > dec as usize {
        return Err(tf!(
            "소수점 아래는 {dec}자리까지만 쓸 수 있어요",
            "Use at most {dec} digits after the decimal point"
        ));
    }
    // 정수부가 너무 길면 거부한다. 옛 alloy `parse_units` 는 자릿수를 맞추는 곱셈이 **넘치면 감아서**
    // `2^250+1` USDC 가 1 USDC 가 됐다 — 화면엔 거대한 수, 한도 검사·송금은 1. 개발 6 에 금액이 u128 이 되면서 상한을
    // 77 → 38 자리로: 10^38 < 2^128 이라 넘칠 수 없다(USDC 로 10^32 달러 — 실제 금액은 이 근처에도 안 간다).
    let int = t.split_once('.').map_or(t, |(i, _)| i);
    let int = int.trim_start_matches(['+', '-']).trim_start_matches('0');
    if int.len() + dec as usize > 38 {
        return Err(ts!("금액이 너무 커요", "That amount is too large").into());
    }
    Ok(())
}

/// USDC 금액/한도 문자열을 base unit 으로 파싱한다. **음수는 거부**한다(「-0」 은 0).
/// (예전 alloy `parse_units` 는 "-1" 을 I256 으로 받아 `get_absolute()` 가 2의 보수 거대 U256 을 돌려줬다
///  → 음수 한도가 "무제한"으로, 음수 금액이 거대 송금으로 둔갑하는 함정. 개발 6 에 손으로 짠 `parse_units` 로 바꿨다.)
pub(crate) fn parse_usdc_nonneg(s: &str, dec: u8) -> Result<u128, String> {
    parse_units(s, dec)
}

/// 십진수 문자열 → base unit. `check_decimals` 가 꼴·소수 자리·정수 자리를
/// 먼저 막으므로(정수+소수 ≤ 38자리 → 10^38 < u128::MAX ≈ 3.4×10^38) 곱셈은 넘칠 수 없지만, 그래도 checked 로 닫는다.
fn parse_units(s: &str, dec: u8) -> Result<u128, String> {
    check_decimals(s, dec)?;
    let t = s.trim();
    let (neg, body) = match t.strip_prefix('-') {
        Some(rest) => (true, rest),
        None => (false, t.strip_prefix('+').unwrap_or(t)),
    };
    let (int, frac) = body.split_once('.').unwrap_or((body, ""));
    let frac = frac.trim_end_matches('0');
    let too_large = || ts!("금액이 너무 커요", "That amount is too large").to_string();
    let int_v: u128 = if int.is_empty() {
        0
    } else {
        int.parse().map_err(|_| too_large())?
    };
    let frac_v: u128 = if frac.is_empty() {
        0
    } else {
        format!("{frac:0<width$}", width = dec as usize)
            .parse()
            .map_err(|_| too_large())?
    };
    let v = 10u128
        .checked_pow(dec as u32)
        .and_then(|scale| int_v.checked_mul(scale))
        .and_then(|x| x.checked_add(frac_v))
        .ok_or_else(too_large)?;
    if neg && v != 0 {
        return Err(ts!(
            "음수 금액은 허용되지 않습니다",
            "A negative amount isn't allowed"
        )
        .into());
    }
    Ok(v)
}

/// base unit → 십진수, 소수 자리를 다 채운다("1.000000"). 끝 0 을 지우려면 `trim_units`.
fn format_units(v: u128, dec: u8) -> String {
    if dec == 0 {
        return v.to_string();
    }
    let base = 10u128.pow(dec as u32);
    format!("{}.{:0width$}", v / base, v % base, width = dec as usize)
}

/// 오늘 누적 사용액 장부. 금액은 USDC base unit(6자리) 십진 문자열. 옛 장부의 `eth` 칸은 읽을 때 무시한다.
/// `day`(UTC 일 단위)가 바뀌면 자동 리셋한다.
#[derive(Serialize, Deserialize, Default)]
pub(crate) struct Spend {
    pub(crate) day: u64,
    pub(crate) usdc: String,
    /// 오늘 **체인 확인으로** 환불해 준 내역 기록의 열쇠 — 같은 기록을 두 번 돌려주지 않는 근거.
    /// 환불을 「내역 파일에 결말을 적었는가」에 매달면, 사본 둘(본 파일·보관 파일) 중 하나만 써지는 실패에서 두 번 주거나
    /// 영영 안 줬다. 장부 한 파일 안에서 「빼기」와 「적어 두기」를 같이 쓰므로 둘이 갈리지 않는다. 날이 바뀌면 같이 비워진다.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) refunded: Vec<String>,
}

/// 프론트로 주는 오늘 사용액 (보기 좋은 십진수).
#[derive(Serialize)]
pub(crate) struct SpendView {
    usdc: String,
}

pub(crate) fn spend_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(chain_file("spend")))
}

/// 예약·환불용 오늘 장부. 옛 `read_spend_today` 는 못 읽으면 0 으로 삼켰다 —
/// **돈이 나가기 전의 한도 검사**에선 「오늘 19 USDC 를 썼는데 장부가 상해 0 부터」가 되어 일일 한도를 넘겨 보냈다
/// (내역·입금에서 두 번 본 「못 읽으면 빈 값」 병). 파일이 있는데 못 읽으면 None — 단 마지막 수정이 **어제 이전**이면 그 파일은
/// 오늘 쓴 돈을 담을 수 없으니 새 날 장부로 간다(안 그러면 한 번 상한 장부가 영영 결제를 막는다).
fn spend_for_reserve() -> Option<Spend> {
    let path = spend_path().ok()?;
    let raw = match fs::read_to_string(&path) {
        Ok(t) => Some(Ok(t)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
        Err(_) => Some(Err(())),
    };
    let mtime_day = fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs() / 86_400);
    judge_spend(raw, mtime_day, current_day())
}

/// `spend_for_reserve` 의 판단 (순수 — 테스트용). `raw` = None(파일 없음) | Ok(본문) | Err(있는데 못 읽음).
fn judge_spend(
    raw: Option<Result<String, ()>>,
    mtime_day: Option<u64>,
    today: u64,
) -> Option<Spend> {
    let fresh = Spend {
        day: today,
        usdc: "0".into(),
        ..Default::default()
    };
    // 금액 칸이 숫자가 아닌 장부도 「못 읽음」이다 — JSON 모양만 맞으면 통과시키고
    // `spent_of` 가 그 칸을 0 으로 읽어, 오늘 쓴 돈이 지워진 채 한도를 검사했다(「못 읽으면 빈 값」 다섯 번째).
    // 빈 칸도 못 읽음이다 — 72 에선 「옛 기본값」이라 0 으로 쳤는데, 0.1.0 부터 장부를 쓰는 곳은
    // 전부 숫자("0" 포함)를 쓴다. 오늘 장부의 빈 칸은 손상뿐이고, 0 으로 치면 오늘 쓴 돈이 지워진다.
    // 빈 칸은 따로 거른다(옛 `u128::from_str("")` 은 Ok(0) 이었다 — u128 은 Err 지만 뜻을 글로 남겨 둔다).
    let amount_ok = |v: &str| !v.trim().is_empty() && v.parse::<u128>().is_ok();
    let parsed = match raw {
        None => return Some(fresh),
        Some(Ok(t)) => serde_json::from_str::<Spend>(&t)
            .ok()
            .filter(|s| amount_ok(&s.usdc)),
        Some(Err(())) => None,
    };
    match parsed {
        Some(s) if s.day == today => Some(s),
        Some(_) => Some(fresh),
        None if mtime_day.is_some_and(|d| d < today) => Some(fresh),
        None => None,
    }
}

/// 단일 + 일일 누적 한도를 검사한다 (체인에 보내기 전에). 통과하면 Ok.
/// 한도가 0이면 "무제한"으로 보고 그 검사를 건너뛴다.
pub(crate) fn enforce_caps(
    amount: u128,
    single_cap: u128,
    spent_today: u128,
    daily_cap: u128,
    unit: &str,
    decimals: u8,
) -> Result<(), String> {
    if single_cap != 0 && amount > single_cap {
        let s = trim_units(&format_units(single_cap, decimals));
        return Err(tf!(
            "단일 거래 한도({s} {unit})를 초과했습니다",
            "Over the per-payment limit ({s} {unit})"
        ));
    }
    if daily_cap != 0 && spent_today.saturating_add(amount) > daily_cap {
        let remaining = daily_cap.saturating_sub(spent_today);
        let r = trim_units(&format_units(remaining, decimals));
        return Err(tf!(
            "오늘 한도를 초과했습니다 — 남은 한도 {r} {unit}",
            "Over today's limit — {r} {unit} left"
        ));
    }
    Ok(())
}

/// 오늘 누적 사용액을 보기 좋은 십진수로 돌려준다.
/// 🔴 장부를 못 읽으면 **오류**다 — 예전엔 화면용이라며 0 을 돌려줘, 결제는 막히는데 설정·보내기
/// 화면은 「오늘 남은 한도」를 꽉 찬 값으로 보였다. 판정은 결제 검사와 같은 `spend_for_reserve`.
#[tauri::command]
pub(crate) fn get_today_spend() -> Result<SpendView, String> {
    let s = spend_for_reserve().ok_or_else(|| {
        ts!(
            "오늘 쓴 금액 기록을 읽지 못했어요.",
            "Couldn't read today's spending record."
        )
        .to_string()
    })?;
    let usdc_raw: u128 = s.usdc.parse().unwrap_or(0);
    Ok(SpendView {
        usdc: format_units(usdc_raw, active_chain().usdc_decimals),
    })
}

/// 승인 창에 보여 줄 한도 상황 — 전부 보기 좋은 십진수. 빈 값 = 그 한도 없음(0 = 무제한).
#[derive(Serialize, Debug, PartialEq)]
pub(crate) struct LimitRoom {
    /// 건당 한도.
    single: String,
    /// 일일 한도.
    daily: String,
    /// 오늘 이미 쓴 금액.
    spent: String,
    /// 이 결제를 하고 나면 오늘 남는 한도(일일 한도가 없으면 빈 값).
    left_after: String,
}

/// 한도 상황을 계산한다 (순수 — 테스트용). 한도를 넘으면 `enforce_caps` 와 같은 문구로 Err.
fn room_for(
    amount: u128,
    single: u128,
    daily: u128,
    spent: u128,
    decimals: u8,
) -> Result<LimitRoom, String> {
    enforce_caps(amount, single, spent, daily, "USDC", decimals)?;
    let fmt = |v: u128| trim_units(&format_units(v, decimals));
    let cap = |v: u128| if v == 0 { String::new() } else { fmt(v) };
    Ok(LimitRoom {
        single: cap(single),
        daily: cap(daily),
        spent: fmt(spent),
        left_after: if daily == 0 {
            String::new()
        } else {
            fmt(daily.saturating_sub(spent.saturating_add(amount)))
        },
    })
}

/// `format_units` 의 끝 0 을 지운다("0.010000" → "0.01", "5.000000" → "5").
fn trim_units(s: &str) -> String {
    match s.split_once('.') {
        Some((i, f)) => {
            let f = f.trim_end_matches('0');
            if f.is_empty() {
                i.to_string()
            } else {
                format!("{i}.{f}")
            }
        }
        None => s.to_string(),
    }
}

/// 🔴 **승인 창 전 한도 검사**.
/// 예약하지 않고 **읽기만** 한다 — 실제 방어선은 여전히 서명 직전의 `reserve_spend` 다(그 사이 다른 결제가 끼면 거기서 막힌다).
/// 한도를 넘으면 `Over`, 한도·장부·금액을 못 읽으면 `Unreadable`(둘 다 서명 단계도 어차피 막는다). 둘을 가르는 건 AI·사람에게
/// 할 말이 달라서다 — 장부가 깨진 걸 «한도 초과» 라고 하면 원인을 잘못 안다.
#[derive(Debug, PartialEq)]
pub(crate) enum Precheck {
    Over(String),
    Unreadable(String),
}

pub(crate) fn precheck_usdc(amount_usdc: &str) -> Result<LimitRoom, Precheck> {
    let dec = active_chain().usdc_decimals;
    let amount = parse_usdc_nonneg(amount_usdc.trim(), dec).map_err(Precheck::Unreadable)?;
    let settings = crate::settings::read_settings();
    let single = parse_limit_usdc(&settings.single_usdc, dec).map_err(Precheck::Unreadable)?;
    let daily = parse_limit_usdc(&settings.daily_usdc, dec).map_err(Precheck::Unreadable)?;
    let spend = spend_for_reserve().map(Ok).unwrap_or_else(|| Err(Precheck::Unreadable(

        ts!(
            "오늘 쓴 금액 장부를 읽지 못해 결제를 막았어요. 한도를 지킬 수 없어서예요 — 내일(UTC) 새 장부로 풀리거나, 파일을 고치면 됩니다.",
            "Couldn't read today's spending ledger, so the payment was blocked — the limits can't be enforced without it. It clears with a fresh ledger tomorrow (UTC), or once the file is fixed."
        )
        .to_string(),
    )))?;
    room_for(amount, single, daily, spent_of(&spend, "USDC"), dec).map_err(Precheck::Over)
}

/// 승인 창이 «건당 한도 · 오늘 남은 한도» 를 그리려고 부른다. 넘으면 Err(문구) — 그런 요청은 원래 창이 안 뜬다
/// (`ipc::refuse_over_limit`). 창이 뜬 뒤 다른 결제로 한도가 줄었으면 여기서 그 문구가 보이고, 서명 단계가 막는다.
#[tauri::command]
pub(crate) fn limit_room(amount: String) -> Result<LimitRoom, String> {
    precheck_usdc(&amount).map_err(|e| match e {
        Precheck::Over(m) | Precheck::Unreadable(m) => m,
    })
}

#[cfg(test)]
mod tests;

