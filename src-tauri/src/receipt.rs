// 작업별 영수증 CSV  — 활성 계정의 내역에서 그 작업의 기록만 뽑아
// 다운로드 폴더에 쓴다. 회계·정산 담당이 엑셀로 여는 걸 전제로 UTF-8 BOM 을 붙이고, 수식으로 읽힐 칸(=,+,-,@ 로 시작)은
// 앞에 작은따옴표를 붙인다(CSV 주입 — 결제 대상 URL·작업 이름은 AI·판매자가 정한 문자열이다).

use crate::i18n::ts;
use std::path::PathBuf;

use crate::history::HistoryEntry;

/// 유닉스 초 → `YYYY-MM-DD HH:MM:SS` (UTC). 날짜 크레이트 없이 — 1970 이후만.
fn utc_text(ts: u64) -> String {
    let (days, secs) = (ts / 86_400, ts % 86_400);
    let (y, m, d) = civil_from_days(days as i64);
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}", secs / 3600, secs / 60 % 60, secs % 60)
}

/// 에포크 일 → (연, 월, 일). Howard Hinnant 의 `civil_from_days`.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d)
}

/// CSV 한 칸 — 따옴표로 감싸고, 수식으로 읽힐 시작 문자는 무력화한다.
fn cell(s: &str) -> String {
    let s: String = s.chars().filter(|c| !c.is_control()).collect();
    let s = if s.starts_with(['=', '+', '-', '@']) {
        format!("'{s}")
    } else {
        s
    };
    format!("\"{}\"", s.replace('"', "\"\""))
}

fn status_text(e: &HistoryEntry) -> &'static str {
    match e.status.as_str() {
        "settled" if e.checked => ts!("정산됨(체인 확인)", "Settled (verified on-chain)"),
        "settled" => ts!("정산됨(판매자 보고)", "Settled (per seller)"),
        "signed" => ts!("정산 대기", "Awaiting settlement"),
        "settle_failed" => ts!("정산 실패(판매자 보고)", "Settlement failed (per seller)"),
        "expired" => ts!("만료 · 안 나감", "Expired · not paid"),
        "reverted" => ts!("체인에서 실패 · 안 나감", "Failed on-chain · not paid"),
        "blocked" => ts!("차단됨", "Blocked"),
        _ => ts!("실패", "Failed"),
    }
}

/// 영수증 CSV 본문 (순수 — 테스트용). `entries` 는 최신순, 파일은 **오래된 순**으로 쓴다(장부처럼).
fn receipt_csv(job: &str, entries: &[HistoryEntry], network: &str, explorer_suffix: &str) -> String {
    let mut out = String::from("\u{FEFF}");
    let head = [
        ts!("시각(UTC)", "Time (UTC)"),
        ts!("작업", "Job"),
        ts!("상태", "Status"),
        ts!("금액", "Amount"),
        ts!("토큰", "Token"),
        ts!("판매자", "Seller"),
        ts!("결제 대상", "Resource"),
        ts!("정산 tx", "Settlement tx"),
        ts!("탐색기", "Explorer"),
        ts!("지갑 서명", "Wallet signature"),
        ts!("네트워크", "Network"),
        ts!("비고", "Note"),
    ];
    out.push_str(&head.iter().map(|h| cell(h)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
    let mut settled: u64 = 0;
    for e in entries.iter().rev().filter(|e| e.job == job) {
        let link = if e.settle_tx.is_empty() || !matches!(e.status.as_str(), "settled" | "reverted") {
            String::new()
        } else {
            format!("https://explorer.solana.com/tx/{}{explorer_suffix}", e.settle_tx)
        };
        let sig = if crate::history::is_signature_row(&e.status) { e.detail.as_str() } else { "" };
        let mut note = if crate::history::is_signature_row(&e.status) { String::new() } else { e.detail.clone() };
        if e.undelivered && matches!(e.status.as_str(), "settled" | "signed" | "settle_failed") {
            note = ts!("내용 못 받음 — 판매자에게 정산 tx 로 문의", "Content not received — contact the seller with the settlement tx").into();
        }
        if e.status == "settled" {
            settled = settled.saturating_add(crate::job::usdc_units(&e.amount).unwrap_or(0));
        }
        let row = [
            utc_text(e.ts),
            e.job.clone(),
            status_text(e).into(),
            e.amount.clone(),
            e.token.clone(),
            e.to.clone(),
            e.resource.clone(),
            e.settle_tx.clone(),
            link,
            sig.into(),
            network.into(),
            note,
        ];
        out.push_str(&row.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
        out.push_str("\r\n");
    }
    let total = [
        String::new(),
        job.into(),
        ts!("합계(정산됨)", "Total (settled)").into(),
        crate::job::fmt_units(settled),
        "USDC".into(),
    ];
    out.push_str(&total.iter().map(|c| cell(c)).collect::<Vec<_>>().join(","));
    out.push_str("\r\n");
    out
}

/// 파일 이름에 쓸 작업 이름 — 글자·숫자만 두고 나머지는 `-`, 40자까지.
fn slug(job: &str) -> String {
    let s: String = job
        .chars()
        .map(|c| if c.is_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-");
    let s: String = s.chars().take(40).collect();
    if s.is_empty() {
        "job".into()
    } else {
        s
    }
}

/// 그 작업의 영수증 CSV 를 다운로드 폴더에 쓰고 경로를 돌려준다.
#[tauri::command]
pub(crate) fn export_job_receipt(job: String) -> Result<String, String> {
    let job = crate::policy::clip_job(&job);
    if job.is_empty() {
        return Err(ts!("작업 이름이 비어 있어요.", "The job name is empty.").into());
    }
    let entries = crate::history::get_history(Some(100_000));
    if !entries.iter().any(|e| e.job == job) {
        return Err(ts!("이 작업의 기록이 없어요.", "There are no records for this job.").into());
    }
    let chain = crate::chain::active_chain();
    let (network, suffix) = if chain.chain_id == crate::policy::SOLANA_MAINNET_ID {
        ("solana-mainnet", "")
    } else {
        ("solana-devnet", "?cluster=devnet")
    };
    let body = receipt_csv(&job, &entries, network, suffix);
    let dir: PathBuf = dirs::download_dir()
        .or_else(dirs::home_dir)
        .ok_or_else(|| ts!("저장할 폴더를 찾지 못했어요.", "Couldn't find a folder to save to.").to_string())?;
    let now = crate::store::now_secs();
    let stamp = utc_text(now).replace([' ', ':'], "").replace('-', "");
    let path = dir.join(format!("budu-receipt-{}-{stamp}.csv", slug(&job)));
    std::fs::write(&path, body.as_bytes()).map_err(|e| e.to_string())?;
    Ok(path.to_string_lossy().into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn utc_text_is_right() {
        assert_eq!(utc_text(0), "1970-01-01 00:00:00");
        assert_eq!(utc_text(951_782_400), "2000-02-29 00:00:00");
        assert_eq!(utc_text(1_795_219_199), "2026-11-20 23:59:59");
    }

    #[test]
    fn cells_are_quoted_and_formulas_defused() {
        assert_eq!(cell("a\"b"), "\"a\"\"b\"");
        assert_eq!(cell("=HYPERLINK(\"x\")"), "\"'=HYPERLINK(\"\"x\"\")\"");
        assert_eq!(cell("-1"), "\"'-1\"");
        assert_eq!(cell("줄\n바꿈"), "\"줄바꿈\"");
    }

    fn e(ts: u64, job: &str, status: &str, amount: &str) -> HistoryEntry {
        HistoryEntry {
            ts,
            token: "USDC".into(),
            to: "SELLER".into(),
            amount: amount.into(),
            status: status.into(),
            detail: "SIG".into(),
            job: job.into(),
            settle_tx: if status == "settled" { "TX".into() } else { String::new() },
            ..Default::default()
        }
    }

    /// 그 작업의 기록만, 오래된 순으로, 합계는 정산된 것만. 다른 작업·만료는 합계에 안 든다.
    #[test]
    fn receipt_has_only_that_job_oldest_first_with_settled_total() {
        let list = vec![
            e(300, "오슬로호", "settled", "0.05"),
            e(250, "다른 작업", "settled", "9"),
            e(200, "오슬로호", "expired", "0.01"),
            e(100, "오슬로호", "settled", "0.01"),
        ];
        let csv = receipt_csv("오슬로호", &list, "solana-devnet", "?cluster=devnet");
        assert!(csv.starts_with('\u{FEFF}'));
        let lines: Vec<&str> = csv.trim_end().split("\r\n").collect();
        assert_eq!(lines.len(), 1 + 3 + 1, "{csv}");
        assert!(lines[1].starts_with("\"1970-01-01 00:01:40\""));
        assert!(lines[3].contains("https://explorer.solana.com/tx/TX?cluster=devnet"));
        assert!(!lines[2].contains("explorer"), "만료는 링크 없음");
        assert!(lines[4].contains("\"0.06\""), "{}", lines[4]);
        assert!(!csv.contains("\"9\""));
    }

    #[test]
    fn slug_keeps_letters() {
        assert_eq!(slug("HMM 오슬로호 12/18 부산 입항"), "HMM-오슬로호-12-18-부산-입항");
        assert_eq!(slug("///"), "job");
    }
}
