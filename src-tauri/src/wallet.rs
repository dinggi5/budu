// 지갑 키 관리 (Session 2~5) — 니모닉 생성·주소 파생·비번 암호화(Argon2id + AES-256-GCM).
// 키는 ~/.budu/wallet.enc 에 암호화 저장. 비번이 있어야만 복호화/서명 가능.
// 기존 평문 wallet.json 은 최초 1회 마이그레이션으로 암호화 후 삭제.
//
// 계정 여러 개 — 시드는 하나, 계정은 HD 파생으로 늘린다.
// 파생이 솔라나로 바뀌었다 — SLIP-0010 ed25519 m/44'/501'/{n}'/0' (keys.rs, 팬텀과 같은 경로). 주소 = base58.
// 백업은 여전히 12단어 하나다(그 시드에서 모든 계정이 다시 나온다). 계정 목록·활성 계정은
// wallet.enc 의 평문 필드(주소는 공개정보)라 MCP 도 비번 없이 읽는다.

use crate::i18n::{tf, ts};
use aes_gcm::aead::rand_core::RngCore;
use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use argon2::Argon2;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use zeroize::Zeroizing;

use crate::chain::chain_file;
use crate::keys::{self, KeyError, SolKeypair};
use crate::policy;
use crate::store::budu_dir;

/// 계정 라벨 최대 글자 수(사람이 붙이는 이름 — 화면 한 줄).
const LABEL_MAX_CHARS: usize = 24;

/// KDF(Argon2id) 파라미터 — wallet.enc v3 (메인넷 전 보안 강화).
/// RFC 9106의 메모리 제약 환경 권장값(64 MiB, t=3). 비번 1회 입력당 ~0.5초 수준의 비용으로
/// 오프라인 무차별 대입(파일 탈취 후 비번 추측)을 크게 비싸게 만든다.
const KDF_M_V3: u32 = 65536; // KiB = 64 MiB
const KDF_T_V3: u32 = 3;
const KDF_P_V3: u32 = 1;

/// v2 시절 파라미터 = argon2 crate 0.5의 `Argon2::default()` (19 MiB, t=2, p=1).
/// kdf 필드가 없는 옛 파일은 이 값으로 복호화해야 한다 — 바꾸면 기존 지갑이 안 열린다.
const KDF_M_V2: u32 = 19_456;
const KDF_T_V2: u32 = 2;
const KDF_P_V2: u32 = 1;

/// 파일에서 읽은 KDF 비용의 상한 — v3 의 4배·몇 배 위. 이보다 크면 손상으로 본다.
const KDF_M_MAX: u32 = 262_144; // KiB = 256 MiB
const KDF_T_MAX: u32 = 10;
const KDF_P_MAX: u32 = 4;

fn default_kdf_m() -> u32 {
    KDF_M_V2
}
fn default_kdf_t() -> u32 {
    KDF_T_V2
}
fn default_kdf_p() -> u32 {
    KDF_P_V2
}

/// 프론트로 넘기는 지갑 정보. 니모닉/키는 절대 포함하지 않는다.
#[derive(Serialize)]
pub(crate) struct WalletInfo {
    address: String,
}

/// 계정 하나 — 같은 시드의 HD 파생 인덱스 + 그 주소. 주소는 공개정보라 평문이고,
/// 비번 없이 잔액·QR·MCP 상태에 쓴다. 라벨은 사람이 붙이는 이름(빈 값 = 화면이 「계정 N」으로).
/// MCP 와 같은 타입(shared/policy.rs) — wallet.enc 의 항목을 두 프로세스가 같은 모양으로 읽는다.
pub(crate) use crate::policy::Account;

/// 지갑 파일 상태. 프론트가 어떤 화면을 띄울지 결정한다.
/// - "encrypted": wallet.enc 존재 (정상)
/// - "legacy":    평문 wallet.json 만 존재 → 비번으로 보호 필요(마이그레이션)
/// - "none":      지갑 없음 → 새로 생성
#[derive(Serialize)]
pub(crate) struct WalletStatus {
    state: String,
    /// **활성 계정**의 주소. 잔액·받기·보내기 화면이 이 주소를 쓴다.
    address: Option<String>,
    /// 시드 백업 완료 여부 (encrypted 상태일 때만 의미 있음).
    backed_up: bool,
    /// 모든 계정(인덱스 순). encrypted 가 아니면 비어 있다.
    accounts: Vec<Account>,
    /// 활성 계정의 파생 인덱스.
    active: u32,
    /// 평문 주소가 솔라나 꼴이 아닌 계정이 있다. 화면은 비번 칸을 띄워
    /// `repair_wallet_address` 로 고친다 — 그 전엔 잔액·받기·결제가 전부 그 주소에서 막힌다.
    legacy_address: bool,
}

/// 아직 암호화 지갑이 없는가 (= 첫 화면이 설정/가져오기여야 하는가).
/// 메뉴바 앱은 평소엔 조용히 트레이에만 있지만, 첫 실행에는 팝오버를 띄워야 한다.
/// 읽기에 실패하면 보수적으로 true — 안내 화면을 띄우는 쪽이 안전하다.
pub(crate) fn needs_setup() -> bool {
    get_wallet_status()
        .map(|s| s.state != "encrypted")
        .unwrap_or(true)
}

/// 암호화된 지갑 파일 (v2/v3). 주소만 평문(공개정보)이라 비번 없이도 잔액/QR 표시 가능.
/// v3 = KDF 파라미터를 파일에 명시 + 강화값(KDF_*_V3). v2 파일은 kdf 필드가 없어서
/// serde default(=옛 기본값)로 복호화되고, 비번 잠금 해제 성공 시 v3로 재암호화된다.
#[derive(Serialize, Deserialize, Clone)]
pub(crate) struct EncryptedWallet {
    version: u32,
    pub(crate) address: String,
    salt: String,       // base64
    nonce: String,      // base64
    ciphertext: String, // base64 (암호화된 니모닉)
    /// 사용자가 시드 12단어를 직접 백업했는지. 옛 파일엔 없어서 기본값 false.
    #[serde(default)]
    backed_up: bool,
    /// Argon2id 파라미터 (KiB / 반복 / 병렬). 옛 v2 파일엔 없어서 v2 기본값으로 폴백.
    #[serde(default = "default_kdf_m")]
    kdf_m: u32,
    #[serde(default = "default_kdf_t")]
    kdf_t: u32,
    #[serde(default = "default_kdf_p")]
    kdf_p: u32,
    /// 계정 목록. 옛 파일엔 없다 → `accounts()` 가 계정 0(= `address`) 하나로 정규화.
    /// 계정 0 은 여기 있든 없든 `address` 가 정본이다(옛 빌드도 그 필드를 읽는다).
    #[serde(default)]
    accounts: Vec<Account>,
    /// 활성 계정의 파생 인덱스. 옛 파일엔 없다 → 0.
    #[serde(default)]
    active: u32,
}

impl EncryptedWallet {
    /// 모든 계정을 인덱스 순으로 — 계정 0 은 항상 있고 그 주소는 `address` 필드다.
    /// (파일의 `accounts` 에 0 이 따로 적혀 있어도 `address` 가 이긴다 — 정본은 하나.)
    /// 정규화는 MCP(EncMeta::accounts)와 **같은 함수**(shared/policy.rs) — 두 프로세스가 같은 계정을 본다.
    pub(crate) fn accounts(&self) -> Vec<Account> {
        policy::normalize_accounts(&self.address, &self.accounts)
    }

    /// 활성 계정. `active` 가 목록에 없으면(손상·옛 빌드가 쓴 파일) 계정 0 — 돈이 나가는
    /// 계정이 「없는 계정」이 되면 안 된다. (MCP 와 같은 함수.)
    pub(crate) fn active_account(&self) -> Account {
        policy::pick_active(&self.accounts(), self.active)
    }
}

/// 옛 평문 형식 (v1). 읽기 전용 — 마이그레이션 때만 사용.
#[derive(Deserialize)]
struct LegacyWallet {
    mnemonic: String,
    address: String,
}

fn enc_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("wallet.enc"))
}

fn legacy_path() -> Result<PathBuf, String> {
    Ok(budu_dir()?.join("wallet.json"))
}

/// 니모닉 문구에서 계정 `index` 의 키쌍을 파생한다.
/// 경로는 솔라나 표준 m/44'/501'/{index}'/0' — 팬텀·솔플레어와 같은 순서라,
/// 이 12단어를 다른 지갑에 넣어도 같은 계정들이 같은 순서로 나온다(keys.rs 테스트가 팬텀 주소로 고정).
pub(crate) fn signer_for_account(phrase: &str, index: u32) -> Result<SolKeypair, String> {
    keys::keypair_for_account(phrase, index).map_err(|e| match e {
        KeyError::InvalidPhrase => ts!(
            "복구 문구가 올바르지 않아 키를 만들지 못했어요",
            "Couldn't derive the key: the recovery phrase isn't valid"
        )
        .to_string(),
        KeyError::IndexTooLarge => ts!(
            "계정 번호가 너무 커요",
            "That account number is too large"
        )
        .to_string(),
    })
}

/// 니모닉 문구에서 계정 0 의 키쌍을 파생한다 (주소 파생·검증용).
pub(crate) fn signer_from_phrase(phrase: &str) -> Result<SolKeypair, String> {
    signer_for_account(phrase, 0)
}

/// 니모닉 문구에서 계정 0 의 솔라나 주소(base58)를 파생한다.
fn derive_address(phrase: &str) -> Result<String, String> {
    Ok(signer_from_phrase(phrase)?.address())
}

tokio::task_local! {
    /// 한 작업(송금/서명/승인) 동안 고정된 계정 인덱스. chain.rs 의 PINNED_CHAIN 과
    /// 같은 처방 — 승인 도중 사용자가 계정을 바꿔도 **서명하는 키와 내역이 적히는 파일이
    /// 요청의 계정에 머문다**. 안 묶으면 「화면 계정 ≠ 돈 나가는 계정」이 된다.
    static PINNED_ACCOUNT: u32;
}

/// 작업 단위로 계정을 고정해 fut 를 실행한다. 이미 고정된 안쪽에서 다시 감싸면 안쪽 값이 이긴다 —
/// 호출자는 항상 `active_account_index()`(= 고정값 우선)를 넘기므로 실제로는 같은 값이다.
/// 안쪽 future 는 힙에 올린다 — 이유는 `chain::with_pinned_chain` 주석(층마다 4배 → 스택 넘침 크래시).
pub(crate) fn with_pinned_account<F: std::future::Future>(
    index: u32,
    fut: F,
) -> impl std::future::Future<Output = F::Output> {
    PINNED_ACCOUNT.scope(index, Box::pin(fut))
}

/// 지금 돈이 나가는 계정의 인덱스 — 작업이 고정했으면 그 값, 아니면 wallet.enc 의 활성 계정.
/// 파일을 못 읽으면 0(원래 계정) — 계정 0 은 어떤 wallet.enc 에도 있다.
pub(crate) fn active_account_index() -> u32 {
    if let Ok(i) = PINNED_ACCOUNT.try_with(|i| *i) {
        return i;
    }
    read_encrypted()
        .map(|w| w.active_account().index)
        .unwrap_or(0)
}

/// 활성 계정(고정값 우선)의 주소까지. 고정된 인덱스가 파일 목록에 없으면 오류 —
/// 요청이 가리키는 계정이 이 지갑에 없다는 뜻이라 조용히 다른 계정으로 갈 수 없다.
pub(crate) fn active_account() -> Result<Account, String> {
    let w = read_encrypted()?;
    let index = PINNED_ACCOUNT
        .try_with(|i| *i)
        .unwrap_or_else(|_| w.active_account().index);
    w.accounts()
        .into_iter()
        .find(|a| a.index == index)
        .ok_or_else(|| {
            ts!(
                "이 지갑에 없는 계정이에요",
                "That account doesn't exist in this wallet"
            )
            .to_string()
        })
}

/// **계정별로도** 분리하는 데이터 파일 이름 — 지금은 내역(history)만.
/// 체인 접미(chain_file)에 계정 접미를 덧붙인다: 계정 0 은 **기존 이름 그대로**(무손실),
/// 그 외는 `-a{n}`("history-a2.json", "history-8453-a2.json"). 한도 장부·신뢰 주소는 일부러
/// 계정 공용이다 — 계정을 하나 더 만드는 것으로 일일 한도가 두 배가 되면 가드레일이 아니다.
/// (이름 규칙은 budu-mcp 와 같은 함수 shared/policy.rs — 어긋나면 GUI 가 적은 내역을 AI 가 못 본다.)
pub(crate) fn account_file(stem: &str) -> String {
    account_file_name(&chain_file(stem), active_account_index())
}

/// `account_file` 의 이름 규칙만 떼어낸 순수 함수 (정산 반영이 계정을 지정해 쓴다). MCP 와 같은 함수.
pub(crate) use crate::policy::account_file_name;

/// 비번 + 솔트 + 명시적 Argon2id 파라미터로 32바이트 대칭키를 유도한다.
/// Zeroizing 반환 — 어느 경로로 드랍돼도(틀린 비번 early-return 포함) 메모리에서 0으로 지워진다.
fn derive_key(
    password: &str,
    salt: &[u8],
    m: u32,
    t: u32,
    p: u32,
) -> Result<Zeroizing<[u8; 32]>, String> {
    use argon2::{Algorithm, Params, Version};
    // 파일에 적힌 비용을 그대로 믿지 않는다 — `kdf_m` 이 터무니없이 크면 Argon2 가 그만큼을
    // 한 번에 할당하다 오류를 돌려주기 전에 프로세스가 죽는다. 우리가 쓰는 v2·v3 값보다 넉넉히 위에서 끊는다.
    if m > KDF_M_MAX || t > KDF_T_MAX || p > KDF_P_MAX {
        return Err(ts!(
            "지갑 파일의 암호 설정이 비정상적으로 커요. 파일이 손상됐을 수 있습니다 — 복구 문구로 복원하세요.",
            "The wallet file's key-derivation settings are abnormally large. The file may be damaged — restore from your recovery phrase."
        )
        .into());
    }
    let params = Params::new(m, t, p, Some(32)).map_err(|e| {
        tf!(
            "KDF 파라미터 오류: {e}",
            "Key-derivation parameters are invalid: {e}"
        )
    })?;
    let mut key = Zeroizing::new([0u8; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut *key)
        .map_err(|e| tf!("키 유도 실패: {e}", "Couldn't derive the key: {e}"))?;
    Ok(key)
}

/// 니모닉을 비번으로 암호화해서 저장용 구조체로 만든다 (파라미터 지정 — 테스트가 v2 재현용으로도 사용).
fn encrypt_with(
    phrase: &str,
    address: &str,
    password: &str,
    version: u32,
    m: u32,
    t: u32,
    p: u32,
) -> Result<EncryptedWallet, String> {
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);

    let key = derive_key(password, &salt, m, t, p)?;
    let cipher = Aes256Gcm::new_from_slice(&*key)
        .map_err(|e| tf!("암호기 생성 실패: {e}", "Couldn't set up encryption: {e}"))?;
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher
        .encrypt(nonce, phrase.as_bytes())
        .map_err(|e| tf!("암호화 실패: {e}", "Encryption failed: {e}"))?;

    Ok(EncryptedWallet {
        version,
        address: address.to_string(),
        salt: B64.encode(salt),
        nonce: B64.encode(nonce_bytes),
        ciphertext: B64.encode(ciphertext),
        backed_up: false,
        kdf_m: m,
        kdf_t: t,
        kdf_p: p,
        accounts: Vec::new(),
        active: 0,
    })
}

/// 니모닉을 비번으로 암호화한다 — 항상 최신(v3, 강화 KDF)으로 쓴다.
fn encrypt_wallet(phrase: &str, address: &str, password: &str) -> Result<EncryptedWallet, String> {
    encrypt_with(phrase, address, password, 3, KDF_M_V3, KDF_T_V3, KDF_P_V3)
}

/// 암호화된 지갑에서 니모닉을 복호화한다. 비번이 틀리면 GCM 인증 실패 → 에러.
/// Zeroizing 반환 — 니모닉은 키 그 자체이므로 모든 호출 경로에서 드랍 시 메모리를 0으로 지운다.
pub(crate) fn decrypt_wallet(
    w: &EncryptedWallet,
    password: &str,
) -> Result<Zeroizing<String>, String> {
    let salt = B64
        .decode(&w.salt)
        .map_err(|e| tf!("솔트 디코드 실패: {e}", "Couldn't decode the salt: {e}"))?;
    let nonce_bytes = B64
        .decode(&w.nonce)
        .map_err(|e| tf!("논스 디코드 실패: {e}", "Couldn't decode the nonce: {e}"))?;
    let ciphertext = B64.decode(&w.ciphertext).map_err(|e| {
        tf!(
            "암호문 디코드 실패: {e}",
            "Couldn't decode the ciphertext: {e}"
        )
    })?;

    // 파일에 기록된 파라미터로 키 유도 — v2(필드 없음)는 serde default = 옛 기본값.
    let key = derive_key(password, &salt, w.kdf_m, w.kdf_t, w.kdf_p)?;
    let cipher = Aes256Gcm::new_from_slice(&*key)
        .map_err(|e| tf!("암호기 생성 실패: {e}", "Couldn't set up encryption: {e}"))?;
    // 🔴 `Nonce::from_slice` 는 길이가 12 가 아니면 **패닉**한다 — JSON 모양만 맞는 깨진 wallet.enc
    // (`"nonce": ""` 등)가 잠금 해제·백업·결제에서 앱을 죽였다. 길이를 먼저 본다.
    if nonce_bytes.len() != 12 {
        return Err(ts!(
            "지갑 파일이 손상됐어요(논스 길이).",
            "The wallet file is damaged (nonce length)."
        )
        .into());
    }
    let nonce = Nonce::from_slice(&nonce_bytes);
    let plaintext = cipher.decrypt(nonce, ciphertext.as_ref()).map_err(|_| {
        ts!("비밀번호가 올바르지 않습니다", "That password isn't right").to_string()
    })?;

    String::from_utf8(plaintext)
        .map(Zeroizing::new)
        .map_err(|e| {
            tf!(
                "복호화 데이터 손상: {e}",
                "The decrypted data is damaged: {e}"
            )
        })
}

/// 🔴 **wallet.enc 의 읽기-수정-쓰기를 한 줄로 세운다**. 계정 추가·전환·이름·백업 표시·
/// KDF 업그레이드가 각자 파일을 통째로 읽고 고쳐 통째로 쓴다 — 특히 업그레이드는 송금 스레드에서 1초 안팎의 KDF 를
/// 돈 뒤에 쓰므로, 그 사이 화면에서 한 계정 추가·전환을 옛 사본으로 덮어 계정이 앱에서 사라졌다.
/// wallet.enc 를 쓰는 건 이 프로세스(GUI)뿐이라 프로세스 안 잠금이면 된다(MCP 는 읽기만).
/// 잠금 안에서 `unlock_signer` 를 부르지 말 것 — 업그레이드가 같은 잠금을 잡는다(재진입 없음).
static WALLET_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn wallet_guard() -> std::sync::MutexGuard<'static, ()> {
    WALLET_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// 암호화된 지갑을 디스크에 저장 (소유자만 읽기/쓰기, 원자 교체 —
/// mark_backed_up 등 재작성 중 크래시로 지갑 파일이 깨지면 키 유실이라 치명적).
/// wallet.enc 는 키 그 자체라 **디스크까지 내린다** — 예전엔 rename 만 해서, 전원이 나가면 새 지갑·
/// 새 계정·KDF 업그레이드가 빈 파일로 돌아갈 수 있었다. 디렉터리 동기화만 실패한 경우(`NotDurable`)는 새 내용이 이미
/// 제자리라 성공으로 친다 — 실패로 돌리면 「이미 지갑이 있다」와 엉킨다. 그 구분이 필요한 곳(평문 원본 삭제)은 `_strict`.
///
/// 디렉터리 동기화만 실패했으면 한 번 더 해 본다 — 그래도 안 되면 성공으로 두고 남긴다. 파일 자체는 동기화했고
/// APFS 에서 디렉터리 fsync 실패는 사실상 안 생긴다. 실패로 돌리면 새 지갑이 「만들었는데 실패」가 돼 다시 만들 수도 없다.
fn write_encrypted(w: &EncryptedWallet) -> Result<(), String> {
    match write_encrypted_strict(w) {
        Ok(()) => Ok(()),
        Err(crate::store::DurableError::NotDurable(e)) => {
            let again = enc_path()
                .ok()
                .and_then(|p| p.parent().map(|d| d.to_path_buf()))
                .map(|d| fs::File::open(d).and_then(|f| f.sync_all()).is_ok())
                .unwrap_or(false);
            if !again {
                eprintln!("[wallet] wallet.enc 디렉터리 동기화 실패(파일은 제자리): {e}");
            }
            Ok(())
        }
        Err(crate::store::DurableError::NotWritten(e)) => Err(e),
    }
}

fn write_encrypted_strict(w: &EncryptedWallet) -> Result<(), crate::store::DurableError> {
    write_wallet_file(w, true)
}

/// **새** 지갑을 내건다 — 이미 wallet.enc 가 있으면 실패한다. 만들기·가져오기·평문 이전만 쓴다:
/// 「없나 보고 쓰기」 사이에 다른 앱이 먼저 만들었으면 그 시드를 덮지 않는다. 실패 구분은 `write_encrypted_strict` 와 같다.
fn create_encrypted_strict(w: &EncryptedWallet) -> Result<(), crate::store::DurableError> {
    write_wallet_file(w, false)
}

/// 새 지갑을 내건다 — 디렉터리 동기화만 실패한 경우는 `write_encrypted` 처럼 성공으로 친다(파일은 제자리).
fn create_encrypted(w: &EncryptedWallet) -> Result<(), String> {
    match create_encrypted_strict(w) {
        Ok(()) | Err(crate::store::DurableError::NotDurable(_)) => Ok(()),
        Err(crate::store::DurableError::NotWritten(e)) => Err(e),
    }
}

fn write_wallet_file(w: &EncryptedWallet, replace: bool) -> Result<(), crate::store::DurableError> {
    let json = serde_json::to_string_pretty(w).map_err(|e| {
        crate::store::DurableError::NotWritten(tf!(
            "직렬화 실패: {e}",
            "Couldn't serialize the wallet: {e}"
        ))
    })?;
    let path = enc_path().map_err(crate::store::DurableError::NotWritten)?;
    if replace {
        crate::store::write_atomic_durable(&path, json.as_bytes())
    } else {
        crate::store::create_durable(&path, json.as_bytes())
    }
}

pub(crate) fn read_encrypted() -> Result<EncryptedWallet, String> {
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

/// v2(옛 KDF) 지갑을 강화 파라미터(v3)로 재암호화한 사본을 만든다. 이미 v3면 None.
/// 순수 함수 — 디스크에 안 쓴다(쓰기는 maybe_upgrade_kdf). backed_up·계정 목록은 보존.
fn upgraded_wallet(w: &EncryptedWallet, phrase: &str, password: &str) -> Option<EncryptedWallet> {
    if w.version >= 3 {
        return None;
    }
    let mut nw = encrypt_wallet(phrase, &w.address, password).ok()?;
    nw.backed_up = w.backed_up;
    nw.accounts = w.accounts.clone();
    nw.active = w.active;
    Some(nw)
}

/// 비번 잠금 해제 성공 시 옛 KDF(v2) 파일을 v3로 재암호화한다 (lazy 업그레이드).
/// 실패해도 조용히 넘어간다 — 업그레이드는 부가 기능, 잠금 해제를 막으면 안 된다.
/// (write_encrypted = 원자 교체라 도중 크래시에도 기존 파일이 깨지지 않는다.)
///
/// 재암호화(KDF)는 잠금 밖에서 하고, 쓰기 직전에 잠금 안에서 **최신 파일을 다시 읽는다**:
/// 그새 바뀐 계정 목록·활성·백업 표시를 싣고, 암호문이 달라졌으면(다른 시드를 가져왔다·이미 업그레이드됐다) 쓰지 않는다.
pub(crate) fn maybe_upgrade_kdf(w: &EncryptedWallet, phrase: &str, password: &str) {
    let Some(nw) = upgraded_wallet(w, phrase, password) else {
        return;
    };
    let _g = wallet_guard();
    let Ok(latest) = read_encrypted() else {
        return;
    };
    if let Some(nw) = carry_metadata(nw, w, &latest) {
        let _ = write_encrypted(&nw);
    }
}

/// 업그레이드 사본에 최신 파일의 메타데이터를 싣는다 — 최신 파일이 업그레이드를 시작한 그 지갑(같은 암호문, 아직 v2)일
/// 때만. 순수 함수(테스트용).
fn carry_metadata(
    mut nw: EncryptedWallet,
    started: &EncryptedWallet,
    latest: &EncryptedWallet,
) -> Option<EncryptedWallet> {
    if latest.version >= 3 || latest.ciphertext != started.ciphertext {
        return None;
    }
    nw.backed_up = latest.backed_up;
    nw.accounts = latest.accounts.clone();
    nw.active = latest.active;
    // 주소도 최신 것을 — 같은 암호문(같은 시드)이라 맞는 주소는 하나인데, 그새 `repair_legacy_addresses` 가
    // `0x…` 를 고쳐 썼으면 업그레이드 사본(옛 `w.address` 로 만듦)이 그걸 되돌려 놓는다.
    nw.address = latest.address.clone();
    Some(nw)
}

/// 평문 주소가 솔라나 꼴이 아닌가 — EVM 시절 파일의 `0x…`(base58 에 `0` 이 없다)·빈 칸.
/// 솔라나 꼴인데 키와 안 맞는 주소는 **여기 해당하지 않는다** — 그건 변조일 수 있어 `signer_matching_file` 이 계속 막는다.
fn is_legacy_address(a: &str) -> bool {
    crate::solana::parse_pubkey(a).is_none()
}

/// 지갑 파일이 옛 형식인가 — 비번 없이(평문 주소만). 못 읽으면 false(다른 검사들이 그 경우를 따로 막는다).
pub(crate) fn file_has_legacy_address() -> bool {
    enc_path().is_ok_and(|p| p.exists()) && read_encrypted().is_ok_and(|w| has_legacy_address(&w))
}

fn has_legacy_address(w: &EncryptedWallet) -> bool {
    w.accounts().iter().any(|a| is_legacy_address(&a.address))
        || w.accounts.iter().any(|a| is_legacy_address(&a.address))
}

/// 솔라나 꼴이 아닌 평문 주소를 같은 니모닉에서 다시 계산한 사본 (순수). 고칠 게 없으면 None.
/// 암호문·백업 표시·라벨·활성은 그대로 — 바뀌는 건 공개 정보(주소)뿐이다.
fn repaired_addresses(w: &EncryptedWallet, phrase: &str) -> Result<Option<EncryptedWallet>, String> {
    if !has_legacy_address(w) {
        return Ok(None);
    }
    let mut nw = w.clone();
    if is_legacy_address(&nw.address) {
        nw.address = signer_for_account(phrase, 0)?.address();
    }
    for a in nw.accounts.iter_mut() {
        if is_legacy_address(&a.address) {
            a.address = signer_for_account(phrase, a.index)?.address();
        }
    }
    Ok(Some(nw))
}

/// 비번을 받은 길(결제 승인·자율결제 잠금 해제·복구 문구 보기·「옛 형식」 칸)에서 `0x…` 주소를 고쳐 쓴다.
/// 개발 2 의 솔라나 전환이 기존 파일을 옮기지 않아, 개발 1 에 만든 지갑은 잔액 조회부터 막혔다.
/// 쓰기 직전에 잠금 안에서 최신 파일을 다시 읽고, 암호문이 달라졌으면(그새 다른 시드를 가져왔다) 손대지 않는다.
/// 돌려주는 값 = 이후 대조에 쓸 파일 내용(고쳤으면 고친 것).
pub(crate) fn repair_legacy_addresses(
    w: &EncryptedWallet,
    phrase: &str,
) -> Result<EncryptedWallet, String> {
    if !has_legacy_address(w) {
        return Ok(w.clone());
    }
    let _g = wallet_guard();
    let latest = read_encrypted()?;
    if latest.ciphertext != w.ciphertext {
        // 그새 다른 길(KDF 업그레이드는 salt·nonce 가 새로 생겨 암호문이 바뀐다)이 이미 고쳤으면 그 최신 파일로 대조하게 —
        // 옛 사본을 돌려주면 「주소와 키가 맞지 않아요」 로 한 번 실패했다. 시드가 달라졌으면 대조가 막는다.
        // v3 일 때만 — v2 면 이어지는 `maybe_upgrade_kdf` 가 이 최신 파일을 옛 시드로 재암호화할 수 있다.
        return Ok(if has_legacy_address(&latest) || latest.version < 3 { w.clone() } else { latest });
    }
    match repaired_addresses(&latest, phrase)? {
        Some(nw) => {
            write_encrypted(&nw)?;
            Ok(nw)
        }
        None => Ok(latest),
    }
}

/// 비번으로 저장된 키를 복호화해 **활성 계정**(작업이 고정했으면 그 계정)의 서명자를 만든다
/// (송금·서명 공용). 비번이 틀리면 복호화 단계에서 거부된다. 성공 시 옛 KDF 파일은 v3로 업그레이드.
#[cfg(test)]
pub(crate) fn unlock_signer(password: &str) -> Result<SolKeypair, String> {
    unlock_signer_keep(password).map(|(s, _)| s)
}

/// `unlock_signer` + 복호화한 12단어를 돌려준다 — 사람이 승인 창에서 작업 예산을 열면 그 키를 작업이 끝날 때까지
/// **메모리에만** 든다(`job.rs`). 비번을 두 번 풀지 않으려고 같은 복호화에서 꺼낸다.
pub(crate) fn unlock_signer_keep(password: &str) -> Result<(SolKeypair, Zeroizing<String>), String> {
    let w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, password)?;
    // 고치기가 실패해도(디스크·손상된 다른 계정 칸) 결제 판단은 대조에 맡긴다 — 쓰는 계정이 멀쩡하면 나간다.
    let w = repair_legacy_addresses(&w, &phrase).unwrap_or(w);
    maybe_upgrade_kdf(&w, &phrase, password);
    let signer = signer_matching_file(&w, &phrase)?;
    Ok((signer, phrase))
}

/// 활성 계정(고정값 우선)의 서명자 — **파일에 적힌 그 계정의 주소와 파생 주소가 같을 때만**.
/// 받기 QR·잔액·MCP 상태·요청의 `from` 각인은 전부 파일의 평문 주소를 쓰고, 서명 키는 니모닉에서 다시 파생한다.
/// 둘이 갈리면(EVM 시절 파일·다른 빌드가 쓴 파일·평문 칸 변조) 사용자는 A 로 입금하는데 서명은 B 키로 나간다 → 닫는다.
fn signer_matching_file(w: &EncryptedWallet, phrase: &str) -> Result<SolKeypair, String> {
    let index = PINNED_ACCOUNT
        .try_with(|i| *i)
        .unwrap_or_else(|_| w.active_account().index);
    let signer = signer_for_account(phrase, index)?;
    let listed = w.accounts().into_iter().find(|a| a.index == index);
    match listed {
        Some(a) if a.address.trim() == signer.address() => Ok(signer),
        _ => Err(ts!(
            "지갑 파일의 주소와 키가 맞지 않아요. 아무것도 서명하지 않았습니다.",
            "The wallet file's address doesn't match its key. Nothing was signed."
        )
        .into()),
    }
}

/// 세션(메모리의 니모닉)용 — 지갑 파일을 읽어 `signer_matching_file` 과 같은 대조를 한다.
pub(crate) fn session_signer_for(phrase: &str) -> Result<SolKeypair, String> {
    signer_matching_file(&read_encrypted()?, phrase)
}

/// 상태 응답을 만든다 — 주소는 활성 계정의 것.
fn status_of(w: &EncryptedWallet) -> WalletStatus {
    let active = w.active_account();
    WalletStatus {
        state: "encrypted".into(),
        address: Some(active.address),
        backed_up: w.backed_up,
        accounts: w.accounts(),
        active: active.index,
        legacy_address: has_legacy_address(w),
    }
}

/// 지갑 파일 상태를 알려준다. 비번 없이 호출 가능 (주소는 공개정보).
#[tauri::command]
pub(crate) fn get_wallet_status() -> Result<WalletStatus, String> {
    if enc_path()?.exists() {
        let w = read_encrypted()?;
        return Ok(status_of(&w));
    }
    if legacy_path()?.exists() {
        let data = fs::read_to_string(legacy_path()?).map_err(|e| {
            tf!(
                "지갑 파일 읽기 실패: {e}",
                "Couldn't read the wallet file: {e}"
            )
        })?;
        let lw: LegacyWallet = serde_json::from_str(&data).map_err(|e| {
            tf!(
                "지갑 파일 파싱 실패: {e}",
                "Couldn't parse the wallet file: {e}"
            )
        })?;
        return Ok(WalletStatus {
            state: "legacy".into(),
            address: Some(lw.address),
            backed_up: false,
            accounts: Vec::new(),
            active: 0,
            legacy_address: false,
        });
    }
    Ok(WalletStatus {
        state: "none".into(),
        address: None,
        backed_up: false,
        accounts: Vec::new(),
        active: 0,
        legacy_address: false,
    })
}

/// 승인 대기 중인 AI 결제가 있으면 계정을 못 바꾼다. 요청은 그 순간의 계정으로
/// 각인돼 있어(MCP 가 `from` 을 적는다) 바꾼 뒤 승인하면 어차피 거절된다 — 거절될 전환을
/// 허용해 「승인 창엔 계정 1, 화면엔 계정 2」를 잠깐이라도 만들 이유가 없다. 먼저 처리(또는
/// 거부)하고 바꾸면 된다. 만료된 요청은 막지 않는다(live_request).
///
/// 예외 = **요청이 각인한 바로 그 계정으로 가는 전환**. 이 검사와 파일
/// 쓰기 사이에 MCP 가 요청을 만들 수 있다(프로세스가 달라 잠글 수 없다) — 그러면 요청은 옛
/// 계정으로 각인된 채 활성은 새 계정이라 승인마다 거절되고, 전환도 막혀 거부밖에 못 한다.
/// 각인 계정으로 돌아가는 건 그 어긋남을 **푸는** 전환이라 허용한다(그러면 승인이 된다).
/// 각인 없는 옛 요청의 계정은 0 이다(ipc::request_account_index).
fn ensure_no_pending_payment(target: Option<u32>) -> Result<(), String> {
    if let Some(req) = crate::ipc::live_request() {
        if target != Some(crate::ipc::request_account_index(&req)) {
            return Err(ts!(
                "승인 대기 중인 결제가 있어요. 먼저 처리한 뒤 계정을 바꾸세요.",
                "A payment is waiting for approval. Handle it first, then switch accounts."
            )
            .into());
        }
    }
    Ok(())
}

/// 다음 계정을 파생한다 — 비번이 필요하다(주소를 얻으려면 시드를 풀어야 한다).
/// 인덱스는 「있는 것 중 최대 + 1」: 계정은 지우지 않으므로 빈 자리가 생기지 않고, 같은 시드를
/// 다른 지갑에 넣었을 때의 순서와도 같다. 새 계정이 곧 활성 계정이 된다(메타마스크와 같다).
#[tauri::command]
pub(crate) fn add_account(password: String) -> Result<WalletStatus, String> {
    let password = Zeroizing::new(password);
    let _g = wallet_guard();
    ensure_no_pending_payment(None)?; // 새 계정은 어떤 요청도 각인했을 수 없다
    let mut w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, &password)?;
    // 잠금을 이미 쥐었다 — `repair_legacy_addresses` 대신 순수 함수로 고쳐 같은 쓰기에 싣는다.
    if let Some(nw) = repaired_addresses(&w, &phrase)? {
        w = nw;
    }
    let list = w.accounts();
    let next = list.iter().map(|a| a.index).max().unwrap_or(0) + 1;
    let address = signer_for_account(&phrase, next)?.address();
    // 파일의 accounts 엔 0 이 없을 수 있다(옛 파일) — 정규화된 목록 위에 얹어 통째로 적는다.
    let mut accounts = list;
    accounts.push(Account {
        index: next,
        address,
        label: String::new(),
    });
    w.accounts = accounts;
    // 비번 검증(KDF, 0.5초+) 동안 MCP 가 요청을 만들었을 수 있다. 그 요청은
    // 옛 계정으로 각인돼 있는데 여기서 활성을 새 계정으로 바꾸면 승인 창이 계정 칩을 덮어 사용자는
    // 거부밖에 못 한다. 활성을 바꾸기 직전에 한 번 더 본다 — 창을 좁힐 뿐 없애진 못한다(프로세스가
    // 달라 잠글 수 없다). 남은 틈은 승인 시 ensure_request_account 가 잡는다.
    ensure_no_pending_payment(None)?;
    w.active = next;
    write_encrypted(&w)?;
    Ok(status_of(&w))
}

/// 활성 계정을 바꾼다. 없는 인덱스는 거부.
#[tauri::command]
pub(crate) fn switch_account(index: u32) -> Result<WalletStatus, String> {
    let _g = wallet_guard();
    ensure_no_pending_payment(Some(index))?;
    let mut w = read_encrypted()?;
    if !w.accounts().iter().any(|a| a.index == index) {
        return Err(ts!(
            "이 지갑에 없는 계정이에요",
            "That account doesn't exist in this wallet"
        )
        .into());
    }
    if w.active != index {
        w.active = index;
        write_encrypted(&w)?;
    }
    Ok(status_of(&w))
}

/// 라벨을 다듬는다: 앞뒤 공백 제거, 제어 문자 제거, 최대 글자 수로 자른다 (순수 함수).
fn clean_label(label: &str) -> String {
    label
        .trim()
        .chars()
        .filter(|c| !c.is_control())
        .take(LABEL_MAX_CHARS)
        .collect::<String>()
        .trim_end()
        .to_string()
}

/// 계정 이름을 바꾼다 (비번 불필요 — 비밀이 아닌 메타데이터). 빈 값 = 기본 이름(「계정 N」)으로.
#[tauri::command]
pub(crate) fn rename_account(index: u32, label: String) -> Result<WalletStatus, String> {
    let _g = wallet_guard();
    let mut w = read_encrypted()?;
    let mut accounts = w.accounts();
    let Some(a) = accounts.iter_mut().find(|a| a.index == index) else {
        return Err(ts!(
            "이 지갑에 없는 계정이에요",
            "That account doesn't exist in this wallet"
        )
        .into());
    };
    a.label = clean_label(&label);
    w.accounts = accounts;
    write_encrypted(&w)?;
    Ok(status_of(&w))
}

/// 새 12단어 니모닉을 생성하고 비번으로 암호화 저장한다.
/// (비번 분실 시 복구 불가 — 시드 백업 흐름이 생성 직후 이어진다.)
#[tauri::command]
pub(crate) fn create_wallet(password: String) -> Result<WalletInfo, String> {
    let password = Zeroizing::new(password); // 사용 후 메모리에서 0으로 덮음
    let _g = wallet_guard();
    if enc_path()?.exists() {
        return Err(ts!("이미 지갑이 있습니다", "A wallet already exists").into());
    }
    let phrase = keys::generate_phrase();
    let address = derive_address(&phrase)?;

    let w = encrypt_wallet(&phrase, &address, &password)?;
    create_encrypted(&w)?;

    Ok(WalletInfo { address })
}

/// 기존 평문 wallet.json 을 비번으로 암호화한다 (최초 1회). 성공 시 평문 파일 삭제.
#[tauri::command]
pub(crate) fn migrate_wallet(password: String) -> Result<WalletInfo, String> {
    let password = Zeroizing::new(password);
    let _g = wallet_guard();
    if enc_path()?.exists() {
        return Err(ts!(
            "이미 암호화된 지갑이 있습니다",
            "An encrypted wallet already exists"
        )
        .into());
    }
    let legacy = legacy_path()?;
    let data = fs::read_to_string(&legacy).map_err(|e| {
        tf!(
            "기존 지갑 읽기 실패: {e}",
            "Couldn't read the existing wallet: {e}"
        )
    })?;
    let lw: LegacyWallet = serde_json::from_str(&data).map_err(|e| {
        tf!(
            "기존 지갑 파싱 실패: {e}",
            "Couldn't parse the existing wallet: {e}"
        )
    })?;
    let mnemonic = Zeroizing::new(lw.mnemonic);

    // 무결성 확인: 저장된 주소가 니모닉에서 실제 파생되는지.
    let derived = derive_address(&mnemonic)?;
    // base58 은 대소문자를 가린다 — 정확히 같아야 한다(EVM 시절의 대소문자 무시 비교를 쓰면 안 된다).
    if derived != lw.address.trim() {
        return Err(ts!(
            "기존 지갑 파일이 손상되었습니다 (주소 불일치)",
            "The existing wallet file is damaged (address mismatch)"
        )
        .into());
    }

    let w = encrypt_wallet(&mnemonic, &derived, &password)?;
    // 🔴 평문 원본은 유일한 사본일 수 있다 — 암호화본이 **디스크까지** 내려가고 다시 읽어 풀린 뒤에만 지운다.
    // 디렉터리 동기화가 실패했으면 원본을 남긴다(다음 실행은 암호화본을 쓰고, 평문은 사람이 지울 때까지 남는다 — 키를 잃는 것보다 낫다).
    create_encrypted_strict(&w).map_err(|e| match e {
        crate::store::DurableError::NotWritten(e) | crate::store::DurableError::NotDurable(e) => e,
    })?;
    let back = read_encrypted()?;
    let again = decrypt_wallet(&back, &password)?;
    if *again != *mnemonic {
        return Err(ts!(
            "암호화한 지갑을 다시 확인하지 못해 기존 파일을 남겼어요",
            "Couldn't verify the encrypted wallet, so the existing file was kept"
        )
        .into());
    }

    // 암호화·확인 성공 후에만 평문 파일 제거.
    fs::remove_file(&legacy).map_err(|e| {
        tf!(
            "기존 평문 파일 삭제 실패: {e}",
            "Couldn't delete the old unencrypted file: {e}"
        )
    })?;

    Ok(WalletInfo { address: derived })
}

/// 복구 문구 정규화: 앞뒤·중간 공백과 줄바꿈을 단일 공백으로 모으고 영문 소문자로 바꾼다.
/// (붙여넣을 때 섞여 들어오는 줄바꿈·중복 공백·대문자를 흡수한다. BIP-39 영문 단어는 전부
/// 소문자 ASCII라 소문자화는 유효한 문구를 절대 깨지 않는다 — 비-ASCII 입력은 그대로 두되
/// 어차피 파생 검증에서 거부된다.)
///
/// 결과를 처음부터 `Zeroizing<String>` 버퍼에 직접 쌓는다 — split/join/to_lowercase 가 만드는
/// 중간 String 사본(니모닉 평문이 든)이 제로화 안 된 채 드랍되는 걸 피하기 위함. 용량을 입력
/// 길이로 잡아두면(정규화 결과는 항상 더 짧다) 재할당이 없어 이 버퍼가 유일한 힙 사본이 된다.
fn normalize_mnemonic(phrase: &str) -> Zeroizing<String> {
    let mut out = Zeroizing::new(String::with_capacity(phrase.len()));
    for word in phrase.split_whitespace() {
        if !out.is_empty() {
            out.push(' ');
        }
        for c in word.chars() {
            out.push(c.to_ascii_lowercase());
        }
    }
    out
}

/// 가져온 복구 문구를 검증·정규화해 암호화 지갑(backed_up=true)으로 만든다.
/// 순수 함수(디스크 미접근) — 명령은 파일 존재 검사·쓰기만 감싼다. 테스트가 직접 호출한다.
/// 사용자가 12단어를 직접 입력했다는 건 이미 시드를 보유했다는 뜻 → 백업 플로우가 불필요하다.
fn build_imported_wallet(phrase: &str, password: &str) -> Result<EncryptedWallet, String> {
    let normalized = normalize_mnemonic(phrase);
    let count = normalized.split_whitespace().count();
    if !matches!(count, 12 | 15 | 18 | 21 | 24) {
        return Err(tf!(
            "복구 문구는 12·15·18·21·24단어 중 하나여야 해요 (지금 {count}개)",
            "A recovery phrase has 12, 15, 18, 21, or 24 words (this one has {count})"
        ));
    }
    // BIP-39 체크섬·단어 검증 + 주소 파생. 잘못된 단어·순서·체크섬이면 여기서 거부된다.
    // (원시 파생 에러는 내부 정보라 노출하지 않고 사람 말 메시지로 바꾼다.)
    let address = derive_address(&normalized).map_err(|_| {
        ts!(
            "복구 문구가 올바르지 않아요. 단어와 순서를 다시 확인해주세요.",
            "That recovery phrase isn't valid. Check the words and their order."
        )
        .to_string()
    })?;

    let mut w = encrypt_wallet(&normalized, &address, password)?;
    w.backed_up = true;
    Ok(w)
}

/// 기존 복구 문구(BIP-39 12~24단어)를 가져와 비번으로 암호화 저장한다.
/// 다른 지갑에서 옮겨오거나 비번을 잊어 재설정할 때 사용. wallet.enc 가 이미 있으면 거부한다
/// (덮어쓰면 기존 키가 영구 유실되므로 — 가져오려면 사용자가 명시적으로 정리해야 한다).
#[tauri::command]
pub(crate) fn import_wallet(password: String, phrase: String) -> Result<WalletInfo, String> {
    let password = Zeroizing::new(password); // 사용 후 메모리에서 0으로 덮음
    let phrase = Zeroizing::new(phrase);
    let _g = wallet_guard();
    if enc_path()?.exists() {
        return Err(ts!("이미 지갑이 있습니다", "A wallet already exists").into());
    }
    let w = build_imported_wallet(&phrase, &password)?;
    let address = w.address.clone();
    create_encrypted(&w)?;
    Ok(WalletInfo { address })
}

/// 비번으로 시드 니모닉(12단어)을 복호화해서 돌려준다. 백업 화면에서만 호출.
/// 비번이 틀리면 복호화 단계에서 거부된다. (자산 영구 손실 방지용 — 비번을 잊어도
/// 이 12단어만 있으면 다른 지갑에서 복구 가능하다.)
#[tauri::command]
pub(crate) fn reveal_mnemonic(password: String) -> Result<Vec<String>, String> {
    let password = Zeroizing::new(password);
    let w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, &password)?;
    // 백업이 먼저다 — 주소 고치기가 실패해도 12단어는 보여 준다.
    let _ = repair_legacy_addresses(&w, &phrase);
    Ok(phrase.split_whitespace().map(|s| s.to_string()).collect())
}

/// 「옛 형식 지갑」 화면의 비번 칸. 비번으로 니모닉을 풀어 `0x…` 주소를 솔라나 주소로 고쳐 쓰고 새 상태를 돌려준다.
/// 솔라나 꼴인데 키와 다른 주소는 고치지 않는다(`is_legacy_address`) — 그 경우 화면은 그대로 막힌 채 남는다.
#[tauri::command]
pub(crate) fn repair_wallet_address(password: String) -> Result<WalletStatus, String> {
    let password = Zeroizing::new(password);
    let w = read_encrypted()?;
    let phrase = decrypt_wallet(&w, &password)?;
    let w = repair_legacy_addresses(&w, &phrase)?;
    maybe_upgrade_kdf(&w, &phrase, &password);
    Ok(status_of(&w))
}

/// 사용자가 시드 백업을 마쳤다고 표시한다. 비번은 필요 없다(비밀이 아닌 메타데이터).
#[tauri::command]
pub(crate) fn mark_backed_up() -> Result<(), String> {
    let _g = wallet_guard();
    let mut w = read_encrypted()?;
    if !w.backed_up {
        w.backed_up = true;
        write_encrypted(&w)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
