// x402 솔라나 `exact` 결제 코어 — v0 메시지 조립 · 자체 검증 · 부분 서명.
//
// 순서 «승인 → 블록해시 → 조립 → 자체 검증 → 서명» 중 뒤 넷이 여기 있다(승인은 ipc/session, 정책은 x402.rs).
// Anza 크레이트를 쓰지 않고 손으로 직렬화한다.
// 손 직렬화가 맞는지는 **정답지**로 고정한다: x402 공식 클라이언트(`@x402/svm` ExactSvmScheme)가 같은 입력으로 만든
// 트랜잭션 바이트(`svm_x402_golden.json`, `budu-port/golden.mjs` 로 생성)와 서명까지 한 바이트도 다르지 않아야 한다.
//
// 만드는 트랜잭션 (Path 1 표준 지갑, 버전 0, 주소 조회 테이블 없음):
//   0  ComputeBudget SetComputeUnitLimit  [2, u32 LE]
//   1  ComputeBudget SetComputeUnitPrice  [3, u64 LE]  (마이크로램포트/CU)
//   2  SPL Token TransferChecked          [12, u64 LE 금액, u8 decimals]  계정: 내 ATA · 민트 · ATA(payTo) · 나(서명)
//   3  SPL Memo                           판매자 memo 또는 16바이트 난수 hex
// 서명자는 [수수료 지불자(페이실리테이터), 나] 정확히 둘. 0번 자리는 비워 두고(0 64바이트) 우리는 1번만 서명한다.

use std::cmp::Ordering;

use crate::keys::SolKeypair;
use crate::solana::{self, encode_pubkey, parse_pubkey};

pub(crate) const COMPUTE_BUDGET_PROGRAM: &str = "ComputeBudget111111111111111111111111111111";
pub(crate) const MEMO_PROGRAM: &str = "MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr";

/// 공식 클라이언트와 같은 값(정답지가 이 값으로 떠졌다). 2만 CU 면 TransferChecked + Memo 에 넉넉하다.
pub(crate) const CU_LIMIT: u32 = 20_000;
/// 1 마이크로램포트/CU — 페이실리테이터 상한(Path 1 기본 5,000,000)보다 한참 낮게. 수수료는 어차피 feePayer 가 낸다.
pub(crate) const CU_PRICE_MICROLAMPORTS: u64 = 1;
/// 레거시·v0 트랜잭션 크기 상한 (스펙 §1.5).
const MAX_TX_BYTES: usize = 1232;
/// `extra.memo` 상한 (스펙 — 256바이트).
pub(crate) const MAX_MEMO_BYTES: usize = 256;

const IX_SET_CU_LIMIT: u8 = 2;
const IX_SET_CU_PRICE: u8 = 3;
const IX_TRANSFER_CHECKED: u8 = 12;
const VERSION_0_PREFIX: u8 = 0x80;

/// 서명할 결제 한 건 — 전부 이미 해석된 바이트. 문자열 → 바이트 해석과 정책 대조는 부르는 쪽(x402.rs)이 한다.
#[derive(Clone, Debug)]
pub(crate) struct PayTerms {
    pub(crate) fee_payer: [u8; 32],
    pub(crate) owner: [u8; 32],
    pub(crate) mint: [u8; 32],
    pub(crate) pay_to: [u8; 32],
    pub(crate) amount: u64,
    pub(crate) decimals: u8,
    pub(crate) memo: Vec<u8>,
    pub(crate) blockhash: [u8; 32],
}

/// 16바이트 난수 → 32자 hex (공식 클라이언트와 같은 모양 — 스펙 «at least 16 bytes, hex-encoded»).
pub(crate) fn random_memo() -> Vec<u8> {
    use aes_gcm::aead::rand_core::RngCore;
    let mut b = [0u8; 16];
    aes_gcm::aead::OsRng.fill_bytes(&mut b);
    b.iter().map(|x| format!("{x:02x}")).collect::<String>().into_bytes()
}

// ── 인코딩 ───────────────────────────────────────────────────────────────────────────────

/// compact-u16 (shortvec): 7비트씩, 최상위 비트 = 다음 바이트 있음.
fn push_compact_u16(out: &mut Vec<u8>, mut n: usize) {
    debug_assert!(n <= u16::MAX as usize);
    loop {
        let mut b = (n & 0x7f) as u8;
        n >>= 7;
        if n == 0 {
            out.push(b);
            return;
        }
        b |= 0x80;
        out.push(b);
    }
}

/// compact-u16 읽기 → (값, 읽은 바이트 수). 3바이트 넘는 값·끝에서 잘린 값·비정규(불필요한 0 바이트) 인코딩은 None.
fn read_compact_u16(b: &[u8]) -> Option<(usize, usize)> {
    let mut val: usize = 0;
    for i in 0..3 {
        let byte = *b.get(i)?;
        val |= ((byte & 0x7f) as usize) << (7 * i);
        if byte & 0x80 == 0 {
            if i > 0 && byte == 0 {
                return None; // 비정규 인코딩
            }
            if val > u16::MAX as usize {
                return None;
            }
            return Some((val, i + 1));
        }
    }
    None
}

/// `@solana/kit` 의 주소 정렬(`Intl.Collator("en", {caseFirst:"lower", sensitivity:"variant"})`)을 base58 글자에 한해 재현한다.
/// 1차: 대소문자를 무시한 비교(숫자 < 글자), 같으면 3차: 처음 갈리는 자리에서 소문자가 앞.
/// 정렬은 트랜잭션의 **유효성**과는 무관하다(어느 순서든 런타임은 받는다) — 정답지와 바이트를 맞추려고 같은 순서를 쓴다.
fn addr_cmp(a: &str, b: &str) -> Ordering {
    let la = a.to_ascii_lowercase();
    let lb = b.to_ascii_lowercase();
    la.cmp(&lb).then_with(|| {
        let ua: Vec<bool> = a.chars().map(|c| c.is_ascii_uppercase()).collect();
        let ub: Vec<bool> = b.chars().map(|c| c.is_ascii_uppercase()).collect();
        ua.cmp(&ub)
    })
}

fn sort_keys(keys: &mut [[u8; 32]]) {
    keys.sort_by(|x, y| addr_cmp(&encode_pubkey(x), &encode_pubkey(y)));
}

fn ata(owner: &[u8; 32], mint: &[u8; 32]) -> Result<[u8; 32], String> {
    solana::associated_token_address(&encode_pubkey(owner), &encode_pubkey(mint))
        .and_then(|s| parse_pubkey(&s))
        .ok_or_else(|| "ATA 파생 실패".to_string())
}

fn program(id: &str) -> [u8; 32] {
    parse_pubkey(id).expect("상수 프로그램 id 는 언제나 32바이트")
}

// ── 조립 ─────────────────────────────────────────────────────────────────────────────────

/// v0 메시지 바이트를 만든다. 계정이 겹치면(나에게 결제·feePayer 가 나·민트 = 받는 쪽 등) **만들지 않는다** —
/// 공식 클라이언트는 겹친 계정을 합쳐 버리지만, 그건 자체 검증 규칙(«서명자 정확히 둘 · feePayer 는 어떤 명령에도 없음»)을
/// 흐리게 하고 데모 범위의 정상 결제엔 일어나지 않는다(닫힌 쪽).
pub(crate) fn build_message(t: &PayTerms) -> Result<Vec<u8>, String> {
    if t.amount == 0 {
        return Err("금액이 0".into());
    }
    if t.memo.is_empty() || t.memo.len() > MAX_MEMO_BYTES {
        return Err("memo 길이".into());
    }
    if std::str::from_utf8(&t.memo).is_err() {
        return Err("memo 가 UTF-8 이 아님".into());
    }
    let src = ata(&t.owner, &t.mint)?;
    let dst = ata(&t.pay_to, &t.mint)?;
    let cb = program(COMPUTE_BUDGET_PROGRAM);
    let tok = program(solana::TOKEN_PROGRAM_ID);
    let memo = program(MEMO_PROGRAM);

    let mut writable = [src, dst];
    sort_keys(&mut writable);
    let mut readonly = [t.mint, cb, tok, memo];
    sort_keys(&mut readonly);
    let mut keys: Vec<[u8; 32]> = vec![t.fee_payer, t.owner];
    keys.extend_from_slice(&writable);
    keys.extend_from_slice(&readonly);
    for i in 0..keys.len() {
        for j in i + 1..keys.len() {
            if keys[i] == keys[j] {
                return Err("트랜잭션 계정이 겹친다(자기 자신에게 결제·수수료 지불자 = 나 등)".into());
            }
        }
    }
    let idx = |k: &[u8; 32]| keys.iter().position(|x| x == k).expect("넣은 키") as u8;

    let mut m = vec![VERSION_0_PREFIX, 2, 1, 4];
    push_compact_u16(&mut m, keys.len());
    for k in &keys {
        m.extend_from_slice(k);
    }
    m.extend_from_slice(&t.blockhash);

    let mut ixs: Vec<(u8, Vec<u8>, Vec<u8>)> = Vec::with_capacity(4);
    let mut d = vec![IX_SET_CU_LIMIT];
    d.extend_from_slice(&CU_LIMIT.to_le_bytes());
    ixs.push((idx(&cb), vec![], d));
    let mut d = vec![IX_SET_CU_PRICE];
    d.extend_from_slice(&CU_PRICE_MICROLAMPORTS.to_le_bytes());
    ixs.push((idx(&cb), vec![], d));
    let mut d = vec![IX_TRANSFER_CHECKED];
    d.extend_from_slice(&t.amount.to_le_bytes());
    d.push(t.decimals);
    ixs.push((idx(&tok), vec![idx(&src), idx(&t.mint), idx(&dst), idx(&t.owner)], d));
    ixs.push((idx(&memo), vec![], t.memo.clone()));

    push_compact_u16(&mut m, ixs.len());
    for (p, accts, data) in ixs {
        m.push(p);
        push_compact_u16(&mut m, accts.len());
        m.extend_from_slice(&accts);
        push_compact_u16(&mut m, data.len());
        m.extend_from_slice(&data);
    }
    push_compact_u16(&mut m, 0); // 주소 조회 테이블 0개
    Ok(m)
}

// ── 풀기 · 자체 검증 ───────────────────────────────────────────────────────────────────────

struct Ix {
    program: usize,
    accounts: Vec<usize>,
    data: Vec<u8>,
}

struct Message {
    header: [u8; 3],
    keys: Vec<[u8; 32]>,
    blockhash: [u8; 32],
    ixs: Vec<Ix>,
}

struct Cursor<'a> {
    b: &'a [u8],
    at: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).ok_or("길이 넘침")?;
        let s = self.b.get(self.at..end).ok_or("메시지가 중간에 끝남")?;
        self.at = end;
        Ok(s)
    }
    fn byte(&mut self) -> Result<u8, String> {
        Ok(self.take(1)?[0])
    }
    fn compact(&mut self) -> Result<usize, String> {
        let (v, n) = read_compact_u16(&self.b[self.at.min(self.b.len())..]).ok_or("compact-u16 이상")?;
        self.at += n;
        Ok(v)
    }
    fn key(&mut self) -> Result<[u8; 32], String> {
        Ok(self.take(32)?.try_into().expect("32"))
    }
}

/// v0 메시지를 푼다. 빌더와 **독립된** 코드다 — 자체 검증이 빌더의 실수를 빌더 자신의 눈으로 보지 않게.
fn decode_message(b: &[u8]) -> Result<Message, String> {
    let mut c = Cursor { b, at: 0 };
    if c.byte()? != VERSION_0_PREFIX {
        return Err("버전 0 메시지가 아님".into());
    }
    let header = [c.byte()?, c.byte()?, c.byte()?];
    let n = c.compact()?;
    let mut keys = Vec::with_capacity(n);
    for _ in 0..n {
        keys.push(c.key()?);
    }
    let blockhash = c.key()?;
    let ni = c.compact()?;
    let mut ixs = Vec::with_capacity(ni);
    for _ in 0..ni {
        let program = c.byte()? as usize;
        let na = c.compact()?;
        let accounts = c.take(na)?.iter().map(|&x| x as usize).collect();
        let nd = c.compact()?;
        let data = c.take(nd)?.to_vec();
        ixs.push(Ix { program, accounts, data });
    }
    if c.compact()? != 0 {
        return Err("주소 조회 테이블이 있음".into());
    }
    if c.at != b.len() {
        return Err("메시지 뒤에 남는 바이트".into());
    }
    Ok(Message { header, keys, blockhash, ixs })
}

/// 🔴 **자체 검증** — 서명할 바이트가 사람이 승인한 조건(`t`)과 같은 결제인지, 서명 직전에 바이트를 다시 풀어 본다.
/// 원본 지갑이 EIP-712 필드를 승인 창과 대조하던 자리. 하나라도 어긋나면 서명하지 않는다.
pub(crate) fn verify_message(msg: &[u8], t: &PayTerms) -> Result<(), String> {
    let m = decode_message(msg)?;
    let [n_sig, ro_signed, ro_unsigned] = m.header.map(|x| x as usize);
    let n = m.keys.len();
    // 서명자 = [feePayer, 나] 정확히 둘. 나는 읽기 전용 서명자, feePayer 는 쓰기 서명자(수수료 인출).
    if n_sig != 2 || ro_signed != 1 {
        return Err("서명자 구성이 [수수료 지불자, 나] 가 아님".into());
    }
    if n != 8 || ro_unsigned != 4 {
        return Err("계정 수가 예상과 다름".into());
    }
    if m.keys[0] != t.fee_payer || m.keys[1] != t.owner {
        return Err("서명자 주소가 승인한 것과 다름".into());
    }
    for i in 0..n {
        for j in i + 1..n {
            if m.keys[i] == m.keys[j] {
                return Err("계정 중복".into());
            }
        }
    }
    let writable = |i: usize| (i < n_sig && i < n_sig - ro_signed) || (i >= n_sig && i < n - ro_unsigned);
    if m.blockhash != t.blockhash {
        return Err("블록해시가 다름".into());
    }
    if m.ixs.len() != 4 {
        return Err("명령이 4개가 아님".into());
    }
    let key = |i: usize| m.keys.get(i).copied().ok_or_else(|| "계정 색인 범위 밖".to_string());
    // feePayer(0번)는 어떤 명령의 계정·프로그램에도 나오면 안 된다(스펙 §2.1.1).
    for ix in &m.ixs {
        if ix.program == 0 || ix.accounts.contains(&0) {
            return Err("수수료 지불자가 명령에 들어 있음".into());
        }
    }
    let cb = program(COMPUTE_BUDGET_PROGRAM);
    // 0: SetComputeUnitLimit
    let ix = &m.ixs[0];
    if key(ix.program)? != cb || !ix.accounts.is_empty() || ix.data.len() != 5 || ix.data[0] != IX_SET_CU_LIMIT {
        return Err("1번 명령이 SetComputeUnitLimit 이 아님".into());
    }
    let limit = u32::from_le_bytes(ix.data[1..5].try_into().expect("4"));
    if limit != CU_LIMIT {
        return Err("CU 한도가 지갑이 정한 값과 다름".into());
    }
    // 1: SetComputeUnitPrice
    let ix = &m.ixs[1];
    if key(ix.program)? != cb || !ix.accounts.is_empty() || ix.data.len() != 9 || ix.data[0] != IX_SET_CU_PRICE {
        return Err("2번 명령이 SetComputeUnitPrice 가 아님".into());
    }
    if u64::from_le_bytes(ix.data[1..9].try_into().expect("8")) != CU_PRICE_MICROLAMPORTS {
        return Err("CU 가격이 지갑이 정한 값과 다름".into());
    }
    // 2: TransferChecked — 내 ATA → ATA(payTo), 권한자 = 나, 금액·decimals 일치, 기존 SPL Token 프로그램.
    let ix = &m.ixs[2];
    if key(ix.program)? != program(solana::TOKEN_PROGRAM_ID) {
        return Err("3번 명령이 SPL Token 프로그램이 아님".into());
    }
    if ix.data.len() != 10 || ix.data[0] != IX_TRANSFER_CHECKED {
        return Err("3번 명령이 TransferChecked 가 아님".into());
    }
    if u64::from_le_bytes(ix.data[1..9].try_into().expect("8")) != t.amount || ix.data[9] != t.decimals {
        return Err("송금 금액·소수 자릿수가 승인한 것과 다름".into());
    }
    if ix.accounts.len() != 4 {
        return Err("TransferChecked 계정 수".into());
    }
    let (s, mi, d, a) = (ix.accounts[0], ix.accounts[1], ix.accounts[2], ix.accounts[3]);
    if key(s)? != ata(&t.owner, &t.mint)? || !writable(s) {
        return Err("보내는 계정이 내 토큰 계정이 아님".into());
    }
    if key(mi)? != t.mint {
        return Err("민트가 다름".into());
    }
    if key(d)? != ata(&t.pay_to, &t.mint)? || !writable(d) {
        return Err("받는 계정이 판매자 토큰 계정이 아님".into());
    }
    if a != 1 {
        return Err("송금 권한자가 내가 아님".into());
    }
    // 3: Memo — 계정 없음, 데이터 = 정한 memo.
    let ix = &m.ixs[3];
    if key(ix.program)? != program(MEMO_PROGRAM) || !ix.accounts.is_empty() || ix.data != t.memo {
        return Err("4번 명령이 정한 Memo 가 아님".into());
    }
    Ok(())
}

// ── 서명 ─────────────────────────────────────────────────────────────────────────────────

/// 조립 → 자체 검증 → 서명 → 트랜잭션 바이트. 반환 = (트랜잭션 전체, 우리 서명 64바이트).
/// 서명자 키가 `t.owner` 와 다르면 서명하지 않는다(파일 주소 ↔ 파생 키 대조는 wallet 이 이미 했지만, 마지막 문에서 한 번 더).
pub(crate) fn sign_payment(kp: &SolKeypair, t: &PayTerms) -> Result<(Vec<u8>, [u8; 64]), String> {
    if kp.public_key() != t.owner {
        return Err("서명 키가 결제 계정과 다름".into());
    }
    let msg = build_message(t)?;
    verify_message(&msg, t)?;
    let sig = kp.sign(&msg);
    if !kp.verify(&msg, &sig) {
        return Err("서명 확인 실패".into());
    }
    let mut tx = Vec::with_capacity(1 + 128 + msg.len());
    push_compact_u16(&mut tx, 2);
    tx.extend_from_slice(&[0u8; 64]); // feePayer 자리 — 페이실리테이터가 채운다
    tx.extend_from_slice(&sig);
    tx.extend_from_slice(&msg);
    if tx.len() > MAX_TX_BYTES {
        return Err("트랜잭션이 1232바이트를 넘음(memo 가 너무 김)".into());
    }
    Ok((tx, sig))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine};
    use serde_json::Value;

    fn pk(s: &str) -> [u8; 32] {
        parse_pubkey(s).unwrap()
    }

    fn golden() -> Vec<Value> {
        let v: Value = serde_json::from_str(include_str!("svm_x402_golden.json")).unwrap();
        v["cases"].as_array().unwrap().clone()
    }

    fn terms_of(c: &Value, kp: &SolKeypair) -> PayTerms {
        PayTerms {
            fee_payer: pk(c["fee_payer"].as_str().unwrap()),
            owner: kp.public_key(),
            mint: pk(c["mint"].as_str().unwrap()),
            pay_to: pk(c["pay_to"].as_str().unwrap()),
            amount: c["amount"].as_str().unwrap().parse().unwrap(),
            decimals: c["decimals"].as_u64().unwrap() as u8,
            memo: c["memo"].as_str().unwrap().as_bytes().to_vec(),
            blockhash: pk(c["blockhash"].as_str().unwrap()),
        }
    }

    /// 🔴 정답지 — 공식 클라이언트(@x402/svm 2.30.0)가 같은 입력으로 만든 트랜잭션과 **서명까지 바이트 단위로 같다**.
    #[test]
    fn matches_official_client_byte_for_byte() {
        for c in golden() {
            let seed = [c["seed_byte"].as_u64().unwrap() as u8; 32];
            let kp = SolKeypair::from_secret_bytes(&seed);
            assert_eq!(kp.address(), c["owner"].as_str().unwrap(), "{}", c["name"]);
            let (tx, _) = sign_payment(&kp, &terms_of(&c, &kp)).unwrap();
            let want = B64.decode(c["transaction_base64"].as_str().unwrap()).unwrap();
            assert_eq!(tx, want, "{}", c["name"]);
        }
    }

    fn base_terms() -> (SolKeypair, PayTerms) {
        let kp = SolKeypair::from_secret_bytes(&[7u8; 32]);
        let t = terms_of(&golden()[0], &kp);
        (kp, t)
    }

    #[test]
    fn verify_rejects_any_tampering() {
        let (_, t) = base_terms();
        let msg = build_message(&t).unwrap();
        verify_message(&msg, &t).unwrap();
        // 한 바이트라도 바뀌면(블록해시 뒤쪽 명령 영역 전부) 통과하지 못한다 — 데이터·색인·길이 어느 것이든.
        let ix_start = 1 + 3 + 1 + 8 * 32 + 32;
        for i in ix_start..msg.len() {
            let mut bad = msg.clone();
            bad[i] ^= 0x01;
            assert!(verify_message(&bad, &t).is_err(), "바이트 {i} 변조가 통과했다");
        }
        // 조건이 다르면 같은 바이트도 실패
        let mut t2 = t.clone();
        t2.amount += 1;
        assert!(verify_message(&msg, &t2).is_err());
        let mut t3 = t.clone();
        t3.pay_to = pk("2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4");
        assert!(verify_message(&msg, &t3).is_err());
        let mut t4 = t.clone();
        t4.memo = b"other".to_vec();
        assert!(verify_message(&msg, &t4).is_err());
        let mut t5 = t.clone();
        t5.blockhash = [1u8; 32];
        assert!(verify_message(&msg, &t5).is_err());
        // 뒤에 바이트를 붙여도 실패
        let mut longer = msg.clone();
        longer.push(0);
        assert!(verify_message(&longer, &t).is_err());
    }

    #[test]
    fn refuses_overlapping_accounts_and_bad_inputs() {
        let (kp, t) = base_terms();
        let mut self_pay = t.clone();
        self_pay.pay_to = t.owner;
        assert!(build_message(&self_pay).is_err());
        let mut fp_is_me = t.clone();
        fp_is_me.fee_payer = t.owner;
        assert!(build_message(&fp_is_me).is_err());
        let mut zero = t.clone();
        zero.amount = 0;
        assert!(build_message(&zero).is_err());
        let mut long = t.clone();
        long.memo = vec![b'a'; MAX_MEMO_BYTES + 1];
        assert!(build_message(&long).is_err());
        let mut nonutf = t.clone();
        nonutf.memo = vec![0xff, 0xfe];
        assert!(build_message(&nonutf).is_err());
        // 다른 키로는 서명하지 않는다
        let other = SolKeypair::from_secret_bytes(&[8u8; 32]);
        assert!(sign_payment(&other, &t).is_err());
        // 최대 memo 도 크기 안에 든다
        let mut max = t.clone();
        max.memo = vec![b'a'; MAX_MEMO_BYTES];
        assert!(sign_payment(&kp, &max).is_ok());
    }

    #[test]
    fn random_memo_shape() {
        let a = random_memo();
        assert_eq!(a.len(), 32);
        assert!(a.iter().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase()));
        assert_ne!(a, random_memo());
    }

    #[test]
    fn compact_u16_roundtrip() {
        for n in [0usize, 1, 127, 128, 255, 16_383, 16_384, 65_535] {
            let mut v = vec![];
            push_compact_u16(&mut v, n);
            assert_eq!(read_compact_u16(&v), Some((n, v.len())), "{n}");
        }
        assert_eq!(read_compact_u16(&[0x80, 0x00]), None); // 비정규
        assert_eq!(read_compact_u16(&[0x80]), None); // 잘림
        assert_eq!(read_compact_u16(&[0xff, 0xff, 0xff]), None);
    }

    #[test]
    fn address_order_matches_collator_rules() {
        assert_eq!(addr_cmp("abc", "ABD"), Ordering::Less); // 1차는 대소문자 무시
        assert_eq!(addr_cmp("abc", "Abc"), Ordering::Less); // 같으면 소문자가 앞
        assert_eq!(addr_cmp("9zz", "Azz"), Ordering::Less); // 숫자 < 글자
        assert_eq!(addr_cmp("ab", "abc"), Ordering::Less);
    }
}
