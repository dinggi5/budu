#!/usr/bin/env bash
# 저장소 전체 검사 한 줄 — `npm run check`.
#
#   디자인 규칙(글자 3단계 · 색 2가지) → 타입(tsc) → MCP 크레이트(clippy -D warnings · test) → 사이드카 → 앱 크레이트(clippy · test) → budu-port 단위 테스트
#
# 앱 크레이트는 Tauri 빌드 스크립트가 사이드카(`src-tauri/binaries/budu-mcp-<트리플>`)를 요구한다.
# 없으면 먼저 만든다 — 새 워크트리에서 첫 `cargo clippy` 가 «resource path doesn't exist» 로 죽는 걸 막는다.
# budu-port 의 node_modules 가 없으면 그 테스트는 건너뛴다(골든 바이트는 앱 크레이트 테스트가 확인한다).
set -euo pipefail
cd "$(dirname "$0")/.."

step() { printf '\n\033[1m▸ %s\033[0m\n' "$*"; }

step "디자인 규칙: 글자 크기 3단계 · 색 2가지 (PLAN §5)"
# 글자 크기는 text-sm(12) · text-md(14) · text-lg(18) · text-hero(잔액) 뿐 — 숫자를 직접 쓰지 않는다.
# 색은 회색 + --color-ok · --color-danger 뿐 — Tailwind 팔레트 색을 직접 쓰지 않는다.
bad_size="$(grep -rnE 'text-\[[0-9.]+(px|rem|em)\]|font-size' src --include='*.tsx' --include='*.ts' || true)"
bad_color="$(grep -rnE '\b(text|bg|border|ring|fill|stroke|from|via|to|outline|decoration|caret|divide|accent|shadow)-(red|orange|amber|yellow|lime|green|emerald|teal|cyan|sky|blue|indigo|violet|purple|fuchsia|pink|rose)-[0-9]' src || true)"
if [[ -n "$bad_size$bad_color" ]]; then
  [[ -n "$bad_size" ]] && printf '글자 크기를 직접 썼어요 — text-sm/md/lg 를 쓰세요:\n%s\n' "$bad_size"
  [[ -n "$bad_color" ]] && printf '팔레트 색을 직접 썼어요 — var(--color-ok) / var(--color-danger) / 회색을 쓰세요:\n%s\n' "$bad_color"
  exit 1
fi

step "tsc"
npx tsc --noEmit

step "budu-mcp: clippy · test"
(cd budu-mcp && cargo clippy --all-targets --quiet -- -D warnings && cargo test --quiet)

HOST="$(rustc -vV | awk '/^host:/{print $2}')"
if [[ ! -f "src-tauri/binaries/budu-mcp-$HOST" ]]; then
  step "사이드카 (없어서 만든다)"
  bash scripts/build-sidecars.sh
fi

step "src-tauri: clippy · test"
(cd src-tauri && cargo clippy --all-targets --quiet -- -D warnings && cargo test --quiet)

if [[ -d budu-port/node_modules ]]; then
  step "budu-port: test"
  (cd budu-port && node test-parse.mjs)
else
  step "budu-port: node_modules 없음 — 건너뜀 (cd budu-port && npm ci)"
fi

printf '\n\033[32m✓ 전부 통과\033[0m\n'
