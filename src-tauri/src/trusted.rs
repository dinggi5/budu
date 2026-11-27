// 신뢰 주소(화이트리스트) — 자율 결제 가드.
//
// 자율 한도(금액)만으로는 "처음 보는 주소"로도 비번 없이 나간다(악의적 엔드포인트가
// 소액으로 일일 한도까지 야금야금). 자율 승인은 이 목록의 주소에만 허용한다(설정 auto_trusted_only, 기본 켜짐).
// 목록은 사용자가 설정에서 직접 넣는다(add_trusted_addr). 비번 승인 때 저절로 넣는 학습은 옵션이다
// (설정 auto_learn_trusted, 기본 꺼짐 — 개발 14 전까지는 늘 학습했고, 그때 들어간 주소는 그대로 남는다).
// 신뢰 목록 = 받는 주소(공개 정보)일 뿐 비밀 없음.

use std::fs;
use std::path::PathBuf;

use crate::chain::chain_file;
use crate::i18n::tf;
use crate::store::{budu_dir, write_json};

// 화이트리스트도 체인별로 분리(chain_file) — 테스트넷에 넣은 주소가 메인넷 자율 결제를 비번 없이 통과시키면
// 안 된다(실돈 가드). 메인넷은 빈 목록에서 시작한다.
fn trusted_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(chain_file("trusted")))
}

/// 보기·판정용 — 못 읽으면 빈 목록(= 신뢰하는 곳 없음, 자율 결제가 닫히는 쪽이라 안전하다).
fn read_trusted() -> Vec<String> {
    read_trusted_checked().unwrap_or_default()
}

/// 고치기용 — 없으면 빈 목록, **있는데 못 읽으면 Err**. 빈 목록으로 접으면 철회는 「지울 게 없다」며
/// 아무것도 안 쓰고 성공을 돌려줬고(다시 읽히면 그 주소가 자율 결제를 계속 통과), 학습은 깨진 파일을 한 줄로 덮었다.
fn read_trusted_checked() -> Result<Vec<String>, String> {
    let path = trusted_path()?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// 주소 정규화 = 앞뒤 공백만 걷는다. 🔴 대소문자는 **그대로**.
/// 솔라나 주소(base58)는 대소문자가 다르면 **다른 주소**다 — EVM 시절처럼 소문자로 접으면, 신뢰 주소의 글자 하나만
/// 대소문자를 바꾼 다른 주소(아무도 키를 모르는 32바이트 — 보내면 돈이 사라진다)가 「신뢰 주소」로 자동 결제를 통과한다.
fn normalize_addr(addr: &str) -> String {
    addr.trim().to_string()
}

/// 목록에 추가 (빈 값·중복 무시). 바뀌었으면 true. 순수 함수 — 테스트용 분리.
fn add_trusted(list: &mut Vec<String>, addr: &str) -> bool {
    let a = normalize_addr(addr);
    if a.is_empty() || list.contains(&a) {
        return false;
    }
    list.push(a);
    true
}

/// 🔴 신뢰 목록의 읽기-수정-쓰기를 한 줄로 세운다 — 승인 완료의 학습과 설정 화면의 철회가
/// 겹치면 나중에 쓴 쪽이 다른 쪽을 되돌렸다. 철회가 되돌려지면 그 주소는 계속 자율 결제를 통과한다.
/// 이 파일을 쓰는 건 GUI 뿐이라 프로세스 안 잠금이면 된다.
static TRUSTED_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn trusted_guard() -> std::sync::MutexGuard<'static, ()> {
    TRUSTED_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 사람이 비번으로 승인한 송금/서명의 받는 주소를 학습한다. 기록 실패가 결제를 막진 않는다.
pub(crate) fn record_trusted(addr: &str) {
    let _g = trusted_guard();
    let Ok(mut list) = read_trusted_checked() else {
        return; // 못 읽는 목록을 한 줄로 덮지 않는다 — 학습을 한 번 건너뛸 뿐이다.
    };
    if add_trusted(&mut list, addr) {
        if let Ok(p) = trusted_path() {
            let _ = write_json(p, &list);
        }
    }
}

pub(crate) fn is_trusted(addr: &str) -> bool {
    read_trusted().contains(&normalize_addr(addr))
}

/// 승인 팝업 표시용 — 이 주소가 이전에 비번으로 승인된 적 있는지.
#[tauri::command]
pub(crate) fn is_trusted_addr(to: String) -> bool {
    is_trusted(&to)
}

/// 설정 화면이 보고 있는 체인이 지금 저장된 체인과 같은지 — 다르면 고치지 않는다.
/// 화면이 메인넷 목록을 그린 채 다른 창이 데브넷으로 바꿨으면, «빼기» 가 데브넷 파일에서 못 찾고 성공처럼 보이며
/// 메인넷 주소는 계속 자율 결제를 통과했다. `None` = 옛 호출(체인을 모름) — 지금 체인으로 본다.
fn check_chain(chain_id: Option<u64>) -> Result<(), String> {
    match chain_id {
        Some(c) if c != crate::chain::active_chain().chain_id => Err(crate::i18n::ts!(
            "네트워크가 바뀌었어요. 설정을 다시 열어 주세요.",
            "The network changed. Reopen Settings."
        )
        .to_string()),
        _ => Ok(()),
    }
}

/// 설정 화면 관리용 화이트리스트 목록과 그 목록의 체인. 받는 주소 = 공개 정보라 비번 불필요.
#[tauri::command]
pub(crate) fn get_trusted_addrs() -> Vec<String> {
    read_trusted()
}

/// 사용자가 직접 주소를 넣는다. 보내기와 같은 규칙 — base58 32바이트 공개키여야 한다.
/// 곡선 위 여부는 보지 않는다: x402 판매자의 받는 주소가 프로그램 소유(PDA) 금고일 수도 있다.
#[tauri::command]
pub(crate) fn add_trusted_addr(to: String, chain_id: Option<u64>) -> Result<(), String> {
    check_chain(chain_id)?;
    let a = normalize_addr(&to);
    if crate::solana::parse_pubkey(&a).is_none() {
        return Err(crate::i18n::ts!(
            "솔라나 주소가 아니에요",
            "That isn't a Solana address"
        )
        .to_string());
    }
    let _g = trusted_guard();
    let mut list = read_trusted_checked().map_err(|e| {
        tf!(
            "화이트리스트를 읽지 못해 추가하지 않았어요: {e}",
            "Couldn't read the whitelist, so nothing was added: {e}"
        )
    })?;
    if add_trusted(&mut list, &a) {
        write_json(trusted_path()?, &list)?;
    }
    Ok(())
}

/// 목록에서 제거 (빈 값·없는 주소 무시). 바뀌었으면 true. 순수 함수 — 테스트용 분리.
fn remove_trusted(list: &mut Vec<String>, addr: &str) -> bool {
    let a = normalize_addr(addr);
    let before = list.len();
    list.retain(|x| x != &a);
    list.len() != before
}

/// 신뢰 철회 — 이후 이 주소로의 자율 결제는 다시 비번 승인이 필요하다.
#[tauri::command]
pub(crate) fn remove_trusted_addr(to: String, chain_id: Option<u64>) -> Result<(), String> {
    check_chain(chain_id)?;
    let _g = trusted_guard();
    let mut list = read_trusted_checked().map_err(|e| {
        tf!(
            "화이트리스트를 읽지 못해 지우지 못했어요: {e}",
            "Couldn't read the whitelist, so nothing was removed: {e}"
        )
    })?;
    if remove_trusted(&mut list, &to) {
        write_json(trusted_path()?, &list)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const A: &str = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk";

    // 신뢰 주소 학습: 공백은 걷고 중복/빈 값은 무시. 🔴 대소문자만 다른 문자열은 **다른 주소**(base58).
    #[test]
    fn trusted_list_normalizes_and_dedupes() {
        let mut list = Vec::new();
        assert!(add_trusted(&mut list, A));
        assert!(!add_trusted(&mut list, &format!("  {A}  "))); // 공백 → 중복
        assert!(!add_trusted(&mut list, "")); // 빈 값 무시
        assert_eq!(list, vec![A]);
        // 대소문자만 바꾼 건 다른 주소 — 신뢰 목록에 저절로 들어가 있으면 안 된다.
        // 글자 하나의 대소문자를 바꿔 **여전히 32바이트로 풀리는** 다른 주소를 찾는다(그래야 의미 있는 공격 모양이다).
        let flipped = (0..A.len())
            .filter_map(|i| {
                let c = A.as_bytes()[i] as char;
                let t = if c.is_ascii_lowercase() { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() };
                (t != c).then(|| format!("{}{}{}", &A[..i], t, &A[i + 1..]))
            })
            .find(|f| crate::solana::parse_pubkey(f).is_some())
            .expect("대소문자 하나 바꾼 유효 주소가 있어야 한다");
        assert_ne!(crate::solana::parse_pubkey(&flipped), crate::solana::parse_pubkey(A));
        assert!(!list.contains(&normalize_addr(&flipped)));
    }

    // 신뢰 철회: 공백이 달라도 같은 주소를 제거, 대소문자만 다른 주소·없는 주소는 변경 없음.
    #[test]
    fn trusted_remove_normalizes() {
        let b = "oeYf6KAJkLYhBuR8CiGc6L4D4Xtfepr85fuDgA9kq96";
        let mut list = vec![A.to_string(), b.to_string()];
        assert!(!remove_trusted(&mut list, &A.to_lowercase())); // 대소문자만 다름 → 다른 주소
        assert!(remove_trusted(&mut list, &format!("  {A}  ")));
        assert_eq!(list, vec![b]);
        assert!(!remove_trusted(&mut list, "없는주소"));
        assert!(!remove_trusted(&mut list, ""));
        assert_eq!(list, vec![b]);
    }
}
