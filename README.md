# 부두 (Budu)

'부두'는 부산항 업무를 하는 AI 에이전트 전용 솔라나 블록체인 지갑입니다. 에이전트가 부산항 데이터를 '부두'의 x402를 통해 USDC를 지불하여 API 등 무역에 필요한 데이터를 호출할 수 있습니다.

부산항 업무를 하는 AI 에이전트의 지출 지갑. 에이전트가 부산항 데이터를 x402(솔라나 USDC)로 건당 사면, 부두는 기본으로 결제마다
맥의 승인 창에서 사람이 비밀번호로 결정하게 하고(자율 결제를 켜면 작은 금액·화이트리스트 주소만 자동), 건당·하루 한도를 넘는 결제는 막는다. 키는 맥을 떠나지 않는다.
사람이 승인 창에서 «이 작업 전체를 N USDC 까지» 를 열면 같은 작업·이미 승인한 판매자·예산 안의 다음 결제는 창 없이 지나가고, 내역·영수증 CSV 는 작업별로 묶인다. Solana Busan 해커톤(Solana Venture 트랙) 출품작.

- 데모 판매자: `budu-port/` 의 `npm run setup-seller` → `npm start` (데브넷 USDC).
- 검사: `npm run check` (tsc · clippy `-D warnings` · cargo test · budu-port).

- **구조(처음 읽는다면 여기부터)**: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 부산 무역·블록체인 조사: [docs/RESEARCH-busan-trade.md](docs/RESEARCH-busan-trade.md)
