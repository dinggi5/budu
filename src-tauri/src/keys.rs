// 솔라나 키 파생 — BIP-39 12단어 → 시드 → SLIP-0010 ed25519 → 주소(base58).
//
// 경로 = m/44'/501'/{n}'/0' (전부 hardened). 팬텀·솔플레어·백팩의 기본값이라 같은 12단어를 그 지갑에 넣으면
// 같은 계정이 같은 순서로 나온다. SLIP-0010 ed25519 는 HMAC-SHA512 몇 줄이라 직접 짰고,
// **SLIP-0010 공식 테스트 벡터 + 알려진 팬텀 주소**로 고정한다(아래 테스트).
//
// 비밀이 지나가는 버퍼(시드·체인 코드·개인키)는 전부 Zeroizing 에 담는다. ed25519-dalek 의 SigningKey 는
// `zeroize` 기능으로 드랍 때 지워진다.

use ed25519_dalek::{Signer as _, SigningKey};
use hmac::{Hmac, Mac};
use sha2::Sha512;
use zeroize::Zeroizing;

use crate::solana;

type HmacSha512 = Hmac<Sha512>;

/// hardened 비트.
const HARDENED: u32 = 0x8000_0000;
/// BIP-44 coin type — 솔라나.
const COIN_SOLANA: u32 = 501;

/// 계정 하나의 키쌍. 주소 = 공개키 32바이트의 base58.
pub(crate) struct SolKeypair {
    signing: SigningKey,
}

impl SolKeypair {
    pub(crate) fn address(&self) -> String {
        solana::encode_pubkey(&self.signing.verifying_key().to_bytes())
    }

    /// 공개키 32바이트 (= 주소의 원래 바이트).
    pub(crate) fn public_key(&self) -> [u8; 32] {
        self.signing.verifying_key().to_bytes()
    }

    /// 메시지 바이트에 ed25519 서명 (64바이트) — x402 부분 서명(svm_x402)이 쓴다.
    pub(crate) fn sign(&self, message: &[u8]) -> [u8; 64] {
        self.signing.sign(message).to_bytes()
    }

    /// 방금 만든 서명이 이 키로 검증되는지 (서명 직후 자기 확인).
    pub(crate) fn verify(&self, message: &[u8], sig: &[u8; 64]) -> bool {
        use ed25519_dalek::Verifier as _;
        self.signing
            .verifying_key()
            .verify(message, &ed25519_dalek::Signature::from_bytes(sig))
            .is_ok()
    }

    /// 32바이트 비밀 시드로 키쌍 — 정답지 테스트(공식 클라이언트의 `createKeyPairSignerFromPrivateKeyBytes` 와 같은 입력).
    #[cfg(test)]
    pub(crate) fn from_secret_bytes(seed: &[u8; 32]) -> Self {
        SolKeypair { signing: SigningKey::from_bytes(seed) }
    }
}

/// 키 파생 실패 — 원시 오류는 사람에게 안 보인다(니모닉 단어가 섞일 수 있다). 부르는 쪽이 문구를 붙인다.
#[derive(Debug, PartialEq)]
pub(crate) enum KeyError {
    /// 단어·개수·체크섬이 BIP-39 가 아니다.
    InvalidPhrase,
    /// 계정 인덱스가 hardened 범위를 넘는다(2^31 이상).
    IndexTooLarge,
}

/// 새 12단어 (128비트 엔트로피, OS 난수).
pub(crate) fn generate_phrase() -> Zeroizing<String> {
    use aes_gcm::aead::rand_core::RngCore;
    let mut entropy = Zeroizing::new([0u8; 16]);
    aes_gcm::aead::OsRng.fill_bytes(&mut entropy[..]);
    let m = bip39::Mnemonic::from_entropy_in(bip39::Language::English, &entropy[..])
        .expect("16바이트 엔트로피는 언제나 12단어");
    Zeroizing::new(m.to_string())
}

/// 12~24단어가 BIP-39(영어 단어·체크섬)로 올바른가. (앱 경로는 파생 자체가 검증이라 테스트만 쓴다.)
#[cfg(test)]
pub(crate) fn validate_phrase(phrase: &str) -> Result<(), KeyError> {
    bip39::Mnemonic::parse_in_normalized(bip39::Language::English, phrase)
        .map(|_| ())
        .map_err(|_| KeyError::InvalidPhrase)
}

/// 니모닉 → 64바이트 시드 (BIP-39, 암호 문구 없음 — 팬텀과 같다).
fn seed_of(phrase: &str) -> Result<Zeroizing<[u8; 64]>, KeyError> {
    let m = bip39::Mnemonic::parse_in_normalized(bip39::Language::English, phrase)
        .map_err(|_| KeyError::InvalidPhrase)?;
    Ok(Zeroizing::new(m.to_seed_normalized("")))
}

/// HMAC-SHA512(key, data) → (왼쪽 32 = 키, 오른쪽 32 = 체인 코드).
fn hmac_split(key: &[u8], data: &[&[u8]]) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let mut mac = HmacSha512::new_from_slice(key).expect("HMAC 은 어떤 길이의 키도 받는다");
    for d in data {
        mac.update(d);
    }
    let out = Zeroizing::new(<[u8; 64]>::from(mac.finalize().into_bytes()));
    let mut k = Zeroizing::new([0u8; 32]);
    let mut c = Zeroizing::new([0u8; 32]);
    k.copy_from_slice(&out[..32]);
    c.copy_from_slice(&out[32..]);
    (k, c)
}

/// SLIP-0010 ed25519 파생. ed25519 는 hardened 자식만 있다 — `path` 의 각 값에 hardened 비트를 붙여 쓴다
/// (이미 붙어 있어도 같다). 반환 = (개인키 시드 32, 체인 코드 32).
fn slip10_ed25519(seed: &[u8], path: &[u32]) -> (Zeroizing<[u8; 32]>, Zeroizing<[u8; 32]>) {
    let (mut k, mut c) = hmac_split(b"ed25519 seed", &[seed]);
    for &i in path {
        let idx = (i | HARDENED).to_be_bytes();
        let (nk, nc) = hmac_split(&c[..], &[&[0u8], &k[..], &idx]);
        k = nk;
        c = nc;
    }
    (k, c)
}

/// 니모닉에서 계정 `index` 의 키쌍 — m/44'/501'/{index}'/0'.
pub(crate) fn keypair_for_account(phrase: &str, index: u32) -> Result<SolKeypair, KeyError> {
    if index >= HARDENED {
        return Err(KeyError::IndexTooLarge);
    }
    let seed = seed_of(phrase)?;
    let (k, _c) = slip10_ed25519(&seed[..], &[44, COIN_SOLANA, index, 0]);
    Ok(SolKeypair {
        signing: SigningKey::from_bytes(&k),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    // SLIP-0010 공식 테스트 벡터 1 (ed25519), seed = 000102…0f.
    // https://github.com/satoshilabs/slips/blob/master/slip-0010.md#test-vector-1-for-ed25519
    #[test]
    fn slip10_official_vector_1() {
        let seed: Vec<u8> = (0u8..16).collect();
        let (k, c) = slip10_ed25519(&seed, &[]);
        assert_eq!(
            hex(&c[..]),
            "90046a93de5380a72b5e45010748567d5ea02bbf6522f979e05c0d8d8ca9fffb"
        );
        assert_eq!(
            hex(&k[..]),
            "2b4be7f19ee27bbf30c667b642d5f4aa69fd169872f8fc3059c08ebae2eb19e7"
        );
        let pk = SigningKey::from_bytes(&k).verifying_key().to_bytes();
        assert_eq!(
            hex(&pk),
            "a4b2856bfec510abab89753fac1ac0e1112364e7d250545963f135f2a33188ed"
        );

        let (k, c) = slip10_ed25519(&seed, &[0]);
        assert_eq!(
            hex(&c[..]),
            "8b59aa11380b624e81507a27fedda59fea6d0b779a778918a2fd3590e16e9c69"
        );
        assert_eq!(
            hex(&k[..]),
            "68e0fe46dfb67e368c75379acec591dad19df3cde26e63b93a8e704f1dade7a3"
        );
        let pk = SigningKey::from_bytes(&k).verifying_key().to_bytes();
        assert_eq!(
            hex(&pk),
            "8c8a13df77a28f3445213a0f432fde644acaa215fc72dcdf300d5efaa85d350c"
        );

        // m/0'/1' — hardened 비트를 미리 붙여 넘겨도 같은 값.
        let (k1, _) = slip10_ed25519(&seed, &[0, 1]);
        let (k2, _) = slip10_ed25519(&seed, &[HARDENED, 1 | HARDENED]);
        assert_eq!(hex(&k1[..]), hex(&k2[..]));
        assert_eq!(
            hex(&k1[..]),
            "b1d0bad404bf35da785a64ca1ac54b2617211d2777696fbffaf208f746ae84f2"
        );
    }

    const ABANDON: &str = "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";

    // 🔴 팬텀 호환 — BIP-39 표준 테스트 문구의 m/44'/501'/0'/0' 주소.
    // 이 값이 바뀌면 같은 12단어가 팬텀과 **다른 지갑**이 된다(복구했는데 잔액이 0).
    #[test]
    fn phantom_compatible_first_account() {
        let kp = keypair_for_account(ABANDON, 0).unwrap();
        assert_eq!(kp.address(), "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk");
    }

    #[test]
    fn accounts_differ_by_index_and_are_on_curve() {
        let a0 = keypair_for_account(ABANDON, 0).unwrap().address();
        let a1 = keypair_for_account(ABANDON, 1).unwrap().address();
        assert_ne!(a0, a1);
        for a in [&a0, &a1] {
            let b = solana::parse_pubkey(a).unwrap();
            assert!(solana::is_on_curve(&b));
        }
        assert_eq!(
            keypair_for_account(ABANDON, HARDENED).err(),
            Some(KeyError::IndexTooLarge)
        );
    }

    #[test]
    fn phrase_validation() {
        assert!(validate_phrase(ABANDON).is_ok());
        // 체크섬 틀림 (마지막 단어)
        assert_eq!(
            validate_phrase(&ABANDON.replace("about", "abandon")),
            Err(KeyError::InvalidPhrase)
        );
        assert_eq!(validate_phrase("hello world"), Err(KeyError::InvalidPhrase));
        assert_eq!(
            keypair_for_account("hello world", 0).err(),
            Some(KeyError::InvalidPhrase)
        );
    }

    #[test]
    fn generated_phrase_is_valid_12_words_and_random() {
        let a = generate_phrase();
        let b = generate_phrase();
        assert_eq!(a.split_whitespace().count(), 12);
        assert!(validate_phrase(&a).is_ok());
        assert_ne!(*a, *b);
    }

    #[test]
    fn signature_verifies() {
        use ed25519_dalek::{Signature, Verifier, VerifyingKey};
        let kp = keypair_for_account(ABANDON, 0).unwrap();
        let sig = kp.sign(b"budu");
        let vk = VerifyingKey::from_bytes(&solana::parse_pubkey(&kp.address()).unwrap()).unwrap();
        assert!(vk.verify(b"budu", &Signature::from_bytes(&sig)).is_ok());
    }
}
