# 부두(Budu) — 저장소 지침

@~/프로젝트/부두-notes/CLAUDE.md

- 이름은 부두 / Budu. 데이터 폴더 `~/.budu`, 번들 ID `com.dinggi5.budu`, MCP 이름 `budu`, 바이너리 `budu-mcp`.
- 결제 코어(키 파생·서명·잔액·정산)는 «승인 → 블록해시 → 조립 → 자체 검증 → 서명» 순서. 메시지·명령은 손으로 직렬화, RPC 는 reqwest JSON-RPC.
- 검사: `npm run check`. 구조는 `docs/ARCHITECTURE.md`.
