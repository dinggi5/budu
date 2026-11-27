// ~/.budu 파일을 읽는 **규칙의 정본** — GUI(src-tauri)와 MCP(budu-mcp)가 같은 소스 파일을
// 컴파일한다.
//
// 크레이트가 아니다. 두 크레이트의 lib.rs 가 `#[path = "../../shared/policy.rs"] mod policy;` 로
// 이 파일을 제 모듈로 끌어들인다 → Cargo 의존성·워크스페이스 변화 0(「공유 크레이트를 만들지
// 않는다 — Tauri 빌드 위험 0」 정책은 그대로), 그러나 정본은 하나다. 개발 52·54 에 생긴 「같은 파일을
// 두 곳(세 곳)에서 다르게 읽는」 뿌리를 여기서 뽑는다:
//   - settings.json → 어느 체인인가 (chain.rs 의 단독 읽기 vs settings.rs 의 Settings 파싱, + MCP 사본)
//   - settings.json → 사용자 지정 RPC (app 은 Settings 전체 파싱, MCP 는 rpc_url 단독 읽기)
//   - history 파일의 항목 형식 HistoryEntry (app 이 쓰고 MCP 가 읽는다 — 어긋나면 AI 가 내역을 못 본다)
//   - AI·화면으로 나가는 문자열의 URL 가리기 redact_urls (키 유출 방지 — 두 벌이면 한쪽만 구멍 난다)
//   - ~/.budu 디렉터리 이름
//   - wallet.enc   → 계정 목록·활성 계정 정규화 (app EncryptedWallet vs MCP EncMeta)
//   - 체인별·계정별 데이터 파일 이름 (chain_file · account_file_name, 양쪽 사본)
//   - 「지갑이 이미 있는가」 (설정·체인·언어 기본값이 전부 이 질문으로 신규/기존을 가른다)
//
// 여기 두는 것의 조건: **IO 판정과 순수 규칙만.** i18n 매크로(`ts!`/`tf!`)·체인 상수 묶음(ChainConfig —
// 공식 RPC 주소 포함)·에러 문구는 각 크레이트가 계속 따로 가진다 — 이 파일은 두 크레이트 어느 쪽의 모듈도 참조하지
// 않아야 양쪽에서 그대로 컴파일된다(의존은 std + serde + serde_json 만).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// ── 체인 ID 정본 ────────────────────────────────────────────────────────────────────────────
// 두 크레이트의 ChainConfig 상수가 이 값을 쓴다. 폴백 판정(아래 chain_id_for)이 데브넷/메인넷을 가리킨다.

/// 솔라나 데브넷 — 기본 클러스터. 데이터 파일이 접미사 없이 저장되는 「원본」 클러스터.
///
/// 솔라나엔 EVM 같은 체인 ID 가 없다. settings.json·데이터 파일 이름·IPC 의 `chain_id` 칸은 원본 구조를 그대로 쓰고,
/// 값은 솔라나 토큰 목록(solana-labs/token-list)이 쓰던 관례 번호를 빌린다: 101 = mainnet-beta, 103 = devnet.
/// 진짜 클러스터 판정은 이 번호가 아니라 RPC 의 `getGenesisHash` 다(shared/solana.rs).
pub const SOLANA_DEVNET_ID: u64 = 103;
/// 솔라나 메인넷(mainnet-beta, 실제 자금).
pub const SOLANA_MAINNET_ID: u64 = 101;
/// 두 크레이트가 아는 클러스터 전부 — settings.json 의 chain_id 가 이 밖이면 크레이트가 데브넷으로 접는다.
/// 추가하면 여기와 양쪽 ChainConfig 탐색에 같이 넣는다(각 크레이트 테스트가 짝을 검사한다).
pub const SUPPORTED_CHAIN_IDS: [u64; 2] = [SOLANA_DEVNET_ID, SOLANA_MAINNET_ID];
/// 못 읽은·깨진·모르는 설정이 접히는 곳. 실돈 클러스터로 접지 않는다.
pub const FALLBACK_CHAIN_ID: u64 = SOLANA_DEVNET_ID;
/// 진짜 신규 설치(지갑도 설정도 없음)의 기본. 대회 데모 단계라 데브넷 — 메인넷은 사람이 설정에서 고를 때만.
/// (원본은 신규 기본이 메인넷이라 아래 `chain_id_for` 의 갈래가 넷이었다. 갈래는 그대로 두고 값만 같다.)
pub const NEW_INSTALL_CHAIN_ID: u64 = SOLANA_DEVNET_ID;

// ── ~/.budu ────────────────────────────────────────────────────────────────────────────────

/// 홈 아래 데이터 디렉터리 이름. 두 크레이트의 `budu_dir()` 이 홈을 각자 구해(dirs 크레이트 + i18n 에러
/// 문구) 여기에 붙인다 — 이름이 갈리면 GUI 가 만든 지갑을 MCP 가 못 찾는다.
pub const BUDU_DIR_NAME: &str = ".budu";

/// 🔴 시작 가드 — 데이터 폴더가 `.jigap` 이면 **컴파일 자체가 안 된다**.
/// `~/.jigap` 은 이 코드의 원본 지갑 앱이 쓰는 실제 메인넷 지갑 폴더다. 같은 폴더·잠금 파일·승인 요청 폴더를
/// 쓰면 남의 지갑을 건드린다. 사람이 규칙을 기억하는 데 기대지 않고 빌드가 막는다.
const FORBIDDEN_DIR_NAME: &str = ".jigap";
const fn same_bytes(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    let mut i = 0;
    while i < a.len() {
        if a[i] != b[i] {
            return false;
        }
        i += 1;
    }
    true
}
const _: () = assert!(
    !same_bytes(BUDU_DIR_NAME, FORBIDDEN_DIR_NAME),
    "데이터 폴더가 .jigap 이다 — 실제 지갑 폴더를 건드리게 된다"
);

/// 홈 → 데이터 디렉터리 경로.
pub fn budu_dir_in(home: &Path) -> PathBuf {
    home.join(BUDU_DIR_NAME)
}

// ── 지갑 유무 ───────────────────────────────────────────────────────────────────────────────

/// `~/.budu` 에 지갑 파일이 이미 있는가 — 암호화본(wallet.enc)과 옛 평문(wallet.json) 둘 다 본다.
/// settings.json 이 없거나 필드가 비었을 때 「진짜 신규 설치」와 「기존 사용자」를 가르는 질문이고,
/// 체인 기본값(아래)·언어 기본값(i18n::init)·설정 기본값(Settings) 이 전부 같은 답을 봐야 한다.
pub fn wallet_exists_in(dir: &Path) -> bool {
    dir.join("wallet.enc").exists() || dir.join("wallet.json").exists()
}

// ── settings.json → 체인 ────────────────────────────────────────────────────────────────────

/// settings.json 을 읽은 결과의 세 갈래. 「없음」과 「있는데 못 읽음」이 다른 답을 내므로(개발 39 —
/// 신규 기본이 메인넷이 된 순간부터) 읽기 실패를 한 덩이로 뭉치면 안 된다.
#[derive(Debug, PartialEq)]
pub enum SettingsFile {
    /// 파일이 없다.
    Missing,
    /// 파일이 있는데 못 읽었다(권한 등) — 깨진 파일과 같이 보수적으로 다룬다.
    Unreadable,
    /// 본문. 해석은 호출자 몫(깨졌을 수도 있다).
    Text(String),
}

impl SettingsFile {
    /// 경로에서 읽어 세 갈래로 나눈다. `NotFound` 만 Missing, 그 외 IO 실패는 전부 Unreadable.
    pub fn read(path: &Path) -> SettingsFile {
        match std::fs::read_to_string(path) {
            Ok(text) => SettingsFile::Text(text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SettingsFile::Missing,
            Err(_) => SettingsFile::Unreadable,
        }
    }
}

/// settings.json 에서 선택된 체인 ID 만 읽는 가벼운 뷰 — **다른 필드는 무시**한다. 한도 필드 하나가
/// 깨진 파일도 chain_id 는 그대로 읽힌다. 돈이 나가는 쪽(서명·송금)이 이 판정을 쓰므로, 화면·설정
/// (Settings 파싱)도 같은 판정을 따라야 한다(「연습용」 라벨에 메인넷 송금).
#[derive(Deserialize)]
struct ChainSel {
    chain_id: u64,
}

/// settings.json 본문 → chain_id. 깨졌거나 필드가 없으면 `FALLBACK_CHAIN_ID`(데브넷).
/// 값은 **정규화하지 않는다**(미지원 id 도 그대로) — 지원 여부는 각 크레이트의 ChainConfig 탐색이
/// 판단한다(미지원 → 데브넷 폴백). 여기서 접으면 화면이 「모르는 체인」을 알릴 수 없다.
pub fn chain_id_in(text: &str) -> u64 {
    serde_json::from_str::<ChainSel>(text)
        .map(|c| c.chain_id)
        .unwrap_or(FALLBACK_CHAIN_ID)
}

/// **사용자가 선택한 체인 ID — 유일한 판정.** 폴백 갈래:
/// - 파일 없음 + **지갑도 없음** = 진짜 신규 → `NEW_INSTALL_CHAIN_ID`
/// - 파일 없음 + 지갑 있음 → `FALLBACK_CHAIN_ID` (신규 기본을 메인넷으로 바꾸는 날에도 기존 지갑을 조용히 실돈 쪽으로 옮기지 않게)
/// - 있는데 못 읽음(권한·홈 못 정함) → `FALLBACK_CHAIN_ID`
/// - 본문 → `chain_id_in` (정상이면 그 값, 깨졌거나 필드가 없으면 `FALLBACK_CHAIN_ID`)
///
/// `wallet_exists` 는 클로저다 — 파일이 없을 때만 묻는다(이 함수는 체인을 쓰는 모든 곳에서 자주 불린다).
pub fn chain_id_for(file: &SettingsFile, wallet_exists: impl FnOnce() -> bool) -> u64 {
    match file {
        SettingsFile::Missing if wallet_exists() => FALLBACK_CHAIN_ID,
        SettingsFile::Missing => NEW_INSTALL_CHAIN_ID,
        SettingsFile::Unreadable => FALLBACK_CHAIN_ID,
        SettingsFile::Text(text) => chain_id_in(text),
    }
}

// ── settings.json → RPC ────────────────────────────────────────────────────────────────────

/// settings.json 본문 → 사용자 지정 RPC(앞뒤 공백 제거). 없거나 비었거나 깨졌으면 빈 값 =
/// 「활성 체인의 공식 RPC 를 따라간다」. ChainSel 과 같은 이유로 **다른 필드는 무시**한다 — 한도
/// 필드 하나가 깨진 파일에서 GUI 는 Settings 파싱에 실패해 공식 RPC 로 접고 MCP 는 이 필드만 읽어
/// 커스텀 RPC 를 쓰던 것이 두 판정의 뿌리였다.
///
/// 🔴 **단, 파일의 `chain_id` 를 못 알아보면 지정 RPC 도 버린다**. 지정 RPC 는 파일이
/// 말하는 체인의 엔드포인트다. chain_id 가 깨졌거나(타입 틀림·null·키 중복) 미지원 값이면 크레이트는
/// 체인을 데브넷으로 접는데, 그 RPC 가 메인넷 것이면 화면·한도는 「연습용」인 채 블록해시·잔액은 메인넷에서 온다.
/// `pick_rpc` 의 `forced_other_chain` 과 같은 이유(딴 체인의 RPC 는 쓰지 않는다). 필드가 **없는** 파일은
/// 기본 체인(데브넷)의 RPC 로 보고 유지 — `chain_id_in` 도 같은 답.
///
/// 🔴 chain_id 는 **`chain_id_in` 과 같은 방식(serde 파생)** 으로 읽는다. `serde_json::Value`
/// 로 읽으면 같은 키가 두 번 있을 때 마지막 값을 조용히 쓰는데 파생은 중복을 거부한다 —
/// `{"chain_id":103,"chain_id":101,"rpc_url":메인넷}` 에서 체인은 데브넷으로 접히고 RPC 만 메인넷이
/// 살아남는 갈림이 생긴다. 파생끼리면 「체인을 못 읽는 파일 = RPC 도 못 읽는 파일」이 구조로 보장된다
/// (`rpc_kept_implies_chain_recognized` 테스트).
///
/// 값 자체는 검사하지 않는다 — http(s) 여부는 저장 경로(set_settings)의 몫이고, 이미 파일에 있는 값은
/// 어느 프로세스든 **같은 값**을 써야 한다.
pub fn rpc_url_in(text: &str) -> String {
    let Ok(sel) = serde_json::from_str::<RpcSel>(text) else {
        return String::new();
    };
    let chain_known = match sel.chain_id {
        None => true, // 필드 없음 — 기본 체인(데브넷)
        Some(id) => SUPPORTED_CHAIN_IDS.contains(&id),
    };
    if chain_known {
        sel.rpc_url.trim().to_string()
    } else {
        String::new()
    }
}

/// `rpc_url_in` 의 뷰 — 다른 필드는 무시하되 **이 두 필드의 형식은 ChainSel 만큼 엄격**하다(중복 키·
/// 타입 틀림·null 이면 파일 전체를 못 읽은 것으로). `chain_id` 가 없는 건 허용(옛 파일)하지만 null 은
/// 아니다 — `Option` 기본 역직렬화는 null 을 None 으로 받으므로 `present_u64` 로 막는다.
#[derive(Deserialize)]
struct RpcSel {
    #[serde(default, deserialize_with = "present_u64")]
    chain_id: Option<u64>,
    #[serde(default)]
    rpc_url: String,
}

/// 필드가 **있으면** u64 여야 한다(null·문자열·실수는 오류). 없을 때만 serde default(None).
fn present_u64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<u64>, D::Error> {
    u64::deserialize(d).map(Some)
}

/// **사용자가 지정한 RPC — 유일한 판정.** 파일이 없거나 못 읽으면 빈 값(공식 RPC). 체인 판정과
/// 달리 지갑 유무는 묻지 않는다 — 어느 갈래든 「지정한 게 없다」 는 같은 답이다.
pub fn rpc_url_for(file: &SettingsFile) -> String {
    match file {
        SettingsFile::Missing | SettingsFile::Unreadable => String::new(),
        SettingsFile::Text(text) => rpc_url_in(text),
    }
}

/// 지정 RPC 와 활성 체인의 공식 RPC 중 무엇을 쓸지.
///
/// `forced_other_chain` = 환경변수(`BUDU_CHAIN_ID`, MCP 만)가 settings 와 **다른** 체인을
/// 강제한 상태. 그때 settings 의 rpc_url 은 **딴 체인의 엔드포인트**다 — 그대로 쓰면 이 체인의
/// 컨트랙트를 저쪽 체인에 물어 잔액이 `returned no data ("0x")` 로 죽는다.
/// 커스텀 RPC 를 조용히 버리는 셈이지만, 대안이 「조용히 안 되는 것」이라 이쪽이 낫다. GUI 에는
/// 환경변수로 체인을 갈아타는 경로가 없으므로 항상 false 를 준다.
pub fn pick_rpc(custom: &str, forced_other_chain: bool, default_rpc: &str) -> String {
    if custom.is_empty() || forced_other_chain {
        default_rpc.to_string()
    } else {
        custom.to_string()
    }
}

// ── 숨은 문자 ──────────────────────────────────────────────────────────────────────────

/// 결제 memo 검사 — MCP(402 고르기)와 GUI(서명 직전 `x402::resolve_terms`)가 같은 함수를 쓴다.
/// 사람 눈에 안 보이거나 글 방향을 바꾸는 문자 — 제어·폭 없는 문자·방향 서식·소프트 하이픈·태그 문자(화면의 `\p{C}` 와 맞춘다).
pub fn is_hidden_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{00AD}'
                | '\u{061C}'
                | '\u{180E}'
                | '\u{200B}'..='\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2060}'..='\u{206F}'
                | '\u{FEFF}'
                | '\u{FFF9}'..='\u{FFFB}'
                | '\u{E0000}'..='\u{E007F}'
                | '\u{E000}'..='\u{F8FF}'
        )
}

// ── 체인별·계정별 데이터 파일 이름 ─────────────────────────────────────────────────────────────

/// 체인별로 분리하는 데이터 파일 이름(spend/history/x402_settlements/trusted).
/// 기본 클러스터(데브넷)는 접미 없이("history.json"), 그 외는 "-{chain_id}" 접미("history-101.json") —
/// 데브넷/메인넷의 사용액·내역·신뢰 목록이 절대 섞이지 않게.
/// `chain_id` 는 **정규화된(지원되는) 값**을 넘긴다 — 호출자는 `active_chain().chain_id` 를 준다.
/// 미지원 id 가 여기까지 오면 데브넷 파일과 어긋난 새 파일이 생겨 한도·신뢰 목록이 조용히 리셋된다.
pub fn chain_file_name(chain_id: u64, stem: &str) -> String {
    if chain_id == SOLANA_DEVNET_ID {
        format!("{stem}.json")
    } else {
        format!("{stem}-{chain_id}.json")
    }
}

/// **계정별로도** 분리하는 데이터 파일 이름 — 내역(history)과 SOL 보내기 기록(sol_sends). 체인 접미 뒤에 계정
/// 접미를 덧붙인다: 계정 0 은 **기존 이름 그대로**(무손실), 그 외는 `-a{n}`("history-a2.json",
/// "history-101-a2.json"). 어긋나면 GUI 가 적은 내역을 AI 가 못 본다.
pub fn account_file_name(chain_base: &str, index: u32) -> String {
    match index {
        0 => chain_base.to_string(),
        n => format!("{}-a{n}.json", chain_base.trim_end_matches(".json")),
    }
}

// ── wallet.enc → 계정 ───────────────────────────────────────────────────────────────────────

/// 계정 하나 — 같은 시드의 HD 파생 인덱스(솔라나 m/44'/501'/n'/0') + 그 주소(공개정보, 평문) +
/// 사람이 붙인 라벨(빈 값 = 화면이 「계정 N」으로). wallet.enc 의 `accounts` 항목이자 MCP 상태의 항목.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Account {
    /// 파생 인덱스 n. 0 = 지갑을 만들 때부터 있던 원래 계정.
    pub index: u32,
    pub address: String,
    #[serde(default)]
    pub label: String,
}

/// wallet.enc 의 계정 목록 정규화 — 인덱스 순, **계정 0 은 항상 있고 그 주소는 파일의 `address`
/// 필드가 정본**이다(목록에 0 이 따로 적혀 있어도 주소는 `address` 가 이기고 라벨만 목록 것을 쓴다 —
/// 옛 빌드도 그 필드를 읽으니 둘이 어긋나면 안 된다). 옛 파일(목록 없음)은 계정 0 하나가 된다.
pub fn normalize_accounts(address: &str, listed: &[Account]) -> Vec<Account> {
    let mut list: Vec<Account> = listed.iter().filter(|a| a.index != 0).cloned().collect();
    let zero_label = listed
        .iter()
        .find(|a| a.index == 0)
        .map(|a| a.label.clone())
        .unwrap_or_default();
    list.push(Account {
        index: 0,
        address: address.to_string(),
        label: zero_label,
    });
    list.sort_by_key(|a| a.index);
    list.dedup_by_key(|a| a.index);
    list
}

/// 활성 계정 — `active` 가 목록에 없으면(손상·옛 빌드가 쓴 파일) 계정 0. 돈이 나가는 계정이
/// 「없는 계정」이 되면 안 된다. `list` 는 `normalize_accounts` 의 결과(계정 0 이 반드시 있다).
pub fn pick_active(list: &[Account], active: u32) -> Account {
    list.iter()
        .find(|a| a.index == active)
        .or_else(|| list.iter().find(|a| a.index == 0))
        .or_else(|| list.first())
        .cloned()
        .expect("normalize_accounts 는 계정 0 을 항상 넣는다")
}

// ── history 파일 항목 ───────────────────────────────────────────────────────────────────────────

/// 송금 시도 1건의 기록(감사 로그) — GUI 가 history 파일에 쓰고 MCP 가 그대로 읽는다.
/// 성공/차단/실패를 모두 남긴다. 필드를 더할 땐 `#[serde(default)]` 를 붙여 옛 기록이 계속 읽히게 한다.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct HistoryEntry {
    /// 유닉스 초.
    pub ts: u64,
    /// "ETH" | "USDC".
    pub token: String,
    /// 받는 주소 (시도 당시 입력값).
    pub to: String,
    /// 금액 (십진수 문자열).
    pub amount: String,
    /// "sent" | "blocked" | "failed" | "signed"(x402 서명·정산 대기) | "settled"(x402 정산됨) | "settle_failed"
    /// | "unknown"
    /// | "received"
    /// | "expired"
    /// | "reverted".
    pub status: String,
    /// sent·unknown=tx 해시, blocked/failed=사유, signed=인가 nonce(정산 매칭용).
    pub detail: String,
    /// x402 정산 tx 해시(페이실리테이터가 온체인 제출). 정산 전엔 빈 문자열.
    #[serde(default)]
    pub settle_tx: String,
    /// 기록마다 고유한 번호. 보관 파일의 겹침 검사가 「모든 필드가 같은 두 기록」(같은 초·금액·사유)을
    /// 하나로 보고 진짜 기록을 건너뛰던 틈을 닫는다. 옛 기록은 빈 값.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub id: String,
    /// 보낸 계정의 주소 = 이 기록의 주인. 파일 이름은 체인·계정 **번호**로만
    /// 갈려서, 지갑 파일을 지우고 다른 시드를 가져오면 새 계정 0 이 옛 지갑의 기록을 제 것처럼 읽었다.
    /// 파일 형식(배열)은 그대로 두고 **기록마다** 싣는다 — 옛 MCP 는 모르는 필드를 무시하고 계속 읽는다.
    /// 읽는 쪽은 주인이 다른 기록을 뺀다(`read_sent_history`). 옛 기록은 빈 값 = 주인을 모름 → 보여 준다.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub from: String,
    /// 체인에서 확정했는가 — GUI 가 영수증(sent·unknown)이나 `authorizationState`(signed)로 확인했다.
    /// 확인한 기록은 다시 묻지 않는다. 옛 기록·아직 못 본 기록은 false.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub checked: bool,
    /// 이 결제가 한도를 예약한 날(UTC 일 번호). 체인 확인이 「안 나갔다」로 확정해 환불할 때
    /// **이 날에만** 돌려준다. 예전엔 기록 시각으로 추정했다 — 자정 전에 예약한 송금이 제출 줄·절전으로 늦게 기록되면
    /// 다음 날 사용액을 깎아 한도를 더 쓰게 했다. 0 = 모름(옛 기록·예약 없는 기록) → 기록 시각 추정으로 물러선다.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub reserved_day: u64,
    /// x402 결제 대상 URL — 내역에서 «무엇을 샀나» 를 보이려고. AI 가 정한 값이라 **표시용일 뿐** 판정에 안 쓴다.
    /// 길이는 `clip_resource` 로 자른다. 옛 기록·송금 기록은 빈 값.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub resource: String,
    /// 작업 이름표 — AI 가 `x402_fetch(job=…)` 로 붙인 값(`clip_job` 으로 정리). 같은 값끼리 내역에서 묶이고
    /// 영수증 CSV 가 작업 단위로 나간다. **이름표일 뿐 권한이 아니다** — 예산은 사람이 승인 창에서만 연다(`job.rs`).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub job: String,
    /// 서명한 트랜잭션의 마지막 유효 블록 높이. 이 높이가 확정(finalized)된 뒤에도 체인에
    /// 우리 서명이 없으면 그 서명은 **영원히 못 들어간다** → 「expired」 로 닫고 한도를 한 번 돌려준다. 0 = 모름(옛 기록) → 확인 안 함.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub valid_until: u64,
    /// 블록해시를 받은 slot — 정산 확정이 서명 목록을 어디까지 볼지 정하는 기준(이 컴퓨터 시계는 틀릴 수 있다).
    /// 0 = 모름 → 목록을 끝까지 본 경우에만 만료를 말한다.
    #[serde(default, skip_serializing_if = "is_zero_u64")]
    pub sign_slot: u64,
    /// 돈은 나갔거나 나갔을 수 있는데 **응답 내용을 끝까지 못 받았다** — MCP 가 정산 기록에 실어 보낸다.
    /// 화면은 「내용 못 받음 · 판매자에게 tx 로 문의」 를 붙인다(같은 결제를 다시 내면 또 나간다).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub undelivered: bool,
    /// MCP 가 정산된 응답의 **내용을 끝까지 받았다**고 알렸다. 작업 예산의 «같은 결제 10분» 예외는
    /// 이 긍정 신호가 있을 때만 연다 — «내용 못 받음 표시가 없다» 로 판정하면, 체인 확인이 먼저 정산을 적은 뒤 늦게 온 표시가
    /// 버려지거나 MCP 가 본문을 읽다 죽어 아무것도 못 적은 결제도 «받음» 으로 읽혔다. 옛 기록·모름은 false(= 사람에게).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub delivered: bool,
}

/// 작업 이름표 최대 길이(글자) — 승인 창·메뉴바 한 줄에 들어갈 만큼.
pub const MAX_JOB_CHARS: usize = 80;

/// 작업 이름표 정리. 제어·방향·서식·사설 영역 문자를 버리고(사람이 본 이름 ≠ 비교하는 바이트가 되지 않게)
/// 공백을 하나로 접고 `MAX_JOB_CHARS` 에서 자른다. 빈 값 = 작업 없음.
pub fn clip_job(s: &str) -> String {
    let clean: String = s
        .chars()
        .filter(|c| {
            !(c.is_control()
                || matches!(
                    c,
                    '\u{00AD}'
                        | '\u{061C}'
                        | '\u{180E}'
                        | '\u{200B}'..='\u{200F}'
                        | '\u{202A}'..='\u{202E}'
                        | '\u{2060}'..='\u{206F}'
                        | '\u{FEFF}'
                        | '\u{FFF9}'..='\u{FFFB}'
                        | '\u{E0000}'..='\u{E007F}'
                        | '\u{E000}'..='\u{F8FF}'
                ))
        })
        .collect();
    let folded = clean.split_whitespace().collect::<Vec<_>>().join(" ");
    folded.chars().take(MAX_JOB_CHARS).collect()
}

/// 내역에 담는 결제 대상 URL 의 최대 길이(바이트) — 넘치면 문자 경계에서 자르고 `…` 를 붙인다.
pub const MAX_HISTORY_RESOURCE: usize = 300;

/// 내역용 URL 자르기. 제어 문자는 버린다(화면·CSV 를 깨지 않게).
pub fn clip_resource(s: &str) -> String {
    let clean: String = s.trim().chars().filter(|c| !c.is_control()).collect();
    if clean.len() <= MAX_HISTORY_RESOURCE {
        return clean;
    }
    let mut end = MAX_HISTORY_RESOURCE;
    while !clean.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &clean[..end])
}

fn is_zero_u64(v: &u64) -> bool {
    *v == 0
}

/// 이 기록이 `owner` 의 것으로 보이는가 — 주인이 적혀 있으면 주소 대조, 없으면(옛 기록) 참.
/// 🔴 대조는 **정확히**(공백만 무시) — 솔라나 주소(base58)는 대소문자가 다르면 다른 주소다.
pub fn history_owned_by(e: &HistoryEntry, owner: &str) -> bool {
    e.from.is_empty() || owner.is_empty() || e.from.trim() == owner.trim()
}

/// history 파일이 품는 최신 기록 수 — 넘치면 오래된 것부터 **보관 파일로 옮긴다**.
/// 예전엔 그냥 잘라 버렸다: 201번째 송금부터 가장 오래된 기록이 조용히 사라졌다.
/// 본 파일을 이 크기로 묶어 두는 이유 = 송금마다 통째로 읽고 쓰는 파일이라(정산 반영도 1초마다 읽는다).
pub const HISTORY_HOT_CAP: usize = 200;

/// history 파일 옆의 보관 파일 — `history-101-a1.json` → `history-101-a1.archive.jsonl`.
/// 한 줄에 한 건, **오래된 것부터** 덧붙인다(덧붙이기라 몇만 건이 쌓여도 송금 한 번에 드는 쓰기는 한 줄).
/// 옛 빌드(0.4.2 이하의 MCP·CLI)는 이 파일을 모른다 — 본 파일 형식이 그대로라 최신 200건은 계속 읽는다.
pub fn history_archive_path(hot: &Path) -> PathBuf {
    hot.with_extension("archive.jsonl")
}

/// 보관 파일의 기록(최신순). 없으면 빈 목록. 못 읽는 줄(덧붙이다 죽은 마지막 줄)은 건너뛴다.
///
/// 🔴 **바이트로 읽는다** — 덧붙이다 끊겨 한글이 잘린 줄이 하나라도 있으면 `read_to_string` 이 통째로 실패해 보관된
/// 기록 **전부**가 GUI·MCP·CLI 에서 사라져 보였다. 잘린 줄만 걸러진다.
pub fn read_history_archive(path: &Path) -> Vec<HistoryEntry> {
    let Ok(bytes) = std::fs::read(path) else {
        return Vec::new();
    };
    let raw = String::from_utf8_lossy(&bytes);
    let mut list: Vec<HistoryEntry> = raw
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    list.reverse();
    list
}

/// 보낸 기록 중 최신 `want` 건(최신순) — 본 파일 다음에 보관 파일. 보관 파일은 본 파일이 모자랄 때만 연다.
/// 본 파일을 못 읽으면 빈 목록(예전과 같다 — 보여 줄 게 없을 뿐 지우지 않는다).
/// `owner` = 지금 계정의 주소 — 주인이 다른 기록(다른 시드의 옛 기록)은 뺀다. 빈 값이면 거르지 않는다.
pub fn read_sent_history(hot: &Path, want: usize, owner: &str) -> Vec<HistoryEntry> {
    let mut list: Vec<HistoryEntry> = std::fs::read_to_string(hot)
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<HistoryEntry>>(&s).ok())
        .unwrap_or_default();
    list.retain(|e| history_owned_by(e, owner));
    if list.len() < want {
        list.extend(
            read_history_archive(&history_archive_path(hot))
                .into_iter()
                .filter(|e| history_owned_by(e, owner)),
        );
    }
    list.truncate(want);
    list
}

// ── URL 가리기 ──────────────────────────────────────────────────────────────────────────────

/// 사용자/AI 에 노출되는 에러·로그 문자열에서 URL 을 통째로 `[RPC]` 로 가린다.
/// 커스텀 RPC 경로·쿼리엔 API 키가 들어가곤 한다(예: alchemy `…/v2/KEY`). alloy/reqwest 에러는
/// URL 을 그대로 실어 나르므로 — 특히 MCP/CLI 결과·거래내역은 AI 채팅으로 나가 키가 LLM 에 샐 수 있다.
/// **설정을 다시 읽지 않고 문자열에 보이는 URL 자체를 가린다** → 설정 변경·host 대소문자 정규화·
/// ws/wss 등에 흔들리지 않는다. URL 외 문자는 그대로 둔다.
/// 개발 57 까지 두 크레이트에 같은 코드가 두 벌 있었다 — 한쪽만 고치면 다른 쪽으로 키가 샌다.
pub fn redact_urls(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut cursor = 0;
    while let Some(rel) = input[cursor..].find("://") {
        let sep = cursor + rel;
        if let Some(start) = scheme_start(input, cursor, sep) {
            out.push_str(&input[cursor..start]);
            // 토큰 끝: 공백·제어문자 또는 URL 에 못 들어가는 문자("· <· >· `)에서 멈춘다.
            // `)`·`,`·`'` 는 URL sub-delim 이라 종료자로 안 씀 — 그 뒤 키가 새지 않게 보수적으로.
            let token = &input[start..];
            let end = token
                .find(|c: char| {
                    c.is_whitespace() || c.is_control() || matches!(c, '"' | '<' | '>' | '`')
                })
                .unwrap_or(token.len());
            out.push_str("[RPC]");
            cursor = start + end;
        } else {
            // "://" 앞에 유효 scheme 이 없다 → 그대로 두고 그 뒤부터 계속 스캔.
            out.push_str(&input[cursor..sep + 3]);
            cursor = sep + 3;
        }
    }
    out.push_str(&input[cursor..]);
    out
}

/// `sep`(="://"의 시작 인덱스) 앞에서 scheme 시작 인덱스를 찾는다. scheme = `[A-Za-z][A-Za-z0-9+.-]*`.
/// `min` 미만으로는 내려가지 않는다(이미 처리한 영역 침범 방지). 유효 scheme 없으면 None.
fn scheme_start(s: &str, min: usize, sep: usize) -> Option<usize> {
    let bytes = s.as_bytes();
    let mut start = sep;
    while start > min {
        let c = bytes[start - 1];
        if c.is_ascii_alphanumeric() || matches!(c, b'+' | b'-' | b'.') {
            start -= 1;
        } else {
            break;
        }
    }
    // 최소 1글자 + 맨 앞은 영문자(RFC 3986 scheme).
    if start < sep && bytes[start].is_ascii_alphabetic() {
        Some(start)
    } else {
        None
    }
}

// ── 결제 방식(kind) ─────────────────────────────────────────────────────────────────────────────
// 요청 파일의 `kind` 값. 두 크레이트가 같은 상수를 쓴다.

/// x402 솔라나 `exact` — 지갑이 트랜잭션을 조립해 부분 서명하고(정산은 페이실리테이터), 지금은 이것 하나다.
/// 🔴 값이 개발 3 에서 `"x402"`(EVM EIP-3009 서명) → `"x402-svm"` 으로 바뀌었다. 요청·결과 모양(`SvmTerms`·`SvmSigned`)이
/// 통째로 달라져, 옛 빌드끼리 섞이면 하트비트의 `kinds` 대조에서 막히게(닫힌 쪽) 이름째 바꿨다.
pub const KIND_X402: &str = "x402-svm";

/// x402 솔라나 결제 조건 — MCP 가 402 에서 **고른** 요구를 GUI 에 넘기는 칸(요청 파일 `svm`). 비밀 없음.
/// GUI 는 이 값으로 트랜잭션을 조립하기 전에 활성 클러스터·요청의 `to`/`amount` 와 다시 대조한다(MCP 를 믿지 않는다).
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SvmTerms {
    /// CAIP-2 (`solana:` + genesis 앞 32자).
    pub network: String,
    /// 토큰 민트(USDC) base58.
    pub asset: String,
    /// 받는 지갑 주소(ATA 아님) base58.
    pub pay_to: String,
    /// base unit 정수 문자열.
    pub amount: String,
    /// 수수료 지불자(페이실리테이터) base58 — 서명자 0번 자리.
    pub fee_payer: String,
    /// 판매자가 정한 Memo(`extra.memo`). 없으면 지갑이 16바이트 난수 hex 를 넣는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memo: Option<String>,
}

/// GUI 가 승인 뒤 돌려주는 부분 서명 트랜잭션(비밀 없음). MCP 는 `transaction` 만 결제 헤더에 싣는다.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct SvmSigned {
    /// 부분 서명된 v0 트랜잭션 전체(base64) — `PAYMENT-SIGNATURE` 의 `payload.transaction`.
    pub transaction: String,
    /// **우리 서명**(`signatures[1]`, base58). 결정적이라 정산 확정·중복 판정의 열쇠.
    pub signature: String,
    /// 트랜잭션의 블록해시(base58)와 그 마지막 유효 블록 높이 — 「영원히 안 들어감」 판정용.
    pub blockhash: String,
    pub last_valid_block_height: u64,
}

/// 하트비트에 `kinds` 가 없는 앱이 할 수 있다고 볼 것 — 없음. 부두는 처음부터 `kinds` 를 싣는다.
/// (못 읽으면 아무 요청도 보내지 않는 쪽으로 닫힌다.)
pub fn kinds_legacy() -> Vec<String> {
    Vec::new()
}

/// 지금 빌드의 앱이 처리하는 방식 전부 — GUI 가 하트비트에 싣는다. 승인 경로(`approve_kind`·`auto_work`)의
/// match 팔과 같이 움직여야 한다.
pub fn kinds_supported() -> Vec<String> {
    vec![KIND_X402.into()]
}

// ── 요청 파일 거두기 ──────────────────────────────────────────────────────────────────

/// 요청 파일이 `id` 의 것일 때만 지운다 — 돌려주는 값 = 지웠는가.
///
/// 🔴 예전엔 「읽어서 id 확인 → 지우기」 두 걸음이었다(MCP `cancel_request_at`, GUI `resolve_request`).
/// 그 사이에 GUI 가 내 요청을 끝내 지우고 **다른 AI 의 새 요청 B** 가 같은 이름으로 생기면, 두 번째 걸음이 B 를
/// 지웠다 — B 의 승인 창이 사라지고 그 AI 는 5분을 기다린다(돈은 안 나가지만 결제가 조용히 막힌다).
/// 이제 **먼저 떼어 온다**(rename — 원자적). 떼어 온 사본은 누구도 못 바꾸니 거기서 id 를 본다. 남의 것이면
/// 제자리로 돌려놓는다 — `hard_link` 는 그 자리에 새 파일이 이미 생겼으면 실패하므로 남의 새 요청을 덮지 않는다.
/// (그렇게 돌려놓지 못한 요청은 기다리던 쪽이 시간 초과로 「아무것도 안 나감」을 받는다 — 세 요청이 겹쳐야 생긴다.)
pub fn remove_request_if_mine(path: &Path, id: &str) -> bool {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let name = path
        .file_name()
        .map(|f| f.to_string_lossy().into_owned())
        .unwrap_or_default();
    let taken = path.with_file_name(format!(".{name}.taking.{}.{n}", std::process::id()));
    if std::fs::rename(path, &taken).is_err() {
        return false; // 없다(이미 누가 치웠다) — 할 일 없음.
    }
    let mine = std::fs::read_to_string(&taken)
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("id").and_then(|i| i.as_str()).map(|i| i == id))
        .unwrap_or(false);
    if !mine {
        let _ = std::fs::hard_link(&taken, path);
    }
    let _ = std::fs::remove_file(&taken);
    mine
}

// ── 결제 시도 기록 `~/.budu/approvals/` ─────────────────────────────────────────────
//
// 🔴 **「결과 불명」을 프로세스 밖에 남기는 자리.** 개발 65 까지 GUI 의 「승인 진행 중」은 메모리
// (`APPROVALS_IN_FLIGHT`)에만 있었다. 그래서 MCP 가 승인 대기 5분을 채우면 — 사람이 막 비번을 넣어
// GUI 가 **전송 중**이어도 — 요청을 거두고 「사용자가 응답하지 않았어요」라고 답했다. 돈은 나가고,
// GUI 의 결과는 요청이 사라졌다고 버려지고, AI 는 재시도한다 = 이중 결제.
//
// 요청 id 하나에 파일 둘. **파일마다 쓰는 쪽은 하나뿐이다**(두 프로세스가 같은 파일을 고치지 않는다):
//   - `<id>.json`       — GUI 가 쓴다. 승인 처리의 상태(sending → done | failed | unknown)와 결과.
//   - `<id>.proof.json` — 원본 지갑의 직접 제출 증거 자리. 부두 MCP 는 쓰지 않는다(GUI 는 디렉터리째 청소만).
//
// 순서 규약(경합을 닫는 것은 이 순서다 — 두 프로세스 사이엔 잠금이 없다):
//   GUI  : 기록을 `sending` 으로 **먼저 쓰고** → 그다음 요청 파일이 아직 있는지 본다 → 있으면 진행.
//   MCP  : 시간 초과면 요청 파일을 **먼저 지우고** → 그다음 기록을 읽는다.
// 그러면 GUI 가 요청을 보고 진행했다면 그 기록은 MCP 가 지우기 전에 쓰였으므로 MCP 가 반드시 본다.
// GUI 가 요청을 못 봤다면 진행하지 않고 기록을 되돌린다(MCP 가 그 사이 `sending` 을 봤다면 곧 사라진다).

/// 결제 시도 기록 디렉터리 이름.
pub const APPROVALS_DIR: &str = "approvals";

/// 기록을 남겨 두는 기간(초). 뒤늦은 증거 제출 창(상대 서버 600초)과 사람이 내역을 보고 따져 볼 시간을
/// 넉넉히 덮는다. 파일은 수백 바이트라 7일치여도 작다.
pub const APPROVAL_KEEP_SECS: u64 = 7 * 24 * 3600;

/// 승인 처리를 시작했다(GUI). 아직 체인으로 나갔는지 모른다 — **이 상태로 멈춰 있으면 「불명」이다**
/// (앱이 전송 중에 죽은 경우).
pub const ATTEMPT_SENDING: &str = "sending";
/// 끝났다 — 서명은 만들어졌다(제출은 MCP·판매자 몫). 결과(`status == "approved"`)가 함께 있다.
pub const ATTEMPT_DONE: &str = "done";
/// 확실히 아무것도 안 나갔다(비번 오류·한도·잠금·체인이 거절). 요청은 살아 있어 다시 승인할 수 있다.
pub const ATTEMPT_FAILED: &str = "failed";
/// 서명한 트랜잭션을 냈는데 **받혔는지 모른다**. GUI 가 직접 제출하던 원본 지갑의 갈래라 부두의 x402-svm 경로는
/// 이 값을 만들지 않는다 — 읽는 쪽은 옛 기록을 위해 «나갔을 수 있음» 으로 닫힌 채 남긴다.
pub const ATTEMPT_UNKNOWN: &str = "unknown";
/// 자율 승인이 조건(세션 잠김·한도·신뢰 주소 등)에 걸려 **사람에게 넘겼다** — 아무것도 시작하지 않았다.
/// 예전엔 `failed`(detail `NEEDS_PASSWORD`)로 적혀 「실패」라는 거짓 이름을 달았다. 뜻은 같다:
/// 다시 승인해도 되고, 시간 초과면 「아무것도 안 나감」. 이 값을 모르는 옛 MCP 도 `after_timeout` 의 `_` 갈래로 같은 답을 낸다.
pub const ATTEMPT_HANDED: &str = "handed";

/// 요청 id 가 파일 이름으로 써도 되는 모양인가. id 는 MCP 가 만든 나노초 숫자지만 요청 파일은 로컬의
/// 다른 프로세스가 쓴 입력이다 — `../` 같은 값으로 디렉터리 밖을 쓰게 두지 않는다.
pub fn attempt_id_ok(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric())
}

/// GUI 가 쓰는 기록 경로. id 가 이상하면 None(기록을 안 남긴다 — 그 요청은 GUI 도 처리하지 않는다).
pub fn attempt_path(dir: &Path, id: &str) -> Option<PathBuf> {
    attempt_id_ok(id).then(|| dir.join(APPROVALS_DIR).join(format!("{id}.json")))
}

/// MCP 가 쓰는 증거 재료 경로. (GUI 는 안 쓴다 — 디렉터리째 청소만 한다.)
#[allow(dead_code)]
pub fn proof_path(dir: &Path, id: &str) -> Option<PathBuf> {
    attempt_id_ok(id).then(|| dir.join(APPROVALS_DIR).join(format!("{id}.proof.json")))
}

/// GUI 의 승인 처리 기록 1건 — `PaymentResult`(결과 파일)와 같은 필드를 싣는다. 결과 파일은 **한 칸뿐**이라
/// 요청이 사라지면 GUI 가 결과를 안 쓴다(남의 결과를 덮지 않으려고). 이 기록은 id 마다 따로라
/// 그 제약이 없다 — MCP 는 시간 초과 뒤에도 여기서 결말을 읽는다.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct AttemptRecord {
    /// 형식 판. 필드를 더할 땐 `#[serde(default)]` 로 — 판을 올리는 건 뜻이 바뀔 때만.
    pub v: u32,
    pub id: String,
    /// `ATTEMPT_*` 중 하나.
    pub state: String,
    /// 요청의 kind — 지금은 `x402-svm` 하나(서명만 하고 제출은 판매자 쪽).
    pub kind: String,
    pub chain_id: u64,
    pub started: u64,
    pub updated: u64,
    /// 끝난 뒤의 결과 — `PaymentResult` 와 같은 뜻(approved | unknown | failed).
    #[serde(default)]
    pub status: String,
    #[serde(default)]
    pub tx_hash: String,
    #[serde(default)]
    pub detail: String,
    /// x402 서명 결과(서명 갈래에서만). 크레이트마다 타입이 달라 값 그대로 싣는다.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub x402: Option<serde_json::Value>,
}

/// 결제 시도 기록을 읽는다 — **없음과 못 읽음을 가른다**. 없으면 Ok(None), 있는데 못 읽거나
/// 깨졌으면 Err. 둘 다 None 으로 접으면 `sending`·`done` 이던 기록이 깨졌을 때 「기록 없음 = 다시 승인해도 됨」·
/// 「아무것도 안 나감」이 되어 같은 요청이 두 번 나갈 수 있었다. id 가 이상하면(경로를 못 만들면) 없음이다.
pub fn read_attempt_at(dir: &Path, id: &str) -> Result<Option<AttemptRecord>, String> {
    let Some(path) = attempt_path(dir, id) else {
        return Ok(None);
    };
    match std::fs::read_to_string(&path) {
        Ok(t) => serde_json::from_str(&t).map(Some).map_err(|e| e.to_string()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// 이 기록이 있는 id 로 **승인을 다시 시작해도 되는가**(GUI `begin_approval`).
///
/// `failed` 와 기록 없음만 된다. `done`·`unknown` 은 돈이 나갔거나 나갔을 수 있다 — 같은 요청을 한 번 더
/// 승인하면 두 번째 결제다. `sending` 이 남아 있는데 진행 중인 승인이 없다면 **앱이 전송 중에 죽은 것**이라
/// 그것도 「불명」이다.
pub fn attempt_allows_retry(prev: Option<&AttemptRecord>) -> bool {
    match prev {
        None => true,
        Some(r) => r.state == ATTEMPT_FAILED || r.state == ATTEMPT_HANDED,
    }
}

/// MCP 가 승인 대기 시간을 넘긴 뒤(요청 파일을 거둔 **뒤**) 기록을 보고 할 일. (MCP 만 쓴다.)
#[allow(dead_code)]
#[derive(Debug, PartialEq)]
pub enum AfterTimeout {
    /// 승인이 시작된 적 없거나 확실히 실패했다 — 아무것도 안 나갔다. 예전의 「시간 초과」 그대로.
    NothingSent,
    /// GUI 가 아직 처리 중이다 — 조금 더 기다린다.
    StillSending,
    /// 끝났다(done | unknown) — 이 기록의 결과를 결과 파일 대신 쓴다.
    Finished,
}

#[allow(dead_code)]
pub fn after_timeout(rec: Option<&AttemptRecord>) -> AfterTimeout {
    match rec.map(|r| r.state.as_str()) {
        Some(ATTEMPT_SENDING) => AfterTimeout::StillSending,
        Some(ATTEMPT_DONE) | Some(ATTEMPT_UNKNOWN) => AfterTimeout::Finished,
        Some(ATTEMPT_HANDED) => AfterTimeout::NothingSent,
        // failed(비번 오류 뒤 사람이 떠난 경우 등)·모르는 상태·기록 없음 → 나간 것이 없다.
        // 모르는 상태를 여기로 접는 게 위험하지 않은가: 모르는 값은 새 앱이 새 상태를 만든 경우인데,
        // 그 앱은 하트비트 kinds 로 이미 걸러진다. 그래도 한 줄 남긴다 — 새 상태를 만들면 여기부터 고칠 것.
        _ => AfterTimeout::NothingSent,
    }
}

// ── 요청 임대 ──────────────────────────────────────────────────────────
//
// MCP 가 승인을 기다리다 **SIGKILL 로 죽으면** `CancelOnDrop` 이 못 돌아 요청 파일이 남고, GUI 는 그걸
// 5분+유예 동안 「살아 있는 요청」으로 보여줬다. 사람이 승인하면 돈은 나가는데 결과를 받을 상대가 없고,
// 다시 켠 AI 는 「아까 실패했다」고 알고 또 요청한다. 이제 기다리는 쪽이 요청 파일의 수정 시각을 주기적으로
// 갱신하고(임대), GUI 는 임대가 끊긴 요청을 보여주지도 승인하지도 않는다. 파일은 지우지 않는다 — 노트북이
// 잠들었다 깨어난 직후처럼 **잠깐** 끊긴 임대는 MCP 가 다시 갱신하면 되살아나야 한다.

/// 기다리는 쪽(MCP)이 요청 파일 수정 시각을 갱신하는 주기(초). (MCP 만 쓴다.)
#[allow(dead_code)]
pub const LEASE_TOUCH_SECS: u64 = 3;
/// 이만큼 갱신이 없으면 기다리는 쪽이 없다고 본다(초). 갱신 주기의 열 배 — 한두 번 밀린 것과 가른다.
pub const REQUEST_LEASE_SECS: u64 = 30;

/// 이 요청의 임대가 끊겼나. `leased` = 요청이 임대를 약속했나(옛 MCP 는 false → 늘 false),
/// `age` = 요청 파일 수정 시각으로부터 지난 초(못 읽으면 None → 끊겼다고 단정하지 않는다).
pub fn lease_lapsed(leased: bool, age: Option<u64>) -> bool {
    leased && age.is_some_and(|a| a > REQUEST_LEASE_SECS)
}

/// 이 기록 파일을 지워도 되는가(GUI 감시 스레드의 청소). 수정 시각 기준.
pub fn attempt_prunable(age_secs: u64) -> bool {
    age_secs > APPROVAL_KEEP_SECS
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 작업 이름표는 숨은 문자를 버리고 공백을 접고 80자에서 자른다.
    #[test]
    fn clip_job_cleans_and_caps() {
        assert_eq!(clip_job("  HMM 오슬로호\t 12/18  부산 입항 "), "HMM 오슬로호 12/18 부산 입항");
        assert_eq!(clip_job("오슬로\u{202E}호\u{200B}"), "오슬로호");
        assert_eq!(clip_job("\n\r "), "");
        assert_eq!(clip_job(&"가".repeat(200)).chars().count(), MAX_JOB_CHARS);
    }

    /// 새 칸(job·valid_until·undelivered)은 옛 기록에 없어도 읽히고, 빈 값은 쓰지 않는다.
    #[test]
    fn history_entry_new_fields_are_optional() {
        let old: HistoryEntry = serde_json::from_str(r#"{"ts":1,"token":"USDC","to":"a","amount":"1","status":"signed","detail":"s"}"#).unwrap();
        assert!(old.job.is_empty() && old.valid_until == 0 && !old.undelivered);
        let json = serde_json::to_string(&old).unwrap();
        assert!(!json.contains("job") && !json.contains("valid_until") && !json.contains("undelivered"));
    }

    /// 🔴 개발 71: 내 것만 지우고, 남의 요청은 제자리에 그대로 둔다(내용·이름 모두).
    #[test]
    fn remove_request_if_mine_leaves_others_alone() {
        let dir = std::env::temp_dir().join(format!("budu-req-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("payment_request.json");
        std::fs::write(&p, r#"{"id":"B","to":"0x1"}"#).unwrap();
        assert!(!remove_request_if_mine(&p, "A"));
        assert_eq!(
            std::fs::read_to_string(&p).unwrap(),
            r#"{"id":"B","to":"0x1"}"#
        );
        assert!(remove_request_if_mine(&p, "B"));
        assert!(!p.exists());
        assert!(!remove_request_if_mine(&p, "B")); // 없으면 아무 일 없음
                                                   // 떼어 온 사본이 남지 않는다.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ~/.budu 이름.
    #[test]
    fn budu_dir_name() {
        assert_eq!(
            budu_dir_in(Path::new("/Users/x")),
            PathBuf::from("/Users/x/.budu")
        );
    }

    // history 파일 형식 — settle_tx 없는 옛 기록도 기본값으로 호환.
    #[test]
    fn history_entry_roundtrips() {
        let json = r#"{"ts":1780623842,"token":"USDC","to":"0xabc","amount":"1","status":"sent","detail":"0xhash"}"#;
        let e: HistoryEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.token, "USDC");
        assert_eq!(e.status, "sent");
        assert_eq!(e.ts, 1780623842);
        assert_eq!(e.settle_tx, "");
        let back: HistoryEntry = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back.to, "0xabc");
    }

    // URL(경로의 API 키)이 에러 메시지에서 통째로 가려져야 한다.
    #[test]
    fn redact_hides_url_api_key() {
        assert_eq!(
            redact_urls("RPC 연결 실패: https://base-sepolia.g.alchemy.com/v2/SUPERSECRETKEY"),
            "RPC 연결 실패: [RPC]",
        );
        // reqwest 처럼 괄호 안에 URL 이 박힌 경우 — 키는 사라지고 뒤 메시지는 남는다.
        let red =
            redact_urls("error sending request for url (https://h/v2/KEY): connection closed");
        assert!(!red.contains("KEY"), "키가 남음: {red}");
        assert!(
            red.contains("[RPC]") && red.contains("connection closed"),
            "형태 깨짐: {red}"
        );
    }

    // host 대소문자 정규화·query 콤마 뒤 키·ws/wss 우회를 막아야 한다.
    #[test]
    fn redact_handles_case_subdelims_and_ws() {
        assert_eq!(
            redact_urls("x HTTPS://BASE.g.ALCHEMY.com/v2/KEY y"),
            "x [RPC] y"
        ); // 대소문자
        let red = redact_urls("https://rpc.example/rpc?x=a,api_key=SECRET done");
        assert!(!red.contains("SECRET"), "콤마 뒤 키가 남음: {red}");
        assert_eq!(red, "[RPC] done");
        assert_eq!(redact_urls("wss://node/abc end"), "[RPC] end"); // websocket RPC
    }

    // URL 아닌 텍스트·빈 scheme 은 그대로 둔다(멀티바이트 안전).
    #[test]
    fn redact_leaves_non_urls() {
        assert_eq!(
            redact_urls("주소 파싱 실패: bad input"),
            "주소 파싱 실패: bad input"
        );
        assert_eq!(redact_urls("just :// floating"), "just :// floating");
        assert_eq!(
            redact_urls("잔액 조회 실패: https://x/y 입니다"),
            "잔액 조회 실패: [RPC] 입니다"
        );
    }

    // settings.json 본문 → chain_id. 깨진 JSON·필드 없는 옛 파일은 테스트넷으로 접는다.
    // 🔴 다른 필드가 깨져도 chain_id 는 그대로 — 화면(Settings 파싱)도 이 답을 따라야 한다.
    #[test]
    fn chain_id_in_reads_only_chain_id() {
        assert_eq!(chain_id_in(r#"{"chain_id":101}"#), SOLANA_MAINNET_ID);
        assert_eq!(chain_id_in(r#"{"chain_id":103}"#), SOLANA_DEVNET_ID);
        assert_eq!(chain_id_in(r#"{"single_usdc":"5"}"#), FALLBACK_CHAIN_ID); // 필드 없음
        assert_eq!(chain_id_in("{ 깨진 JSON"), FALLBACK_CHAIN_ID);
        assert_eq!(chain_id_in(""), FALLBACK_CHAIN_ID);
        // 타입 틀림(문자열) → 못 읽음 → 폴백.
        assert_eq!(chain_id_in(r#"{"chain_id":"101"}"#), FALLBACK_CHAIN_ID);
        // 한도 필드가 빠졌거나 타입이 틀린 파일 — Settings 로는 못 읽지만 체인은 읽힌다.
        assert_eq!(
            chain_id_in(r#"{"single_usdc":5,"chain_id":101}"#),
            SOLANA_MAINNET_ID
        );
        // 정규화하지 않는다 — 미지원 id(옛 EVM 8453 포함)는 그대로(지원 판단은 ChainConfig 탐색 몫).
        assert_eq!(chain_id_in(r#"{"chain_id":8453}"#), 8453);
        // 🔴 폴백은 실돈 클러스터가 아니다.
        assert_ne!(FALLBACK_CHAIN_ID, SOLANA_MAINNET_ID);
    }

    // 🔴 신규(파일 없음)와 깨진 파일(있는데 못 읽음)은 다른 답이어야 한다. 신규 기본이
    // 메인넷이 된 순간 「깨졌으면 기본값」 경로는 테스트넷 사용자를 조용히 실돈 체인으로 옮기는 문이 된다.
    #[test]
    fn chain_id_for_splits_new_legacy_and_broken() {
        assert_eq!(
            chain_id_for(&SettingsFile::Missing, || false),
            NEW_INSTALL_CHAIN_ID
        );
        assert_eq!(
            chain_id_for(&SettingsFile::Missing, || true),
            FALLBACK_CHAIN_ID
        );
        // 있는데 못 읽음 → 폴백. 지갑 유무는 묻지도 않는다.
        assert_eq!(
            chain_id_for(&SettingsFile::Unreadable, || unreachable!()),
            FALLBACK_CHAIN_ID
        );
        // 본문이 있으면 지갑 유무와 무관하게 chain_id_in.
        let text = |s: &str| SettingsFile::Text(s.to_string());
        assert_eq!(
            chain_id_for(&text(r#"{"chain_id":101}"#), || unreachable!()),
            SOLANA_MAINNET_ID
        );
        assert_eq!(
            chain_id_for(&text("{ 깨진 JSON"), || unreachable!()),
            FALLBACK_CHAIN_ID
        );
        // 🔴 데모 단계: 신규 설치가 실돈 클러스터로 시작하지 않는다.
        assert_eq!(NEW_INSTALL_CHAIN_ID, SOLANA_DEVNET_ID);
    }

    // settings.json 본문 → rpc_url. 한도 필드가 깨져 Settings 로는 못 읽는 파일에서도 지정 RPC 는
    // 그대로 — GUI(Settings 파싱)와 MCP(단독 읽기)가 같은 답을 내야 한다.
    #[test]
    fn rpc_url_in_reads_only_rpc_url() {
        assert_eq!(
            rpc_url_in(r#"{"rpc_url":"https://example.invalid/v2/KEY"}"#),
            "https://example.invalid/v2/KEY"
        );
        // 앞뒤 공백은 잘린다(양쪽 effective_rpc 가 trim 하던 값).
        assert_eq!(
            rpc_url_in(r#"{"rpc_url":"  https://x.invalid  "}"#),
            "https://x.invalid"
        );
        assert_eq!(rpc_url_in(r#"{"chain_id":101}"#), ""); // 필드 없음(옛 파일) = 공식
        assert_eq!(rpc_url_in(r#"{"rpc_url":""}"#), "");
        assert_eq!(rpc_url_in("{ 깨진 JSON"), "");
        assert_eq!(rpc_url_in(""), "");
        // 타입 틀림 → 못 읽음 → 공식.
        assert_eq!(rpc_url_in(r#"{"rpc_url":7}"#), "");
        // 개발 51 하네스의 그 파일 — single_usdc 가 빠져 Settings 파싱은 실패하지만 RPC 는 살아 있다.
        assert_eq!(
            rpc_url_in(
                r#"{"daily_usdc":"20","single_eth":"0.01","daily_eth":"0.05","chain_id":101,"rpc_url":"http://127.0.0.1:8545"}"#
            ),
            "http://127.0.0.1:8545"
        );
        // 🔴 chain_id 를 못 알아보는 파일은 지정 RPC 도 버린다 — 체인은 데브넷으로
        // 접히는데 RPC 는 메인넷 것이면 「연습용」 화면 뒤에서 메인넷에 붙는다.
        let custom = r#""rpc_url":"https://base-mainnet.example/v2/KEY""#;
        // 타입 틀림·null·미지원·음수 — 전부 「못 알아봄」.
        assert_eq!(
            rpc_url_in(&format!(r#"{{"chain_id":"101",{custom}}}"#)),
            ""
        );
        assert_eq!(rpc_url_in(&format!(r#"{{"chain_id":null,{custom}}}"#)), "");
        assert_eq!(rpc_url_in(&format!(r#"{{"chain_id":1,{custom}}}"#)), ""); // 미지원
        assert_eq!(rpc_url_in(&format!(r#"{{"chain_id":-1,{custom}}}"#)), "");
        // 키 중복·실수 — Value 로 읽으면 마지막 값이 조용히 이기지만 파생은 거부한다.
        assert_eq!(
            rpc_url_in(&format!(r#"{{"chain_id":103,"chain_id":101,{custom}}}"#)),
            ""
        );
        assert_eq!(
            rpc_url_in(&format!(
                r#"{{"chain_id":101,{custom},"rpc_url":"https://other.invalid"}}"#
            )),
            ""
        );
        assert_eq!(
            rpc_url_in(&format!(r#"{{"chain_id":101.0,{custom}}}"#)),
            ""
        );
        // 지원 체인이면 유지, 필드가 없는 파일도 유지(기본 체인 — chain_id_in 과 같은 답).
        for id in SUPPORTED_CHAIN_IDS {
            assert_eq!(
                rpc_url_in(&format!(r#"{{"chain_id":{id},{custom}}}"#)),
                "https://base-mainnet.example/v2/KEY"
            );
        }
        assert_eq!(
            rpc_url_in(&format!("{{{custom}}}")),
            "https://base-mainnet.example/v2/KEY"
        );
        // 파일 갈래: 없음·못 읽음은 빈 값, 본문은 rpc_url_in.
        assert_eq!(rpc_url_for(&SettingsFile::Missing), "");
        assert_eq!(rpc_url_for(&SettingsFile::Unreadable), "");
        assert_eq!(
            rpc_url_for(&SettingsFile::Text(
                r#"{"rpc_url":"https://x.invalid"}"#.to_string()
            )),
            "https://x.invalid"
        );
    }

    // 🔴 구조 계약: **지정 RPC 를 살린 파일이면 `chain_id_in` 이 알아본 체인**
    // (지원 목록 안)이어야 한다. 한쪽만 못 읽는 파일이 하나라도 있으면 「연습용」 화면 뒤에서 메인넷
    // RPC 에 서명이 나갈 수 있다. 새 케이스는 여기에 보태라 — 두 함수가 서로 다른 파서를 쓰기 시작하면
    // 여기서 잡힌다.
    #[test]
    fn rpc_kept_implies_chain_recognized() {
        let rpc = r#""rpc_url":"https://base-mainnet.example/v2/KEY""#;
        let cases = [
            format!("{{{rpc}}}"),
            format!(r#"{{"chain_id":101,{rpc}}}"#),
            format!(r#"{{"chain_id":103,{rpc}}}"#),
            format!(r#"{{"chain_id":101,{rpc}}}"#),
                        format!(r#"{{"chain_id":1,{rpc}}}"#),
            format!(r#"{{"chain_id":"101",{rpc}}}"#),
            format!(r#"{{"chain_id":null,{rpc}}}"#),
            format!(r#"{{"chain_id":101.0,{rpc}}}"#),
            format!(r#"{{"chain_id":-101,{rpc}}}"#),
            format!(r#"{{"chain_id":103,"chain_id":101,{rpc}}}"#),
            format!(r#"{{"chain_id":101,"chain_id":103,{rpc}}}"#),
            format!(r#"{{"chain_id":101,{rpc},{rpc}}}"#),
            format!(r#"{{"single_usdc":5,"chain_id":101,{rpc}}}"#),
            format!(r#"[{{"chain_id":101,{rpc}}}]"#),
            "{ 깨진 JSON".to_string(),
        ];
        for text in &cases {
            let kept = !rpc_url_in(text).is_empty();
            let chain = chain_id_in(text);
            if kept {
                assert!(
                    SUPPORTED_CHAIN_IDS.contains(&chain),
                    "RPC 는 살았는데 체인은 못 알아봄: {text}"
                );
            }
        }
        // 살아남는 쪽도 실제로 있다(전부 버리는 구현이 이 검사를 공짜로 통과하지 않게).
        assert!(!rpc_url_in(&cases[1]).is_empty());
        assert!(!rpc_url_in(&cases[0]).is_empty());
    }

    // RPC 선택. 환경변수로 체인을 갈아탄 경우엔 settings 의 커스텀 RPC 를 버린다 —
    // 그 URL 은 **딴 체인의 엔드포인트**라 그대로 쓰면 잔액 조회가 조용히 죽는다.
    #[test]
    fn pick_rpc_drops_custom_when_env_forces_other_chain() {
        let custom = "https://base-mainnet.example/v2/KEY";
        let default = "https://sepolia.base.org";
        // 평소: 커스텀이 있으면 커스텀.
        assert_eq!(pick_rpc(custom, false, default), custom);
        // 커스텀이 비면 언제나 기본값.
        assert_eq!(pick_rpc("", false, default), default);
        assert_eq!(pick_rpc("", true, default), default);
        // 환경변수가 다른 체인을 강제 → 커스텀을 버리고 그 체인의 기본 RPC 로.
        assert_eq!(pick_rpc(custom, true, default), default);
    }

    // 파일 → 세 갈래. Unreadable 은 「디렉터리를 파일처럼 읽기」로 만든다(권한 조작 없이 재현).
    #[test]
    fn settings_file_read_classifies_io() {
        let dir = std::env::temp_dir().join(format!("budu-policy-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(
            SettingsFile::read(&dir.join("settings.json")),
            SettingsFile::Missing
        );
        std::fs::write(dir.join("settings.json"), r#"{"chain_id":101}"#).unwrap();
        assert_eq!(
            SettingsFile::read(&dir.join("settings.json")),
            SettingsFile::Text(r#"{"chain_id":101}"#.to_string())
        );
        assert_eq!(SettingsFile::read(&dir), SettingsFile::Unreadable);
        // 지갑 유무: 둘 중 하나만 있어도 true.
        assert!(!wallet_exists_in(&dir));
        std::fs::write(dir.join("wallet.json"), "{}").unwrap();
        assert!(wallet_exists_in(&dir));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    // 체인별·계정별 파일 이름: 기본 체인·계정 0 은 기존 이름 그대로(무손실), 그 외는 접미.
    #[test]
    fn data_file_names_keep_defaults_and_suffix_others() {
        assert_eq!(chain_file_name(SOLANA_DEVNET_ID, "history"), "history.json");
        assert_eq!(
            chain_file_name(SOLANA_MAINNET_ID, "history"),
            "history-101.json"
        );
        assert_eq!(chain_file_name(SOLANA_MAINNET_ID, "spend"), "spend-101.json");
        assert_eq!(
            account_file_name("history-8453.json", 0),
            "history-8453.json"
        );
        assert_eq!(account_file_name("history.json", 2), "history-a2.json");
        assert_eq!(
            account_file_name("history-8453.json", 3),
            "history-8453-a3.json"
        );
        // 둘을 이어 쓰는 실제 경로.
        assert_eq!(
            account_file_name(&chain_file_name(SOLANA_MAINNET_ID, "history"), 2),
            "history-101-a2.json"
        );
    }

    // 계정 정규화: 옛 파일(목록 없음)은 계정 0 하나, 주소는 address 필드.
    #[test]
    fn legacy_wallet_normalizes_to_single_account() {
        let list = normalize_accounts("0xAbc", &[]);
        assert_eq!(
            list,
            vec![Account {
                index: 0,
                address: "0xAbc".into(),
                label: String::new()
            }]
        );
        assert_eq!(pick_active(&list, 0).address, "0xAbc");
        assert_eq!(pick_active(&list, 7).index, 0); // 없는 계정 → 0
    }

    // 인덱스 순 정렬, 계정 0 의 주소는 address 필드가 이긴다(라벨은 목록 것), 중복 인덱스는 하나로,
    // 활성이 목록에 없으면 계정 0.
    #[test]
    fn accounts_sorted_zero_from_address_and_active_fallback() {
        let listed = vec![
            Account {
                index: 2,
                address: "0xTwo".into(),
                label: "AI".into(),
            },
            Account {
                index: 0,
                address: "0xStale".into(),
                label: "나".into(),
            },
            Account {
                index: 1,
                address: "0xOne".into(),
                label: String::new(),
            },
            Account {
                index: 2,
                address: "0xDup".into(),
                label: String::new(),
            },
        ];
        let list = normalize_accounts("0xZero", &listed);
        assert_eq!(
            list.iter().map(|a| a.index).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(list[0].address, "0xZero"); // 정본은 address 필드
        assert_eq!(list[0].label, "나");
        assert_eq!(list[2].address, "0xTwo"); // 중복은 먼저 온 것(안정 정렬)
        assert_eq!(pick_active(&list, 2).address, "0xTwo");
        assert_eq!(pick_active(&list, 9).index, 0);
    }

    // 🔴 결제 시도 기록 — 다시 승인해도 되는 건 「기록 없음」과 「확실한 실패」뿐.
    #[test]
    fn attempt_retry_and_timeout_rules() {
        let mk = |state: &str| AttemptRecord {
            v: 1,
            id: "1".into(),
            state: state.into(),
            kind: "transfer".into(),
            chain_id: 5042,
            started: 1,
            updated: 1,
            status: String::new(),
            tx_hash: String::new(),
            detail: String::new(),
            x402: None,
        };
        assert!(attempt_allows_retry(None));
        assert!(attempt_allows_retry(Some(&mk(ATTEMPT_FAILED))));
        // 자율이 사람에게 넘긴 것 — 사람이 곧 승인할 수 있어야 하고, 시간 초과면 아무것도 안 나갔다.
        assert!(attempt_allows_retry(Some(&mk(ATTEMPT_HANDED))));
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_HANDED))),
            AfterTimeout::NothingSent
        );
        for s in [ATTEMPT_SENDING, ATTEMPT_DONE, ATTEMPT_UNKNOWN, "weird"] {
            assert!(
                !attempt_allows_retry(Some(&mk(s))),
                "{s} 는 다시 승인하면 안 된다"
            );
        }
        assert_eq!(after_timeout(None), AfterTimeout::NothingSent);
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_FAILED))),
            AfterTimeout::NothingSent
        );
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_SENDING))),
            AfterTimeout::StillSending
        );
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_DONE))),
            AfterTimeout::Finished
        );
        assert_eq!(
            after_timeout(Some(&mk(ATTEMPT_UNKNOWN))),
            AfterTimeout::Finished
        );
        // 옛 필드만 있는 기록도 읽힌다(결과 필드는 기본값).
        let r: AttemptRecord = serde_json::from_str(
            r#"{"v":1,"id":"9","state":"sending","kind":"x402","chain_id":1,"started":1,"updated":1}"#,
        )
        .unwrap();
        assert_eq!(r.status, "");
        assert!(r.x402.is_none());
    }

    // id 가 경로를 벗어나지 못한다.
    /// 🔴 개발 73: 결제 시도 기록은 「없음」과 「있는데 깨짐」을 가른다.
    #[test]
    fn read_attempt_separates_missing_from_broken() {
        let d = std::env::temp_dir().join(format!("budu-attempt-{}-{:?}", std::process::id(), std::thread::current().id()));
        std::fs::create_dir_all(d.join(APPROVALS_DIR)).unwrap();
        assert_eq!(read_attempt_at(&d, "1789"), Ok(None));
        std::fs::write(attempt_path(&d, "1789").unwrap(), "{\"v\":1,\"id\":").unwrap();
        assert!(read_attempt_at(&d, "1789").is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn attempt_paths_refuse_odd_ids() {
        let d = Path::new("/h/.budu");
        assert_eq!(
            attempt_path(d, "1789").unwrap(),
            PathBuf::from("/h/.budu/approvals/1789.json")
        );
        assert_eq!(
            proof_path(d, "1789").unwrap(),
            PathBuf::from("/h/.budu/approvals/1789.proof.json")
        );
        for bad in ["", "../x", "a/b", "1.json", &"9".repeat(65)] {
            assert!(attempt_path(d, bad).is_none(), "{bad}");
        }
        assert!(
            !lease_lapsed(false, Some(10_000)),
            "옛 MCP 요청엔 임대 규칙이 없다"
        );
        assert!(
            !lease_lapsed(true, None),
            "수정 시각을 못 읽으면 끊겼다고 단정하지 않는다"
        );
        assert!(!lease_lapsed(true, Some(REQUEST_LEASE_SECS)));
        assert!(lease_lapsed(true, Some(REQUEST_LEASE_SECS + 1)));
        assert!(!attempt_prunable(APPROVAL_KEEP_SECS));
        assert!(attempt_prunable(APPROVAL_KEEP_SECS + 1));
    }
}
