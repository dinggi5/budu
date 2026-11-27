// 작업 표시 이름 — 사람이 작업 이름을 바꿀 수 있게.
//
// 작업 이름표(`job`)는 AI 가 결제마다 붙이는 **열쇠**다 — 예산 판정(job.rs)이 그 글자 그대로를 맞춰 본다. 그래서 이름을 바꾼다는 건
// 열쇠를 고치는 게 아니라 **사람이 읽는 별명**을 따로 다는 것이다. 열쇠를 고치면 AI 가 다음 결제에 옛 이름표를 붙였을 때
// 예산 밖으로 떨어져 사람에게 간다(안전 쪽이지만 «이름만 바꿨는데 창이 뜬다» 는 거짓말이 된다).
//
// 파일: `~/.budu/job_names.<체인>.json` = { "<이름표>": "<별명>" }. 내역·신뢰 주소처럼 체인별(데브넷 연습 작업의 별명이 메인넷에 섞이지 않게).
// 비밀이 아니라 비번이 필요 없다(계정 이름 바꾸기와 같은 결). 영수증 CSV·MCP 응답은 이름표 그대로 — 별명은 화면에만.

use std::collections::BTreeMap;
use std::fs;
use std::path::PathBuf;

use crate::chain::chain_file;
use crate::i18n::ts;
use crate::store::{budu_dir, write_json};

/// 별명 최대 글자 수 — 계정 이름과 같은 감각(목록 한 줄에 들어가게).
const NAME_MAX_CHARS: usize = 40;

fn names_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join(chain_file("job_names")))
}

/// 없으면 빈 표, **있는데 못 읽으면 Err** — 고치기 쪽이 깨진 파일을 한 줄짜리 표로 덮어 다른 별명을 지우지 않게(trusted.rs 와 같은 이유).
fn read_checked() -> Result<BTreeMap<String, String>, String> {
    let path = names_path()?;
    let text = match fs::read_to_string(&path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(e) => return Err(e.to_string()),
    };
    serde_json::from_str(&text).map_err(|e| e.to_string())
}

/// 별명 다듬기 — 앞뒤 공백·제어 문자를 걷고 길이를 자른다.
fn clean(name: &str) -> String {
    name.trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(NAME_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// 표에 별명 하나를 반영한다(순수 함수 — 테스트용). 빈 별명이나 이름표와 같은 별명 = 별명 지우기.
fn apply(map: &mut BTreeMap<String, String>, job: &str, name: &str) -> Result<(), &'static str> {
    let key = job.trim();
    if key.is_empty() {
        return Err("empty job");
    }
    let name = clean(name);
    if name.is_empty() || name == key {
        map.remove(key);
    } else {
        map.insert(key.to_string(), name);
    }
    Ok(())
}

static NAMES_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// 별명 표 — 화면 표시용이라 못 읽으면 빈 표(이름표가 그대로 보일 뿐이다).
#[tauri::command]
pub(crate) fn get_job_names() -> BTreeMap<String, String> {
    read_checked().unwrap_or_default()
}

/// 작업 별명을 바꾼다. 빈 값 = 별명을 지우고 AI 가 붙인 이름표로 돌아간다.
#[tauri::command]
pub(crate) fn rename_job(job: String, name: String) -> Result<BTreeMap<String, String>, String> {
    let _g = NAMES_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut map = read_checked()?;
    apply(&mut map, &job, &name).map_err(|_| String::from(ts!("프로젝트 이름표가 비어 있어요", "The project label is empty")))?;
    write_json(names_path()?, &map)?;
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_sets_trims_and_clears() {
        let mut m = BTreeMap::new();
        apply(&mut m, " 입항-0412 ", "  부산 신항 입항 \n").unwrap();
        assert_eq!(m.get("입항-0412").map(String::as_str), Some("부산 신항 입항"));
        // 이름표와 같은 별명 = 지우기(표가 쓸데없이 커지지 않게).
        apply(&mut m, "입항-0412", "입항-0412").unwrap();
        assert!(m.is_empty());
        apply(&mut m, "a", "b").unwrap();
        apply(&mut m, "a", "   ").unwrap();
        assert!(m.is_empty());
        assert!(apply(&mut m, "  ", "x").is_err());
        let long = "가".repeat(80);
        apply(&mut m, "a", &long).unwrap();
        assert_eq!(m["a"].chars().count(), NAME_MAX_CHARS);
    }

    #[test]
    fn rename_round_trips_and_refuses_broken_file() {
        let _d = crate::store::test_dir::use_temp("job-names");
        assert!(get_job_names().is_empty());
        let m = rename_job("w1".into(), "파고 점검".into()).unwrap();
        assert_eq!(m["w1"], "파고 점검");
        assert_eq!(get_job_names()["w1"], "파고 점검");
        // 깨진 파일은 덮지 않는다 — 다른 별명이 사라지면 안 된다.
        fs::write(names_path().unwrap(), "{not json").unwrap();
        assert!(rename_job("w2".into(), "x".into()).is_err());
        assert_eq!(fs::read_to_string(names_path().unwrap()).unwrap(), "{not json");
        assert!(get_job_names().is_empty());
    }
}
