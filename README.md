# 부두 (Budu)

'부두'는 부산항 업무를 하는 AI 에이전트 전용 솔라나 블록체인 지갑입니다. 에이전트가 부산항 데이터를 '부두'의 x402를 통해 USDC를 지불하여 API 등 무역에 필요한 데이터를 호출할 수 있습니다.

## 설치 (macOS, Apple Silicon)

1. [Releases](https://github.com/dinggi5/budu/releases/latest) 에서 `Budu-1.0.0.dmg` 를 받습니다.
2. DMG 를 열고 `Budu.app` 을 **응용 프로그램** 폴더로 끌어 넣습니다. (AI 연결은 응용 프로그램 폴더에서 실행해야 등록됩니다.)
3. 처음 한 번은 Finder 에서 `Budu.app` 을 **우클릭 → 열기 → 열기** 합니다. 애플 공증을 받지 않은 앱이라 그냥 더블클릭하면 «확인되지 않은 개발자» 로 막힙니다.
   그래도 «손상되었기 때문에 열 수 없습니다» 가 뜨면 터미널에서 격리 표시를 지우고 다시 엽니다:

   ```bash
   xattr -dr com.apple.quarantine /Applications/Budu.app
   ```

4. 지갑을 만들고 시드 12단어를 종이에 적은 뒤, 왼쪽의 «AI 연결» 에서 Claude Code 나 Codex 를 연결합니다.

### 데브넷에서 해 보기 (돈 안 듦)

기본 네트워크는 **솔라나 데브넷**입니다 — 시험용 토큰이라 실제 돈이 들지 않습니다. 홈의 «받기» 로 주소를 복사해 아래에서 받습니다.

- 데브넷 USDC: [faucet.circle.com](https://faucet.circle.com) → *Solana Devnet* 을 고르고 주소를 붙여 넣기. 결제는 USDC 만 씁니다(SOL 필요 없음).
- 데브넷 SOL(선택, «SOL 보내기» 를 해 볼 때만): [faucet.solana.com](https://faucet.solana.com).

## 개발

- 데모 판매자: `budu-port/` 의 `npm run setup-seller` → `npm start` (데브넷 USDC).
- 검사: `npm run check` (tsc · clippy `-D warnings` · cargo test · budu-port).
- 전체 구조: [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)
- 부산 무역·블록체인 생태계 조사: [docs/RESEARCH-busan-trade.md](docs/RESEARCH-busan-trade.md)

## 라이선스

[MIT](LICENSE)
