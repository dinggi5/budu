# 부두 (Budu)

'부두'는 부산항 업무를 하는 AI 에이전트 전용 솔라나 블록체인 지갑입니다. 에이전트가 부산항 데이터를 '부두'의 x402를 통해 USDC를 지불하여 API 등 무역에 필요한 데이터를 호출할 수 있습니다.

- 데모 판매자: `budu-port/` 의 `npm run setup-seller` → `npm start` (데브넷 USDC).
- 검사: `npm run check` (tsc · clippy `-D warnings` · cargo test · budu-port).
- 전체 구조: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 부산 무역·블록체인 생태계 조사: [docs/RESEARCH-busan-trade.md](docs/RESEARCH-busan-trade.md)
