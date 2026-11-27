// 데모 판매자 준비 (데브넷 전용, 한 번만) — 판매자 키를 만들고 그 USDC 토큰 계정(ATA)을 체인에 만든다.
//
// x402 구매자 지갑(부두)은 판매자 ATA 가 없으면 승인 창을 띄우기 전에 멈춘다. 그래서 판매자 쪽 ATA 를 미리 만든다.
// ATA 생성 수수료·렌트(~0.002 SOL)는 **공개 테스트 니모닉**(abandon ×11 about — BIP-39 표준 문구, 누구나 쓰는 데브넷 지갑)
// 의 첫 계정이 낸다. 메인넷에서는 절대 쓰지 않는다 — 이 스크립트는 데브넷 genesis 가 아니면 멈춘다.
//
// 판매자 키는 `.seller-devnet.json`(gitignore)에 남긴다 — 받은 데브넷 USDC 를 나중에 옮길 수 있게.
// 실행: `npm run setup-seller`  → 판매자 주소를 출력. 서버는 이 파일에서 payTo 를 읽는다.

import fs from "node:fs";
import crypto from "node:crypto";
import { hmac } from "@noble/hashes/hmac";
import { sha512 } from "@noble/hashes/sha512";
import { pbkdf2 } from "@noble/hashes/pbkdf2";
import {
  appendTransactionMessageInstructions,
  createKeyPairSignerFromPrivateKeyBytes,
  createSolanaRpc,
  createTransactionMessage,
  getBase64EncodedWireTransaction,
  pipe,
  setTransactionMessageFeePayerSigner,
  setTransactionMessageLifetimeUsingBlockhash,
  signTransactionMessageWithSigners,
} from "@solana/kit";
import {
  findAssociatedTokenPda,
  getCreateAssociatedTokenIdempotentInstruction,
  TOKEN_PROGRAM_ADDRESS,
} from "@solana-program/token";

const RPC = "https://api.devnet.solana.com";
const DEVNET_GENESIS = "EtWTRABZaYq6iMfeYKouRu166VU2xqa1wcaWoxPkrZBG";
const USDC_DEVNET = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
const TEST_MNEMONIC =
  "abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon abandon about";
const TEST_ADDRESS = "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk"; // 팬텀 첫 계정 (부두 keys.rs 테스트와 같은 값)
const SELLER_FILE = new URL("./.seller-devnet.json", import.meta.url);

// BIP-39 시드 → SLIP-0010 ed25519 m/44'/501'/0'/0' (부두 keys.rs 와 같은 경로).
function testKeyBytes() {
  const seed = pbkdf2(sha512, TEST_MNEMONIC.normalize("NFKD"), "mnemonic", { c: 2048, dkLen: 64 });
  let I = hmac(sha512, new TextEncoder().encode("ed25519 seed"), seed);
  let k = I.slice(0, 32);
  let c = I.slice(32);
  for (const i of [44, 501, 0, 0]) {
    const data = new Uint8Array(37);
    data.set(k, 1);
    new DataView(data.buffer).setUint32(33, (i | 0x80000000) >>> 0);
    I = hmac(sha512, c, data);
    k = I.slice(0, 32);
    c = I.slice(32);
  }
  return k;
}

const rpc = createSolanaRpc(RPC);
if ((await rpc.getGenesisHash().send()) !== DEVNET_GENESIS) throw new Error("데브넷이 아니다 — 멈춤");

const funder = await createKeyPairSignerFromPrivateKeyBytes(testKeyBytes());
if (funder.address !== TEST_ADDRESS) throw new Error(`테스트 키 파생이 다르다: ${funder.address}`);

let sellerSecret;
if (fs.existsSync(SELLER_FILE)) {
  sellerSecret = Uint8Array.from(JSON.parse(fs.readFileSync(SELLER_FILE, "utf8")).secret);
} else {
  sellerSecret = crypto.getRandomValues(new Uint8Array(32));
}
const seller = await createKeyPairSignerFromPrivateKeyBytes(sellerSecret);
fs.writeFileSync(
  SELLER_FILE,
  JSON.stringify({ cluster: "devnet", address: seller.address, secret: Array.from(sellerSecret) }, null, 2),
  { mode: 0o600 },
);

const [ata] = await findAssociatedTokenPda({ owner: seller.address, mint: USDC_DEVNET, tokenProgram: TOKEN_PROGRAM_ADDRESS });
const existing = await rpc.getAccountInfo(ata, { encoding: "base64" }).send();
if (existing.value) {
  console.log(JSON.stringify({ seller: seller.address, ata, created: false }));
  process.exit(0);
}

const { value: latest } = await rpc.getLatestBlockhash().send();
const ix = getCreateAssociatedTokenIdempotentInstruction({
  payer: funder,
  ata,
  owner: seller.address,
  mint: USDC_DEVNET,
});
const msg = pipe(
  createTransactionMessage({ version: 0 }),
  (m) => setTransactionMessageFeePayerSigner(funder, m),
  (m) => setTransactionMessageLifetimeUsingBlockhash(latest, m),
  (m) => appendTransactionMessageInstructions([ix], m),
);
const signed = await signTransactionMessageWithSigners(msg);
const sig = await rpc
  .sendTransaction(getBase64EncodedWireTransaction(signed), { encoding: "base64", preflightCommitment: "confirmed" })
  .send();
for (let i = 0; i < 30; i++) {
  const { value } = await rpc.getSignatureStatuses([sig]).send();
  const st = value[0];
  if (st?.err) throw new Error(`ATA 생성 실패: ${JSON.stringify(st.err)}`);
  if (st?.confirmationStatus === "confirmed" || st?.confirmationStatus === "finalized") break;
  await new Promise((r) => setTimeout(r, 1000));
}
console.log(JSON.stringify({ seller: seller.address, ata, created: true, tx: sig }));
