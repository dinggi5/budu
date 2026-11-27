// 작업 예산 — 자율 승인 세션(`session.rs`)을 **작업 하나**로 좁힌 것.
//
// 사람이 승인 창에서 «이 작업 전체를 N USDC 까지» 를 고르고 비번을 넣으면, 그 키를 GUI 프로세스 **메모리에만** 들고
// 같은 작업 이름표 · 이미 승인한 판매자 · 남은 예산 · 시간 상한 안의 다음 결제를 비번 없이 서명한다.
//
// 불변식:
//   - 🔴 예산은 **사람만** 연다 — 승인 창의 비번 승인에서만 생긴다. AI 가 보낸 `job` 문자열은 이름표일 뿐이다.
//   - 키는 디스크에 안 나간다(zeroize). 앱 종료·긴급 잠금·수동 세션 잠금·자리비움 잠금·시간 상한이면 소멸.
//     「작업 끝내기」 는 «다시 열기» 를 위해 `RESUME_SECS`(또는 시간 상한 중 먼저 오는 것)까지만 들고 있다가 버린다.
//     시간 상한 = 설정의 자동 잠금(마지막 사용부터). 0 이면 시간으로는 안 닫힌다(예산 상한·끝내기·잠금·앱 종료만).
//   - 예산을 돌려주는 건 **그 예산에 실제로 넣은 서명**만이다 — 이름이 같다고 다른 결제의 만료로 예산을 늘리지 않는다.
//   - 작업 예산 결제도 `do_sign_x402` 를 그대로 거친다 — 긴급 잠금 · 건당/일일 한도 · 내역 · 자체 검증이 같다.
//   - 예산 차감은 서명 **전에** 잡고(두 결제가 겹쳐도 예산을 넘지 않게), 서명이 안 됐으면(Err = 확실히 안 나감) 돌려준다.
//   - 한 번에 작업 하나 — 새 작업에 예산을 열면 앞의 것은 닫힌다(데모 범위, 메뉴바 한 줄).

use crate::i18n::ts;
use serde::Serialize;
use std::sync::Mutex;
use zeroize::Zeroizing;

use crate::store::now_secs;

/// 작업 예산의 시간 상한 = 설정의 «자동 잠금» 과 같은 값(개발 14, 시간 손잡이는 하나).
/// 마지막으로 쓴(열거나 늘리거나 예산으로 결제한) 뒤 이만큼 아무 결제도 없으면 닫는다. 0(«잠그지 않음») 이면 시간으로는 안 닫는다.
/// 설정을 바꾸면 바로 적용되게 저장하지 않고 볼 때마다 읽는다(깨진 값은 자율 세션과 같이 30분).
fn idle_secs() -> u64 {
    crate::session::auto_lock_secs(&crate::settings::read_settings())
}

/// USDC 소수 자릿수 — 솔라나 USDC 민트는 데브넷·메인넷 모두 6.
const USDC_DEC: u8 = 6;

struct Grant {
    /// 예산을 열 때마다 새 번호 — 떼어 둔 몫을 돌려줄 때 「그 사이 다른 예산으로 바뀌지 않았나」 를 본다.
    generation: u64,
    job: String,
    /// 예산을 연 결제의 체인·계정 주소 — 다른 체인·계정의 요청에는 안 쓴다.
    chain_id: u64,
    from: String,
    /// base unit(6자리).
    budget: u64,
    spent: u64,
    /// 사람이 이 작업에서 비번으로 승인한 판매자(payTo, base58 정확 일치).
    sellers: Vec<String>,
    /// 마지막으로 쓴 시각(유닉스 초) — `idle_secs` 동안 쓰지 않으면 닫는다.
    last_active: u64,
    mnemonic: Zeroizing<String>,
    /// 이 예산의 사용액에 넣은 서명과 금액 — 체인 확인이 「안 나감」 으로 닫으면 이것만 돌려준다.
    charged: Vec<(String, u64)>,
}

static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

static GRANT: Mutex<Option<Grant>> = Mutex::new(None);

/// «끝내기» 를 실수로 눌렀을 때 «다시 열기» 로 되돌릴 수 있는 시간. 이 동안만 끝낸 예산(과 키)을 메모리에 둔다 —
/// 끝낸 예산은 어떤 결제도 덮지 않는다(`covers`·`claim` 은 GRANT 만 본다). 긴급 잠금·세션 잠금·새 예산이 열리면 바로 버린다.
const RESUME_SECS: u64 = 10 * 60;

struct Ended {
    grant: Grant,
    at: u64,
}

static ENDED: Mutex<Option<Ended>> = Mutex::new(None);

fn ended() -> std::sync::MutexGuard<'static, Option<Ended>> {
    ENDED.lock().unwrap_or_else(|e| e.into_inner())
}

fn guard() -> std::sync::MutexGuard<'static, Option<Grant>> {
    GRANT.lock().unwrap_or_else(|e| e.into_inner())
}

/// 십진 USDC → base unit. 음수·형식 오류·자릿수 초과는 None.
pub(crate) fn usdc_units(s: &str) -> Option<u64> {
    let v = crate::limits::parse_usdc_nonneg(s.trim(), USDC_DEC).ok()?;
    u64::try_from(v).ok()
}

/// base unit → 사람이 읽는 십진(끝의 0 은 뗀다: 0.060000 → 0.06, 1.000000 → 1).
pub(crate) fn fmt_units(v: u64) -> String {
    let s = crate::solana::format_units(v, USDC_DEC);
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// 작업 예산을 닫는다 — 긴급 잠금·세션 잠금·「작업 끝내기」.
pub(crate) fn clear() {
    *guard() = None;
    *ended() = None;
}

/// 승인 창에서 고른 예산이 이 결제에 쓸 수 있는 값인지 — **서명 전에** 본다(틀렸으면 아무것도 서명하지 않는다).
pub(crate) fn check_budget(job: &str, budget_usdc: &str, amount_usdc: &str) -> Result<u64, String> {
    if crate::policy::clip_job(job).is_empty() {
        return Err(ts!(
            "프로젝트 이름이 없는 결제라 프로젝트 예산을 열 수 없어요. 아무것도 결제하지 않았습니다.",
            "This payment has no project name, so a project budget can't be set. Nothing was paid."
        )
        .into());
    }
    let budget = usdc_units(budget_usdc).filter(|b| *b > 0).ok_or_else(|| {
        ts!(
            "프로젝트 예산은 0보다 큰 USDC 금액이어야 해요(소수 6자리까지). 아무것도 결제하지 않았습니다.",
            "The project budget must be a USDC amount above 0 (up to 6 decimals). Nothing was paid."
        )
        .to_string()
    })?;
    let amount = usdc_units(amount_usdc).unwrap_or(u64::MAX);
    if budget < amount {
        return Err(ts!(
            "프로젝트 예산이 이 결제 금액보다 작아요. 아무것도 결제하지 않았습니다.",
            "The project budget is smaller than this payment. Nothing was paid."
        )
        .into());
    }
    Ok(budget)
}

/// 사람이 비번으로 승인한 결제가 서명된 **뒤** — 작업 예산을 열거나(같은 작업이면 늘리고) 이 결제를 예산에 넣는다.
/// `budget` = Some(사람이 고른 예산, base unit) 이면 열기/늘리기, None 이면 «이 결제만» — 같은 작업의 열린 예산이 있으면
/// 그 사용액에만 더한다(판매자는 안 더한다: 사람이 «이번만» 이라고 했다).
#[allow(clippy::too_many_arguments)]
pub(crate) fn after_human_approval(
    job: &str,
    chain_id: u64,
    from: &str,
    to: &str,
    amount_usdc: &str,
    signature: &str,
    budget: Option<u64>,
    phrase: Option<Zeroizing<String>>,
) {
    let job = crate::policy::clip_job(job);
    if job.is_empty() {
        return;
    }
    let amount = usdc_units(amount_usdc).unwrap_or(0);
    let mut g = guard();
    let same = g
        .as_ref()
        .is_some_and(|x| x.job == job && x.chain_id == chain_id && x.from == from.trim());
    match (budget, phrase) {
        (Some(budget), Some(mnemonic)) => {
            let now = now_secs();
            if same {
                let x = g.as_mut().expect("same 이면 있다");
                x.budget = budget;
                x.spent = x.spent.saturating_add(amount);
                if !x.sellers.iter().any(|s| s == to.trim()) {
                    x.sellers.push(to.trim().to_string());
                }
                x.last_active = now;
                x.mnemonic = mnemonic;
                if !signature.is_empty() {
                    x.charged.push((signature.to_string(), amount));
                }
            } else {
                *ended() = None; // 새 예산이 열렸다 — 끝낸 예산은 되돌릴 일이 없다(키를 오래 들고 있지 않게)
                *g = Some(Grant {
                    generation: NEXT_GENERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
                    job,
                    chain_id,
                    from: from.trim().to_string(),
                    budget,
                    spent: amount,
                    sellers: vec![to.trim().to_string()],
                    last_active: now,
                    mnemonic,
                    charged: if signature.is_empty() { Vec::new() } else { vec![(signature.to_string(), amount)] },
                });
            }
        }
        _ => {
            if same {
                let x = g.as_mut().expect("same 이면 있다");
                x.spent = x.spent.saturating_add(amount);
                if !signature.is_empty() {
                    x.charged.push((signature.to_string(), amount));
                }
            }
        }
    }
}

/// 작업 예산이 이 요청을 덮는가 — 차감 없이 보기만(자율 경로가 먼저 고르는 데 쓴다).
pub(crate) fn covers(req: &crate::ipc::PaymentRequest) -> bool {
    let idle = idle_secs();
    let mut g = guard();
    expire(&mut g, idle);
    g.as_ref().is_some_and(|x| matches(x, req).is_ok())
}

fn expire(g: &mut Option<Grant>, idle: u64) {
    if idle > 0 && g.as_ref().is_some_and(|x| now_secs().saturating_sub(x.last_active) >= idle) {
        *g = None;
    }
}

/// 요청이 예산 안인가 — 맞으면 이 결제 금액(base unit).
fn matches(x: &Grant, req: &crate::ipc::PaymentRequest) -> Result<u64, ()> {
    let job = crate::policy::clip_job(&req.job);
    let ok = !job.is_empty()
        && job == x.job
        && req.kind == crate::policy::KIND_X402
        && req.token == "USDC"
        // 체인·계정을 각인하지 않은 옛 요청은 예산으로 처리하지 않는다(사람에게).
        && req.chain_id != 0
        && req.chain_id == x.chain_id
        && !req.from.is_empty()
        && req.from.trim() == x.from
        && x.sellers.iter().any(|s| s == req.to.trim());
    if !ok {
        return Err(());
    }
    let amount = usdc_units(&req.amount).filter(|a| *a > 0).ok_or(())?;
    if x.spent.checked_add(amount).is_none_or(|t| t > x.budget) {
        return Err(());
    }
    Ok(amount)
}

/// 예산에서 떼어 둔 몫 — 서명되면 `attach`, 안 되면 `release_claim`.
pub(crate) struct Claim {
    generation: u64,
    units: u64,
}

/// 예산에서 이 결제 금액을 **먼저 떼고** 키를 내준다. 못 덮으면 None(→ 사람에게).
pub(crate) fn claim(req: &crate::ipc::PaymentRequest) -> Option<(Zeroizing<String>, Claim)> {
    let idle = idle_secs();
    let mut g = guard();
    expire(&mut g, idle);
    let x = g.as_mut()?;
    let units = matches(x, req).ok()?;
    x.spent += units;
    x.last_active = now_secs();
    Some((x.mnemonic.clone(), Claim { generation: x.generation, units }))
}

/// 떼어 둔 몫이 서명됐다 — 그 서명을 이 예산의 몫으로 적는다(체인 확인이 「안 나감」 이면 돌려받는다).
pub(crate) fn attach(claim: Claim, signature: &str) {
    let mut g = guard();
    if let Some(x) = g.as_mut().filter(|x| x.generation == claim.generation && !signature.is_empty()) {
        x.charged.push((signature.to_string(), claim.units));
    }
}

/// 떼어 둔 몫을 돌려준다 — 서명이 안 나갔다(`do_sign_x402` 의 Err = 확실히 안 나감). 그 사이 예산이 바뀌었으면 아무것도 안 한다.
pub(crate) fn release_claim(claim: Claim) {
    let mut g = guard();
    if let Some(x) = g.as_mut().filter(|x| x.generation == claim.generation) {
        x.spent = x.spent.saturating_sub(claim.units);
    }
}

/// 체인 확인이 「만료·안 나감」 으로 닫은 서명 — **이 예산에 넣은 서명일 때만** 그 몫을 돌려준다(한 번).
pub(crate) fn release_signature(signature: &str) {
    if signature.is_empty() {
        return; // 빈 서명은 어떤 몫과도 안 맞춘다
    }
    let mut g = guard();
    if let Some(x) = g.as_mut() {
        if let Some(i) = x.charged.iter().position(|(s, _)| s == signature) {
            let (_, units) = x.charged.remove(i);
            x.spent = x.spent.saturating_sub(units);
        }
    }
}

/// 메뉴바·승인 창에 보이는 작업 예산 상태.
#[derive(Serialize, Debug, PartialEq)]
pub(crate) struct JobView {
    job: String,
    spent: String,
    budget: String,
    left: String,
    secs_left: u64,
    sellers: usize,
    /// 사람이 승인한 판매자(payTo) — 큰 창의 판매자 화면이 «이 작업에선 창 없이» 를 표시한다. 공개 주소라 비밀 아님.
    seller_addrs: Vec<String>,
    chain_id: u64,
    /// 예산을 연 계정 주소 — 승인 시트·판매자 화면이 «같은 체인·같은 계정의 작업» 인지 맞춰 본다.
    from: String,
}

#[tauri::command]
pub(crate) fn job_status() -> Option<JobView> {
    let idle = idle_secs();
    {
        let mut e = ended();
        if e.as_ref().is_some_and(|x| !resumable(x, idle)) {
            *e = None; // 끝낸 예산의 키를 되돌릴 시간 뒤까지 들고 있지 않게(1초 폴링이 치운다)
        }
    }
    let mut g = guard();
    expire(&mut g, idle);
    g.as_ref().map(|x| JobView {
        job: x.job.clone(),
        spent: fmt_units(x.spent),
        budget: fmt_units(x.budget),
        left: fmt_units(x.budget.saturating_sub(x.spent)),
        secs_left: if idle == 0 { 0 } else { (x.last_active + idle).saturating_sub(now_secs()) },
        sellers: x.sellers.len(),
        seller_addrs: x.sellers.clone(),
        chain_id: x.chain_id,
        from: x.from.clone(),
    })
}

/// 「작업 끝내기」 — 예산을 닫는다. 결제는 지금부터 다시 승인 창으로 가고, `RESUME_SECS` 동안만 «다시 열기» 로 되돌릴 수 있다.
#[tauri::command]
pub(crate) fn end_job() {
    let idle = idle_secs();
    // GRANT 를 쥔 채 ENDED 에 쓴다(순서 GRANT→ENDED, clear 와 같음) — 그 사이 긴급 잠금이 끼어 끝낸 키를 되살리지 않게.
    let mut g = guard();
    expire(&mut g, idle); // 이미 시간이 지난 예산은 끝내기로 옮기지 않는다(다시 열기로 되살아나지 않게)
    if let Some(grant) = g.take() {
        *ended() = Some(Ended { grant, at: now_secs() });
    }
}

/// 끝낸 예산이 아직 되돌릴 수 있는가 — 되돌릴 시간(`RESUME_SECS`)과 시간 상한(마지막 사용부터) 둘 다 안 지났어야 한다.
fn resumable(e: &Ended, idle: u64) -> bool {
    let now = now_secs();
    now.saturating_sub(e.at) < RESUME_SECS && (idle == 0 || now.saturating_sub(e.grant.last_active) < idle)
}

/// 시간이 지난 끝낸 예산(과 키)을 버린다. 되돌릴 수 있으면 true. 프론트가 «다시 열기» 를 보이는 동안 1초마다 부른다.
#[tauri::command]
pub(crate) fn can_resume_job() -> bool {
    let idle = idle_secs();
    let mut e = ended();
    if e.as_ref().is_some_and(|x| !resumable(x, idle)) {
        *e = None;
    }
    e.is_some()
}

/// 「다시 열기」 — 방금 끝낸 예산을 그대로 되살린다(사용액·판매자·키·마지막 사용 시각 그대로 — 끝낸 동안도 유휴로 센다).
/// 되돌릴 시간이 지났거나, 시간 상한이 지났거나, 그 사이 잠겼거나, 다른 예산이 열렸으면 false.
#[tauri::command]
pub(crate) fn resume_job() -> bool {
    let idle = idle_secs();
    let taken = ended().take();
    let Some(e) = taken else { return false };
    if !resumable(&e, idle) {
        return false;
    }
    let mut g = guard();
    if g.is_some() {
        return false;
    }
    *g = Some(e.grant);
    true
}

#[cfg(test)]
pub(crate) fn test_lock() -> std::sync::MutexGuard<'static, ()> {
    static L: Mutex<()> = Mutex::new(());
    L.lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SELLER: &str = "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4";
    const OTHER: &str = "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5";
    const ME: &str = "AqynRZwvVqUPRwRJXvm6odUb3t93fDjnWe3p6BeuUFxD";

    fn req(job: &str, to: &str, amount: &str) -> crate::ipc::PaymentRequest {
        crate::ipc::PaymentRequest {
            id: "1".into(),
            token: "USDC".into(),
            to: to.into(),
            amount: amount.into(),
            memo: String::new(),
            created: now_secs(),
            kind: crate::policy::KIND_X402.into(),
            resource: String::new(),
            chain_id: 103,
            account: 0,
            from: ME.into(),
            lease: true,
            svm: None,
            job: job.into(),
        }
    }

    fn open(budget: &str) {
        let b = check_budget("오슬로호", budget, "0.01").unwrap();
        after_human_approval("오슬로호", 103, ME, SELLER, "0.01", "SIG0", Some(b), Some(Zeroizing::new("k".into())));
    }

    /// 🔴 시나리오 3~6: 예산을 연 작업의 같은 판매자·예산 안 결제만 덮고, 다른 판매자·다른 작업·예산 초과·다른 체인은 사람에게.
    #[test]
    fn grant_covers_only_same_job_seller_and_budget() {
        let _l = test_lock();
        clear();
        assert!(!covers(&req("오슬로호", SELLER, "0.01")), "예산을 열기 전엔 아무것도 안 덮는다");
        open("0.20");
        assert!(covers(&req("오슬로호", SELLER, "0.01")));
        assert!(covers(&req(" 오슬로호 ", SELLER, "0.01")), "이름표는 정리한 값으로 비교");
        assert!(!covers(&req("오슬로호", OTHER, "0.01")), "처음 보는 판매자");
        assert!(!covers(&req("다른 작업", SELLER, "0.01")));
        assert!(!covers(&req("", SELLER, "0.01")));
        assert!(!covers(&req("오슬로호", SELLER, "0.20")), "이미 0.01 썼으니 0.20 은 넘는다");
        let mut r = req("오슬로호", SELLER, "0.01");
        r.chain_id = 101;
        assert!(!covers(&r), "다른 체인");
        r.chain_id = 0;
        assert!(!covers(&r), "체인 각인 없는 옛 요청");
        let mut r = req("오슬로호", SELLER, "0.01");
        r.from = OTHER.into();
        assert!(!covers(&r), "다른 계정");
        clear();
    }

    /// 예산은 서명 전에 떼고, 안 나갔으면 돌려준다 — 둘이 겹쳐도 예산을 넘지 않는다.
    #[test]
    fn claim_reserves_and_release_returns() {
        let _l = test_lock();
        clear();
        open("0.07");
        let (_, a) = claim(&req("오슬로호", SELLER, "0.05")).unwrap();
        assert_eq!(a.units, 50_000);
        attach(a, "SIG1");
        assert!(claim(&req("오슬로호", SELLER, "0.02")).is_none(), "0.01+0.05+0.02 = 0.08 은 넘는다");
        let (_, c) = claim(&req("오슬로호", SELLER, "0.01")).unwrap();
        assert!(claim(&req("오슬로호", SELLER, "0.000001")).is_none(), "다 썼다");
        release_claim(c); // 서명이 안 나갔다
        let v = job_status().unwrap();
        assert_eq!((v.spent.as_str(), v.budget.as_str(), v.left.as_str()), ("0.06", "0.07", "0.01"));
        // 만료된 서명은 한 번만, 이 예산에 넣은 것만 돌려준다.
        release_signature("SIG1");
        release_signature("SIG1");
        release_signature("SIG-남의-것");
        assert_eq!(job_status().unwrap().spent, "0.01");
        clear();
        assert!(job_status().is_none());
    }

    /// «이 결제만» 승인은 같은 작업의 사용액에만 더하고 판매자는 안 더한다. 새 작업에 예산을 열면 앞의 것은 닫힌다.
    #[test]
    fn human_only_payment_counts_but_does_not_add_seller() {
        let _l = test_lock();
        clear();
        open("0.20");
        after_human_approval("오슬로호", 103, ME, OTHER, "0.05", "SIGX", None, None);
        assert!(!covers(&req("오슬로호", OTHER, "0.01")));
        assert_eq!(job_status().unwrap().spent, "0.06");
        let b = check_budget("다음 작업", "1", "0.5").unwrap();
        after_human_approval("다음 작업", 103, ME, OTHER, "0.5", "SIGY", Some(b), Some(Zeroizing::new("k".into())));
        // 🔴 1차 리뷰 P2: 앞 작업(같은 이름이 아니어도)의 서명이 만료돼도 새 예산의 사용액은 안 준다.
        release_signature("SIG0");
        assert_eq!(job_status().unwrap().spent, "0.5");
        // 떼어 둔 뒤 예산이 바뀌었으면 그 몫을 돌려줘도 새 예산은 그대로다.
        let (_, old) = claim(&req("다음 작업", OTHER, "0.5")).unwrap();
        after_human_approval("다음 작업", 101, ME, OTHER, "0.5", "SIGZ", Some(b), Some(Zeroizing::new("k".into())));
        release_claim(old);
        assert_eq!(job_status().unwrap().spent, "0.5");
        assert!(!covers(&req("오슬로호", SELLER, "0.01")));
        assert_eq!(job_status().unwrap().chain_id, 101, "다른 체인으로 다시 열면 새 예산");
        // 2차 리뷰: **같은 이름**으로 새로 연 예산도 앞 예산 서명의 만료로 늘지 않는다.
        clear();
        let b = check_budget("다음 작업", "1", "0.5").unwrap();
        after_human_approval("다음 작업", 103, ME, OTHER, "0.5", "SIGN", Some(b), Some(Zeroizing::new("k".into())));
        release_signature("SIGZ");
        release_signature("SIGY");
        release_signature("");
        assert_eq!(job_status().unwrap().spent, "0.5");
        clear();
    }

    #[test]
    fn budget_must_cover_the_payment() {
        assert!(check_budget("j", "0.20", "0.01").is_ok());
        assert!(check_budget("j", "0.01", "0.02").is_err());
        assert!(check_budget("j", "0", "0.01").is_err());
        assert!(check_budget("j", "-1", "0.01").is_err());
        assert!(check_budget("j", "0.0000001", "0.000001").is_err());
        assert!(check_budget("  ", "1", "0.01").is_err());
    }

    /// 자동 잠금 시간 동안 안 쓰면 닫힌다(빈 설정 = 기본 자동 잠금).
    #[test]
    fn grant_expires() {
        let _l = test_lock();
        let _d = crate::store::test_dir::use_temp("job-expire");
        clear();
        open("0.20");
        guard().as_mut().unwrap().last_active = 0; // 아주 오래전 — 자동 잠금이 0 이 아닌 한 닫힌다
        assert!(!covers(&req("오슬로호", SELLER, "0.01")));
        assert!(job_status().is_none());
    }

    /// 자동 잠금 «잠그지 않음»(0) 이면 시간으로는 안 닫힌다. 예산으로 결제하면 마지막 사용 시각이 갱신된다.
    #[test]
    fn grant_follows_auto_lock_setting() {
        let _l = test_lock();
        let _d = crate::store::test_dir::use_temp("job-nolock");
        let mut s = crate::settings::Settings::default().normalize();
        s.auto_lock_mins = "0".into();
        crate::settings::set_settings(s).unwrap();
        clear();
        open("0.20");
        guard().as_mut().unwrap().last_active = 0;
        assert!(covers(&req("오슬로호", SELLER, "0.01")));
        let (_, c) = claim(&req("오슬로호", SELLER, "0.01")).unwrap();
        assert!(guard().as_ref().unwrap().last_active > 0);
        release_claim(c);
        clear();
    }

    /// 끝내면 결제를 덮지 않고, 다시 열면 그대로 돌아온다. 잠금(clear)·시간 초과 뒤엔 못 연다.
    #[test]
    fn end_then_resume() {
        let _l = test_lock();
        let _d = crate::store::test_dir::use_temp("job-resume");
        clear();
        open("0.20");
        end_job();
        assert!(!covers(&req("오슬로호", SELLER, "0.01")));
        assert!(job_status().is_none());
        let before = ended().as_ref().map(|x| x.grant.last_active);
        assert!(resume_job());
        assert_eq!(guard().as_ref().map(|x| x.last_active), before); // 마지막 사용 시각은 그대로(끝낸 동안도 유휴)
        assert!(covers(&req("오슬로호", SELLER, "0.01")));
        assert!(!resume_job()); // 한 번만

        end_job();
        clear(); // 긴급 잠금 등
        assert!(!resume_job());

        open("0.20");
        end_job();
        assert!(can_resume_job());
        ended().as_mut().unwrap().at = 0;
        assert!(!can_resume_job());
        assert!(ended().is_none()); // 시간이 지나면 키째 버린다
        open("0.20");
        end_job();
        ended().as_mut().unwrap().at = 0;
        let _ = job_status();
        assert!(ended().is_none()); // 1초 폴링(job_status)도 치운다
        assert!(!resume_job());

        // 끝낸 동안도 유휴로 센다 — 시간 상한이 지난 예산은 다시 열 수 없다.
        open("0.20");
        end_job();
        ended().as_mut().unwrap().grant.last_active = 0;
        assert!(!resume_job());

        // 이미 시간이 지난 예산은 끝내기로 옮겨지지도 않는다.
        open("0.20");
        guard().as_mut().unwrap().last_active = 0;
        end_job();
        assert!(ended().is_none());
        clear();
    }
}
