// 지금 그리는 창이 어느 쪽인가 — 메뉴바 팝오버(`main`) 또는 큰 창(`window`).
//
// 두 창은 같은 프론트를 띄운다. 갈리는 건 셋뿐이다:
//   ① 결제 승인 — **큰 창만** 한다(모달·자율 승인). 팝오버는 「승인하러 가기」 띠만.
//      백엔드의 `appwin::APPROVER` 와 같은 뜻이고, 어긋나면 테스트(appwin.rs approver_matches_frontend)가 깨진다.
//   ② 겉모양 — 팝오버는 투명 창 안의 둥근 판, 큰 창은 신호등 아래로 내용이 이어지는 보통 창.
//      CSS 는 `<html data-win="window">` 와 `win:` 변형(App.css)으로 가른다.
//   ③ 넓이 — 내역 표처럼 「넓을 때만」은 창 종류가 아니라 실제 폭(`useWide`)으로 가른다.
//      큰 창도 좁히면 팝오버처럼 한 줄 목록으로 돌아온다.

import { useEffect, useState } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

function label(): string {
  try {
    return getCurrentWindow().label;
  } catch {
    // 모르면 팝오버로 — 승인을 두 창이 같이 하는 것보다 아무도 안 하는 쪽이 낫다. ⚠️ 큰 창에서 이게 실패하면 그 창은
    // 승인을 안 하는데 백엔드는 진짜 라벨로 「승인 창이 살아 있다」고 보므로 감시 스레드가 깨우지 않는다(5분 침묵).
    // 라벨 조회가 실패하는 경로는 알려진 게 없어 그대로 둔다.
    return "main";
  }
}

/** 큰 창인가. 창의 라벨은 평생 안 바뀌므로 모듈 로드 때 한 번 정한다. */
export const isWindow = label() === "window";

/** 결제 승인(모달·자율 승인)을 이 창이 맡는가. 한 줄 그대로 두기 — 백엔드 테스트가 이 줄을 글자로 대조한다. */
export const isApprover = label() === "window";

/** 첫 프레임 전에 `<html>` 에 표식을 단다(main.tsx 가 렌더 전에 부른다). */
export function markWindowKind(): void {
  if (isWindow) document.documentElement.dataset.win = "window";
}

/** 내역을 표로 펼칠 만큼 넓은가 — Tailwind `md`(768px)와 같은 경계. 팝오버(420)는 늘 false. */
const WIDE = "(min-width: 768px)";

export function useWide(): boolean {
  const [wide, setWide] = useState(() => window.matchMedia(WIDE).matches);
  useEffect(() => {
    const mq = window.matchMedia(WIDE);
    const on = () => setWide(mq.matches);
    mq.addEventListener("change", on);
    return () => mq.removeEventListener("change", on);
  }, []);
  return wide;
}
