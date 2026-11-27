// 읽기 전용 지갑 조회 — ~/.budu 파일을 읽고 솔라나 JSON-RPC 로 잔액을 본다(shared/solana.rs).
//
// 비밀(니모닉/키)은 절대 건드리지 않는다. wallet.enc 의 평문 address 필드만 읽고,
// 잔액은 공개 RPC로 조회한다. → 비번 없이도 안전하게 노출 가능한 정보들.
//
// ⚠️ 클러스터 기본값(RPC·USDC 민트·decimals·genesis)은 chain.rs 의 active_chain() 에서 온다. 실제 RPC는
//    effective_rpc() 가 settings.json(GUI와 공유)을 **GUI 와 같은 함수**(shared/policy.rs)로
//    읽어 결정하므로 사용자가 RPC를 바꾸면 두 프로세스가 자동으로 같은 RPC를 쓴다. default_rpc 는
//    설정이 비었을 때의 폴백.

use crate::{tf, ts};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;

use crate::chain::{active_chain, chain_file};
use crate::policy::{self, SettingsFile};
use crate::solana::{self, BalanceError, BalanceQuery};

pub fn budu_dir() -> Result<PathBuf, String> {
    let home = dirs::home_dir().ok_or(ts!(
        "홈 디렉터리를 찾을 수 없습니다",
        "Couldn't find the home folder"
    ))?;
    Ok(policy::budu_dir_in(&home))
}

fn enc_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("wallet.enc"))
}

fn legacy_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("wallet.json"))
}

/// 활성 계정의 내역 파일 (체인별 + 계정별). src-tauri 의 account_file 과 같은 규칙.
/// 계정 0 폴백은 **wallet.enc 가 없을 때뿐**(옛 평문 wallet.json 만 있는 지갑 — 계정이 하나고
/// 내역은 예전 이름 그대로다. 여기서 에러를 내면 내역이 통째로 빈다).
/// 파일은 있는데 못 읽거나 깨졌으면 에러 — 그것까지 0 으로 접으면 활성이 딴 계정일 때 **남의
/// 계정 내역**을 활성 계정 것처럼 AI 에게 준다. read_history 는 에러를
/// 빈 목록으로 다룬다.
fn history_path() -> Result<PathBuf, String> {
    let index = history_account_index(enc_path()?.exists(), active_account)?;
    Ok(budu_dir()?.join(account_file_name(&chain_file("history"), index)))
}

/// 내역 파일의 계정 인덱스 — 순수 판정(테스트용). 암호화 지갑이 없으면 0, 있으면 활성 계정을
/// 읽되 실패는 그대로 에러(폴백 금지).
fn history_account_index(
    enc_exists: bool,
    active: impl FnOnce() -> Result<Account, String>,
) -> Result<u32, String> {
    if enc_exists {
        Ok(active()?.index)
    } else {
        Ok(0)
    }
}

/// 계정별 데이터 파일 이름 — GUI 와 같은 함수(shared/policy.rs). 어긋나면 GUI 가 적은
/// 내역을 AI 가 못 본다.
pub use crate::policy::account_file_name;

fn settings_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("settings.json"))
}

/// 잔액·내역 조회·x402 제출이 붙는 RPC. 판정은 `policy::rpc_url_for` + `policy::pick_rpc` — GUI
/// `effective_rpc` 와 **같은 함수**로 같은 settings.json 을 읽으므로 두 프로세스의 RPC 가 어긋날 길이
/// 없다(개발 57. 그 전엔 GUI 가 Settings 전체를 파싱해 한도 필드 하나 깨진 파일에서 공식 RPC 로 접는
/// 동안 여기는 rpc_url 만 읽어 커스텀 RPC 를 썼다 — 개발 51 하네스 실측). 지정한 게 없으면 활성 체인의
/// 공식 RPC. 단, 환경변수로 체인을 갈아탄 경우엔 그 rpc_url 이 딴 체인 것이므로 쓰지 않는다(pick_rpc 주석).
pub fn effective_rpc() -> String {
    let file = settings_path()
        .map(|p| SettingsFile::read(&p))
        .unwrap_or(SettingsFile::Unreadable);
    policy::pick_rpc(
        &policy::rpc_url_for(&file),
        crate::chain::env_forces_other_chain(),
        active_chain().default_rpc,
    )
}

/// AI(MCP)로 나가는 문자열의 URL 을 `[RPC]` 로 가린다 — 정본은 `policy::redact_urls`(GUI 와 같은 함수).
pub use crate::policy::redact_urls;

/// 계정 하나 — 같은 시드의 HD 파생 인덱스 + 주소(공개정보) + 사람이 붙인 라벨.
/// GUI 와 같은 타입(shared/policy.rs) — wallet.enc 의 항목이자 MCP 상태의 항목.
pub use crate::policy::Account;

/// 지갑 상태 + 주소. 프론트/에이전트가 어떤 상태인지 알 수 있게.
/// - "encrypted": 정상 (wallet.enc 존재)
/// - "legacy":    평문 wallet.json 만 존재 (앱에서 비번 설정 필요)
/// - "none":      지갑 없음
#[derive(Serialize)]
pub struct WalletStatus {
    pub state: String,
    /// **활성 계정**의 주소. 잔액·내역·결제 요청이 이 계정 기준이다.
    pub address: Option<String>,
    pub backed_up: bool,
    /// 활성 계정의 파생 인덱스.
    pub account: u32,
    /// 모든 계정(인덱스 순). encrypted 가 아니면 비어 있다. 계정 전환은 앱에서만 한다.
    pub accounts: Vec<Account>,
}

/// wallet.enc 에서 공개 정보(주소·백업여부·계정 목록)만 읽는다. 니모닉(ciphertext)은 무시.
#[derive(Deserialize)]
struct EncMeta {
    address: String,
    #[serde(default)]
    backed_up: bool,
    /// 계정 목록. 옛 파일엔 없다 → 계정 0(= address) 하나.
    #[serde(default)]
    accounts: Vec<Account>,
    #[serde(default)]
    active: u32,
}

impl EncMeta {
    /// 모든 계정을 인덱스 순으로 — 계정 0 은 항상 있고 그 주소는 `address` 필드(정본).
    /// 정규화는 GUI(EncryptedWallet::accounts)와 **같은 함수**(shared/policy.rs).
    fn accounts(&self) -> Vec<Account> {
        crate::policy::normalize_accounts(&self.address, &self.accounts)
    }

    /// 활성 계정. `active` 가 목록에 없으면 계정 0 (GUI 와 같은 함수).
    fn active_account(&self) -> Account {
        crate::policy::pick_active(&self.accounts(), self.active)
    }
}

fn read_enc_meta() -> Result<EncMeta, String> {
    let data = fs::read_to_string(enc_path()?).map_err(|e| {
        tf!(
            "지갑 파일 읽기 실패: {e}",
            "Couldn't read the wallet file: {e}"
        )
    })?;
    serde_json::from_str(&data).map_err(|e| {
        tf!(
            "지갑 파일 파싱 실패: {e}",
            "Couldn't parse the wallet file: {e}"
        )
    })
}

/// 지금 활성인 계정 — 결제 요청에 각인하고(GUI 가 승인 시 대조) 내역 파일을 고를 때 쓴다.
/// 암호화 지갑이 없으면 오류(옛 평문 지갑은 계정이 하나라 결제 요청 자체가 앱에서 막힌다).
pub fn active_account() -> Result<Account, String> {
    Ok(read_enc_meta()?.active_account())
}

#[derive(Deserialize)]
struct LegacyMeta {
    address: String,
}

/// 지갑 파일 상태를 알려준다 (비번 불필요).
pub fn wallet_status() -> Result<WalletStatus, String> {
    if enc_path()?.exists() {
        let m = read_enc_meta()?;
        let active = m.active_account();
        return Ok(WalletStatus {
            state: "encrypted".into(),
            address: Some(active.address),
            backed_up: m.backed_up,
            account: active.index,
            accounts: m.accounts(),
        });
    }
    if legacy_path()?.exists() {
        let data = fs::read_to_string(legacy_path()?).map_err(|e| {
            tf!(
                "지갑 파일 읽기 실패: {e}",
                "Couldn't read the wallet file: {e}"
            )
        })?;
        let m: LegacyMeta = serde_json::from_str(&data).map_err(|e| {
            tf!(
                "지갑 파일 파싱 실패: {e}",
                "Couldn't parse the wallet file: {e}"
            )
        })?;
        return Ok(WalletStatus {
            state: "legacy".into(),
            address: Some(m.address),
            backed_up: false,
            account: 0,
            accounts: Vec::new(),
        });
    }
    Ok(WalletStatus {
        state: "none".into(),
        address: None,
        backed_up: false,
        account: 0,
        accounts: Vec::new(),
    })
}

/// 잔액 — 십진수 문자열. `sol` = 네이티브(수수료) 토큰 — x402 결제엔 필요 없다(수수료는 페이실리테이터가 낸다).
/// `usdc` = 결제에 쓰는 돈. 내 USDC ATA 가 아직 없으면 0. 앱(src-tauri balance.rs)과 같은 모양.
#[derive(Serialize)]
pub struct Balances {
    pub sol: String,
    pub usdc: String,
}

/// 실패 갈래 → 사람 말 (순수 — 테스트가 직접 부른다). URL 은 가린다. 앱 balance.rs 의 같은 함수와 같은 문구.
fn describe(e: BalanceError, want_genesis: &str) -> String {
    match e {
        // 실제로 본 원인은 EVM 시절 파일의 `0x…` 주소 — 앱의 「옛 형식 지갑」 화면이 비번 한 번으로 고친다.
        BalanceError::BadAddress => ts!(
            "지갑 주소가 올바르지 않아요 — 옛 형식 지갑이면 부두 앱을 열어 비밀번호를 넣으면 고쳐져요",
            "The wallet address isn't valid — if it's an old-format wallet, open the Budu app and enter your password to fix it"
        )
        .to_string(),
        BalanceError::WrongCluster(got) => {
            let got_name = solana::cluster_name_of(&got).unwrap_or("?");
            let want_name = solana::cluster_name_of(want_genesis).unwrap_or("?");
            tf!(
                "지정한 RPC 가 다른 클러스터({got_name})에 붙어 있어요 — 고른 네트워크({want_name})와 달라서 잔액을 보여주지 않아요. 설정에서 RPC 를 비우거나 맞는 주소로 바꾸세요.",
                "The custom RPC is on a different cluster ({got_name}) than the network you picked ({want_name}), so the balance isn't shown. Clear the RPC in Settings or point it at the right cluster."
            )
        }
        // getGenesisHash 를 모르는 서버 = 솔라나 RPC 가 아니다(옛 EVM RPC 등) — «연결 실패» 로 부르면 오해한다.
        BalanceError::Genesis(solana::RpcError::Rpc { code: -32601, .. }) => ts!(
            "지정한 RPC 가 솔라나 RPC 가 아니에요. 설정에서 RPC 를 비우거나 솔라나 주소로 바꾸세요.",
            "The custom RPC isn't a Solana RPC. Clear it in Settings or use a Solana RPC address."
        )
        .to_string(),
        BalanceError::Genesis(e) => tf!(
            "RPC 연결 실패: {}",
            "Couldn't reach the RPC server: {}",
            redact_urls(&e.to_string())
        ),
        BalanceError::Sol(e) => tf!(
            "SOL 잔액 조회 실패: {}",
            "Couldn't read the SOL balance: {}",
            redact_urls(&e.to_string())
        ),
        BalanceError::Usdc(e) => tf!(
            "USDC 잔액 조회 실패: {}",
            "Couldn't read the USDC balance: {}",
            redact_urls(&e.to_string())
        ),
    }
}

/// 지갑 주소의 SOL + USDC 잔액을 활성 클러스터에서 조회한다 (지정 RPC 가 딴 클러스터면 거절 — shared/solana.rs).
pub async fn get_balances(address: &str) -> Result<Balances, String> {
    let chain = active_chain();
    let rpc = effective_rpc();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(20))
        .build()
        .map_err(|e| tf!("HTTP 준비 실패: {e}", "Couldn't set up HTTP: {e}"))?;
    let q = BalanceQuery {
        rpc_url: &rpc,
        owner: address.trim(),
        usdc_mint: chain.usdc_mint,
        usdc_decimals: chain.usdc_decimals,
        genesis_hash: chain.genesis_hash,
    };
    let raw = solana::fetch_balances(&client, &q)
        .await
        .map_err(|e| describe(e, chain.genesis_hash))?;
    Ok(Balances {
        sol: solana::format_units(raw.lamports, 9),
        usdc: solana::format_units(raw.usdc_units, chain.usdc_decimals),
    })
}

/// 거래 내역 1건 — src-tauri 가 기록한 history 파일을 **같은 타입**으로 읽는다(`policy::HistoryEntry`).
pub use crate::policy::HistoryEntry;

/// 저장된 거래 내역을 읽는다 (최신순). 없거나 깨졌으면 빈 목록.
/// detail 은 출력 시점에 redact — 이번 패치 이전(또는 다른 빌드)이 기록한 비redact 에러에
/// RPC URL·키가 들어 있어도 get_history(→AI)로 다시 새지 않게 한다.
///
/// `limit` = 돌려줄 줄 수. 보낸 기록은 본 파일(최신 200건) 다음에 **보관 파일**까지 읽는다(개발 70 — 예전엔
/// 200건을 넘친 기록이 지워져서 여기서도 200건이 끝이었다). 보관 파일은 본 파일이 모자랄 때만 연다.
///
/// 주인이 다른 기록(지갑 파일을 지우고 다른 시드를 가져온 뒤 남은 옛 기록)은 뺀다. 주소를 못 읽으면
/// (옛 평문 지갑 등) 거르지 않는다 — 예전과 같다.
pub fn read_history(limit: usize) -> Vec<HistoryEntry> {
    let owner = active_account().map(|a| a.address).unwrap_or_default();
    let mut entries: Vec<HistoryEntry> = history_path()
        .map(|p| crate::policy::read_sent_history(&p, limit, &owner))
        .unwrap_or_default();
    for e in &mut entries {
        e.detail = redact_urls(&e.detail);
    }
    let mut list = entries;
    list.truncate(limit);
    list
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 지정 RPC 가 딴 클러스터면 잔액을 내지 않는다 — 판정은 shared/solana.rs(genesis 대조), 여기는 문구.
    /// 원시 오류의 URL(지정 RPC 의 API 키)은 AI 에게 가기 전에 가려진다.
    #[test]
    fn balance_errors_name_clusters_and_redact() {
        let m = describe(
            BalanceError::WrongCluster(solana::GENESIS_MAINNET.into()),
            solana::GENESIS_DEVNET,
        );
        assert!(m.contains("mainnet-beta") && m.contains("devnet"), "{m}");
        let m = describe(
            BalanceError::Sol(solana::RpcError::Http(
                "error sending request for url (https://rpc.example/v2/SECRET)".into(),
            )),
            solana::GENESIS_DEVNET,
        );
        assert!(!m.contains("SECRET"), "{m}");
    }

    // RPC 선택(pick_rpc)·rpc_url 읽기 테스트는 policy::tests 가 본다(정본이 그쪽으로 갔다).

    /// 핵심 계약: wallet.enc 를 읽을 때 주소만 가져오고 ciphertext(니모닉)는 무시한다.
    /// EncMeta 에 ciphertext 필드 자체가 없으므로 역직렬화 결과에 비밀이 들어올 수 없다.
    #[test]
    fn enc_meta_reads_address_and_ignores_secrets() {
        let json = r#"{
            "version": 2,
            "address": "0x8b7ba5077d261739f5FeBB31B10167671e590161",
            "salt": "c2FsdA==",
            "nonce": "bm9uY2U=",
            "ciphertext": "c2VjcmV0LW1uZW1vbmlj",
            "backed_up": true
        }"#;
        let m: EncMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.address, "0x8b7ba5077d261739f5FeBB31B10167671e590161");
        assert!(m.backed_up);
        // EncMeta 구조체엔 ciphertext/salt/nonce 필드가 없다 → 컴파일 타임에 비밀 차단.
    }

    /// 계정 목록: 옛 파일은 계정 0 하나, 새 파일은 인덱스 순 + address 필드가 0 의 정본,
    /// 없는 active 는 0 으로. 규칙 자체는 policy::tests 가 보고, 여기는 EncMeta 역직렬화가 그 함수에
    /// 제대로 물리는지(필드 이름·serde 기본값)만 본다.
    #[test]
    fn enc_meta_accounts_normalize_like_gui() {
        let legacy: EncMeta = serde_json::from_str(
            r#"{"version":3,"address":"0xAbc","salt":"x","nonce":"y","ciphertext":"z"}"#,
        )
        .unwrap();
        assert_eq!(legacy.accounts().len(), 1);
        assert_eq!(legacy.active_account().address, "0xAbc");

        let m: EncMeta = serde_json::from_str(
            r#"{"version":3,"address":"0xZero","salt":"x","nonce":"y","ciphertext":"z",
                "accounts":[{"index":2,"address":"0xTwo","label":"AI"},{"index":0,"address":"0xStale"}],
                "active":2}"#,
        )
        .unwrap();
        let list = m.accounts();
        assert_eq!(list.iter().map(|a| a.index).collect::<Vec<_>>(), vec![0, 2]);
        assert_eq!(list[0].address, "0xZero");
        assert_eq!(m.active_account(), list[1]);
        let mut gone = m;
        gone.active = 9;
        assert_eq!(gone.active_account().index, 0);
    }

    /// 내역 파일의 계정: 계정 0 폴백은 wallet.enc 가 없을 때뿐. 파일이 있는데 깨졌으면 에러 —
    /// 0 으로 접으면 남의 계정 내역을 준다.
    #[test]
    fn history_account_index_falls_back_to_zero_only_without_enc() {
        let broken = || Err::<Account, String>("broken".into());
        assert_eq!(history_account_index(false, broken), Ok(0));
        assert!(history_account_index(true, broken).is_err());
        let active = || {
            Ok(Account {
                index: 2,
                address: "0xabc".into(),
                label: String::new(),
            })
        };
        assert_eq!(history_account_index(true, active), Ok(2));
    }

    /// 옛 파일에 backed_up 필드가 없어도 기본값 false 로 로드된다.
    #[test]
    fn enc_meta_defaults_backed_up_false() {
        let json = r#"{"version":2,"address":"0xabc","salt":"x","nonce":"y","ciphertext":"z"}"#;
        let m: EncMeta = serde_json::from_str(json).unwrap();
        assert!(!m.backed_up);
    }

    /// 평문 legacy 파일에서도 주소만 읽는다 (니모닉 필드는 무시).
    #[test]
    fn legacy_meta_reads_only_address() {
        let json = r#"{"mnemonic":"word1 word2 ...","address":"0xdef"}"#;
        let m: LegacyMeta = serde_json::from_str(json).unwrap();
        assert_eq!(m.address, "0xdef");
    }

    // HistoryEntry 형식·redact_urls 테스트는 policy::tests 가 본다(정본이 그쪽으로 갔다).
    // 여기 남은 것은 **이 크레이트의 읽기 경로**가 그 타입·함수를 실제로 물고 있는지 보는 검사다.

    /// 과거 history 의 비redact detail 도 읽기 시점에 가려진다.
    #[test]
    fn read_history_redacts_detail() {
        let e = HistoryEntry {
            ts: 1,
            token: "ETH".into(),
            to: "0xabc".into(),
            amount: "0".into(),
            status: "failed".into(),
            detail: "RPC 연결 실패: https://base.alchemy.com/v2/LEAKEDKEY".into(),
            settle_tx: String::new(),
            ..Default::default()
        };
        // read_history 가 적용하는 redact 를 같은 함수로 검증(파일 I/O 없이).
        let red = redact_urls(&e.detail);
        assert!(!red.contains("LEAKEDKEY"), "{red}");
        assert_eq!(red, "RPC 연결 실패: [RPC]");
    }
}
