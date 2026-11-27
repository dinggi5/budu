// 앱 루트 — 지갑 상태(생성/마이그레이션/정상)에 따라 첫 화면을 고른다.
//
// 파일 지도 — 전체 구조는 docs/ARCHITECTURE.md.
//   lib/types.ts        백엔드와 주고받는 공유 타입
//   lib/ledger.ts       금액·상태·묶음 판정 한 곳(장부·홈·팝오버가 같이 쓴다)
//   lib/i18n.ts         화면 언어(한국어·영어) — t(ko, en)
//   lib/win.ts·sync.ts  창 종류(팝오버/큰 창) · 두 창 사이 다시 읽기 신호
//   components/ui.tsx   셸·카드·버튼 스타일, components/win.tsx 큰 창 공용 조각
//   components/…        BalanceCard·ReceiveCard·PaymentApprovalModal(승인 시트)·JobBar·SessionBar·banners
//   screens/WalletScreen  큰 창(레일 + 페이지) / 팝오버(미니)의 뿌리. 페이지 = WindowHome · ledger/* · Settings · Connect · Help · BackupFlow
//   screens/SetupScreen·LegacyWalletScreen  지갑 만들기·복원 / 옛 형식 지갑 고치기

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import type { WalletStatus } from "@/lib/types";
import { initLang, t } from "@/lib/i18n";
import { shell } from "@/components/ui";
import { SetupScreen } from "@/screens/SetupScreen";
import { LegacyWalletScreen } from "@/screens/LegacyWalletScreen";
import { WalletScreen } from "@/screens/WalletScreen";
import { isWindow, markWindowKind } from "@/lib/win";
import { useSync } from "@/lib/sync";
import "./App.css";

// 첫 프레임 전에 — 팝오버와 큰 창은 겉모양이 다르다.
markWindowKind();

/** 큰 창의 맨 위 띠 — 제목 막대를 숨겼으니(신호등만 남김) 여기를 잡고 창을 옮긴다. 두 번 누르면 확대.
 *  내용보다 위에 있지만 셸의 윗여백(win:pt) 안에만 걸쳐 버튼을 가리지 않는다. */
function DragStrip() {
  return <div data-tauri-drag-region className="fixed inset-x-0 top-0 h-8 z-30" aria-hidden />;
}

function App() {
  const view = <AppView />;
  return isWindow ? (
    <>
      <DragStrip />
      {view}
    </>
  ) : (
    view
  );
}

function AppView() {
  const [status, setStatus] = useState<WalletStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const loadStatus = useCallback(() => {
    invoke<WalletStatus>("get_wallet_status")
      .then(setStatus)
      .catch((e) => setStatusError(String(e)));
  }, []);

  // 언어는 모듈 로드 때 이미 정해져 있다(캐시·시스템). 이건 백엔드 설정과의 사후 대조 —
  // 어긋날 때만 창을 한 번 다시 읽는다.
  useEffect(() => {
    void initLang();
  }, []);

  // ⌘W = 창 닫기(= 숨기기). 개발 58 — 실물에서 이 키가 아무 일도 안 하는 걸 발견했다.
  // macOS 는 ⌘W 를 앱 메뉴의 「닫기」로 보내는데 팝오버는 테두리가 없어(decorations: false)
  // AppKit 이 그 항목을 비활성화한다 → 러스트의 CloseRequested 핸들러까지 오지 않는다.
  // 그래서 웹뷰에서 직접 받아 `hide_main_window` 로 보낸다(큰 창 = appwin::hide_by_user, 팝오버 = tray::hide).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (!e.metaKey || e.ctrlKey || e.altKey) return;
      if (e.key.toLowerCase() !== "w") return;
      e.preventDefault();
      void invoke("hide_main_window");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  useEffect(loadStatus, [loadStatus]);

  // 다른 창에서 계정을 바꾸거나 백업을 마치거나 언어를 바꿨으면 이 창도 따라간다.
  // 처음 읽기와 달리 실패는 조용히 — 멀쩡히 떠 있는 지갑 화면을 오류 화면으로 바꿀 이유가 없다.
  useSync(() => {
    void initLang();
    invoke<WalletStatus>("get_wallet_status")
      .then(setStatus)
      .catch(() => {});
  });

  // 로딩
  if (!status && !statusError) {
    return (
      <main className={shell}>
        <div />
        <span className="text-[13px] text-[var(--color-ink-300)] font-mono">
          {t("지갑 여는 중…", "Opening your wallet…")}
        </span>
        <div />
      </main>
    );
  }

  if (statusError) {
    return (
      <main className={shell}>
        <div />
        <p className="text-[12px] text-red-500 font-mono max-w-md text-center">
          {t("지갑 상태 확인 실패: ", "Couldn't read wallet status: ")}
          {statusError}
        </p>
        <div />
      </main>
    );
  }

  // 비번 미설정 (legacy 평문 / 신규) → 보호 화면
  if (status!.state !== "encrypted") {
    return (
      <SetupScreen
        status={status!}
        onDone={(address, backedUp) =>
          // 방금 만든(가져온) 지갑은 계정 0 하나다.
          setStatus({
            state: "encrypted",
            address,
            backed_up: backedUp,
            accounts: [{ index: 0, address, label: "" }],
            active: 0,
            legacy_address: false,
          })
        }
      />
    );
  }

  // 개발 1(EVM 틀) 시절 파일 — 주소가 `0x…` 라 지갑 화면은 잔액부터 막힌다. 비번 한 번으로 고친다.
  if (status!.legacy_address) {
    return <LegacyWalletScreen onDone={setStatus} />;
  }

  return (
    <WalletScreen
      address={status!.address!}
      initialBackedUp={status!.backed_up}
      accounts={status!.accounts}
      active={status!.active}
    />
  );
}

export default App;
