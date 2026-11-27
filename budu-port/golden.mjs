// 정답지 — x402 공식 클라이언트(@x402/svm 의 ExactSvmScheme)가 고정 입력으로 만든 트랜잭션 바이트를 뽑는다.
//
// 부두 지갑(src-tauri/src/svm_x402.rs)은 Anza 크레이트 없이 v0 메시지·명령을 손으로 직렬화한다.
// 손으로 짠 바이트가 레퍼런스 구현과 **한 바이트도 안 다른지**를 이 출력으로 고정한다.
// ed25519 서명은 결정적이라 서명까지 포함한 전체 tx 를 비교할 수 있다.
//
// 네트워크에 안 나간다: 민트 정보(decimals·토큰 프로그램)를 묻는 RPC 하나를 로컬 가짜 서버가 답하고,
// 블록해시는 `extra.recentBlockhash` 로 준다(공식 클라이언트는 그걸 그대로 쓴다).
//
// 실행: `npm run golden` → 표준 출력의 JSON 을 src-tauri/src/svm_x402_golden.json 에 그대로 붙인다.

import http from "node:http";
import { createKeyPairSignerFromPrivateKeyBytes } from "@solana/kit";
import { ExactSvmScheme } from "@x402/svm/exact/client";

const TOKEN_PROGRAM = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const USDC_DEVNET = "4zMMC9srt5Ri5X14GAgXhaHii3GnPAEERYPJgZJDncDU";
const DEVNET = "solana:EtWTRABZaYq6iMfeYKouRu166VU2xqa1";

// SPL Token 민트 계정 82바이트: mintAuthority(COption 4+32) · supply u64 · decimals u8 · isInitialized u8 · freeze(4+32).
function mintAccountBase64(decimals) {
  const b = Buffer.alloc(82);
  b[44] = decimals;
  b[45] = 1;
  return b.toString("base64");
}

const server = http.createServer((req, res) => {
  let body = "";
  req.on("data", (c) => (body += c));
  req.on("end", () => {
    const { id, method } = JSON.parse(body);
    if (method !== "getAccountInfo") {
      res.writeHead(500).end();
      return;
    }
    res.setHeader("content-type", "application/json");
    res.end(
      JSON.stringify({
        jsonrpc: "2.0",
        id,
        result: {
          context: { slot: 1 },
          value: {
            owner: TOKEN_PROGRAM,
            lamports: 1461600,
            executable: false,
            rentEpoch: 0,
            space: 82,
            data: [mintAccountBase64(6), "base64"],
          },
        },
      }),
    );
  });
});
await new Promise((r) => server.listen(0, "127.0.0.1", r));
const rpcUrl = `http://127.0.0.1:${server.address().port}`;

// 사례들 — 개인키 시드·받는 쪽·수수료 지불자·금액·memo·블록해시. memo 는 반드시 주어야 결정적이다
// (없으면 공식 클라이언트가 16바이트 난수를 넣는다 — 그 갈래는 Rust 쪽이 따로 검사한다).
const cases = [
  {
    name: "devnet_0.01_seller_memo",
    seed: 7,
    payTo: "HAgk14JpMQLgt6rVgv7cBQFJWFto5Dqxi472uT3DKpqk",
    feePayer: "CKPKJWNdJEqa81x7CkZ14BVPiY6y16Sxs7owznqtWYp5",
    amount: "10000",
    memo: "budu-golden-1",
    blockhash: "EZ3rST5dvHmbanh75jc4PuLfV96vp9fEYBVeNk4FfM1k",
  },
  {
    name: "devnet_large_amount_utf8_memo",
    seed: 42,
    payTo: "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4",
    feePayer: "2wKupLR9q6wXYppw8Gr2NvWxKBUqm4PPJKkQfoxHDBg4", // 판매자가 수수료도 내는 경우(feePayer = payTo)
    amount: "18446744073709551615", // u64 최대 — 금액 인코딩 끝값
    memo: "부산항 입항 준비 · invoice#42",
    blockhash: "11111111111111111111111111111111",
  },
];

const out = [];
for (const c of cases) {
  const signer = await createKeyPairSignerFromPrivateKeyBytes(new Uint8Array(32).fill(c.seed));
  const scheme = new ExactSvmScheme(signer, { rpcUrl });
  const req = {
    scheme: "exact",
    network: DEVNET,
    amount: c.amount,
    asset: USDC_DEVNET,
    payTo: c.payTo,
    maxTimeoutSeconds: 60,
    extra: { feePayer: c.feePayer, memo: c.memo, recentBlockhash: c.blockhash },
  };
  const { payload } = await scheme.createPaymentPayload(2, req);
  out.push({
    name: c.name,
    seed_byte: c.seed,
    owner: signer.address,
    pay_to: c.payTo,
    fee_payer: c.feePayer,
    mint: USDC_DEVNET,
    decimals: 6,
    amount: c.amount,
    memo: c.memo,
    blockhash: c.blockhash,
    transaction_base64: payload.transaction,
  });
}
server.close();
console.log(JSON.stringify({ generator: "@x402/svm 2.30.0 ExactSvmScheme", cases: out }, null, 2));
