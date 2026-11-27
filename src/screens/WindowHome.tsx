// 큰 창의 홈 — 시안 B «장부». 개발 9 부터 모든 페이지가 이 화면을 기준으로 맞춘다.
//
// 모양: 왼쪽 64px 아이콘 레일 / 가운데 잔액 숫자 하나(64px) + 게이지 둘(오늘 한도·작업 예산) + 맨 아래 받는 주소 /
// 오른쪽 320px 회색 판 = 작업 패널(열린 작업 + 일지, ledger/JobPanel — 홈에만 있다).
// 카드·테두리를 빼고 숫자와 여백으로 위계를 세운다. 보라는 돈이 걸린 곳(게이지 채움·작업 아이콘)에만.
//
// 데이터는 WalletScreen 이 이미 폴링하는 것을 그대로 받는다. 금액·상태 규칙은 lib/ledger.ts 한 곳.

import {
  Database,
  HelpCircle,
  History,
  Home,
  KeyRound,
  PanelRight,
  Plug,
  QrCode,
  Send,
  RotateCw,
  Settings as SettingsIcon,
  ShieldAlert,
  ShieldCheck,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { accountName, fmtExact, shortenAddress } from "@/lib/format";
import { t } from "@/lib/i18n";
import { balanceText, limitKind, minusDec, money, ratio } from "@/lib/ledger";
import { JobPanel } from "@/screens/ledger/JobPanel";
import { Gauge, GhostButton, IconButton } from "@/components/win";
import type { Account, AgentStatus, Balances, HistoryEntry, JobView, Settings, SpendView } from "@/lib/types";

// ── 레일 ──────────────────────────────────────────────────────────────────────────────

export type RailPage = "home" | "history" | "sellers" | "connect" | "backup" | "settings" | "help";

/** 큰 창의 왼쪽 레일. 위 = 보는 곳, 아래(빈자리 뒤) = 지갑을 지키는 것(백업·잠금·설정). */
export function Rail({
  page,
  onPage,
  locked,
  onToggleLock,
  backedUp,
}: {
  page: RailPage;
  onPage: (p: RailPage) => void;
  locked: boolean;
  onToggleLock: () => void;
  backedUp: boolean;
}) {
  return (
    // 위 60px = 신호등 자리. 끌기 띠(App 의 DragStrip, h-8)는 그 안에만 걸친다.
    // 오른쪽 선은 신호등 아래(56px)부터 — 창 맨 위부터 그으면 초록 원 옆을 스쳐 엉성하다.
    <nav className="relative h-screen w-16 shrink-0 flex flex-col items-center gap-1.5 pt-[60px] pb-4">
      <span aria-hidden className="absolute right-0 top-14 bottom-0 w-px bg-[var(--color-ivory-300)] dark:bg-[var(--color-night-700)]" />
      <RailButton on={page === "home"} onClick={() => onPage("home")} label={t("홈", "Home")}>
        <Home size={18} />
      </RailButton>
      <RailButton on={page === "history"} onClick={() => onPage("history")} label={t("작업 내역", "Job history")}>
        <History size={18} />
      </RailButton>
      <RailButton on={page === "sellers"} onClick={() => onPage("sellers")} label={t("데이터 구매처", "Data sources")}>
        <Database size={18} />
      </RailButton>
      <RailButton on={page === "connect"} onClick={() => onPage("connect")} label={t("AI 연결", "Connect AI")}>
        <Plug size={18} />
      </RailButton>
      <div className="flex-1" />
      <RailButton
        on={page === "backup"}
        onClick={() => onPage("backup")}
        label={backedUp ? t("시드 백업", "Back up your words") : t("시드 백업 — 아직 안 했어요", "Back up your words — not done yet")}
        dot={!backedUp}
      >
        <KeyRound size={18} />
      </RailButton>
      <RailButton
        on={false}
        onClick={onToggleLock}
        label={
          locked
            ? t("긴급 잠금 해제", "Turn off emergency lock")
            : t("긴급 잠금 (모든 결제 차단)", "Emergency lock (blocks every payment)")
        }
        danger={locked}
      >
        {locked ? <ShieldAlert size={18} /> : <ShieldCheck size={18} />}
      </RailButton>
      <RailButton on={page === "help"} onClick={() => onPage("help")} label={t("도움말", "Help")}>
        <HelpCircle size={18} />
      </RailButton>
      <RailButton on={page === "settings"} onClick={() => onPage("settings")} label={t("설정", "Settings")}>
        <SettingsIcon size={18} />
      </RailButton>
    </nav>
  );
}

function RailButton({
  on,
  onClick,
  label,
  danger,
  dot,
  children,
}: {
  on: boolean;
  onClick: () => void;
  label: string;
  danger?: boolean;
  /** 해야 할 일이 남았다(백업 안 함) — 작은 점 하나. */
  dot?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      aria-current={on ? "page" : undefined}
      className={cn(
        "group relative w-10 h-10 rounded-[10px] grid place-items-center transition-colors duration-[var(--duration-fast)]",
        on
          ? "bg-[var(--color-ivory-200)] text-[var(--color-ink-900)] dark:bg-[var(--color-night-700)] dark:text-[#E8E5DD]"
          : "text-[var(--color-ink-500)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
        danger && "text-[var(--color-danger)] hover:text-[var(--color-danger-active)]",
      )}
    >
      {children}
      {dot && <span className="absolute top-2 right-2 w-1.5 h-1.5 rounded-full bg-[var(--color-danger)]" aria-hidden />}
      {/* 이름 툴팁 — 브라우저 기본 title 은 늦게 뜨고 WebKit 에선 안 뜰 때도 있어 직접 그린다. */}
      <span
        aria-hidden
        className={cn(
          "pointer-events-none absolute left-full ml-2.5 top-1/2 -translate-y-1/2 z-50 whitespace-nowrap",
          "rounded-[8px] px-2.5 py-1.5 text-sm font-medium shadow-[var(--shadow-lift)]",
          "bg-[var(--color-ink-900)] text-white dark:bg-[#E8E5DD] dark:text-[var(--color-ink-900)]",
          "opacity-0 group-hover:opacity-100 group-focus-visible:opacity-100 transition-opacity duration-[var(--duration-fast)] group-hover:delay-300",
        )}
      >
        {label}
      </span>
    </button>
  );
}

// ── 홈 ────────────────────────────────────────────────────────────────────────────────

export function WindowHome({
  account,
  address,
  balances,
  balanceError,
  refreshing,
  onRefresh,
  agent,
  onConnect,
  settings,
  spend,
  job,
  onEndJob,
  history,
  onOpenJob,
  onReceive,
  onSend,
  panelOpen,
  onPanel,
  notices,
  locked,
}: {
  account: Account;
  address: string;
  balances: Balances | null;
  balanceError: string | null;
  refreshing: boolean;
  onRefresh: () => void;
  agent: AgentStatus;
  onConnect: () => void;
  settings: Settings | null;
  spend: SpendView | null;
  job: JobView | null;
  onEndJob: () => void;
  history: HistoryEntry[] | null;
  /** 작업 이름을 누르면 장부의 그 작업으로. */
  onOpenJob: (label: string) => void;
  onReceive: () => void;
  onSend: () => void;
  /** 오른쪽 작업 패널 — 홈에만 있고, 오른쪽 위 단추로 닫고 연다. */
  panelOpen: boolean;
  onPanel: (open: boolean) => void;
  /** 잠금·백업·자율 결제·설정 고장 띠 — WalletScreen 이 고른 것을 그대로 받는다. */
  notices: React.ReactNode;
  /** 긴급 잠금 — 작업은 살아 있지만 결제가 멈춘다. 「열린 작업 없음」 이라고 쓰면 거짓이다. */
  locked: boolean;
}) {
  const chain = useChain();
  const spent = spend?.usdc;
  const daily = settings?.daily_usdc?.trim() ?? "";
  const dailyKind = limitKind(settings?.daily_usdc);

  return (
    <div
      className={cn(
        "h-screen min-w-0 flex-1 grid grid-cols-1 overflow-y-auto min-[880px]:overflow-hidden",
        panelOpen && "min-[880px]:grid-cols-[1fr_320px]",
      )}
    >
      {/* 패널 토글 — 열려 있든 닫혀 있든 같은 자리의 네모 단추 하나(장부의 목록 접기와 같은 모양). */}
      <div className="fixed top-3 right-4 z-40">
        <IconButton
          label={panelOpen ? t("작업 패널 닫기", "Hide job panel") : t("작업 패널 열기", "Show job panel")}
          onClick={() => onPanel(!panelOpen)}
        >
          <PanelRight size={16} />
        </IconButton>
      </div>
      <section className="min-w-0 flex flex-col px-8 min-[880px]:px-14 pt-[72px] pb-10 min-[880px]:overflow-y-auto">
        {/* 눈썹 — 지금 어느 계정·어느 체인인지, AI 가 붙어 있는지. 잔액보다 먼저 읽혀야 하는 사실. */}
        <div className="flex items-center gap-2 text-md text-[var(--color-ink-500)] min-w-0">
          <span className="truncate text-[var(--color-ink-700)] dark:text-[#B5AFA2]">{accountName(account)}</span>
          <span aria-hidden>·</span>
          <span className={cn("shrink-0", !chain.testnet && "text-[var(--color-danger)] font-medium")}>{chain.name}</span>
          <span aria-hidden>·</span>
          <button
            type="button"
            onClick={onConnect}
            className={cn(
              "shrink-0 inline-flex items-center gap-1.5 hover:underline underline-offset-2",
              agent.connected ? "text-[var(--color-ok)]" : "text-[var(--color-ink-500)]",
            )}
          >
            <span
              className={cn("w-1.5 h-1.5 rounded-full", agent.connected ? "bg-[var(--color-ok)]" : "bg-[var(--color-ink-300)]")}
              aria-hidden
            />
            {agent.connected ? agent.client || t("AI 연결됨", "AI connected") : t("AI 연결 안 됨", "AI not connected")}
          </button>
        </div>

        {/* 잔액 — 이 화면의 주인공 하나. */}
        <div className="mt-3.5 flex flex-wrap items-baseline gap-x-2 gap-y-1">
          <span className="balance num text-hero leading-none font-bold tracking-[-0.03em] text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
            {balanceText(balances)}
          </span>
          <span className="text-lg font-semibold text-[var(--color-ink-500)]">USDC</span>
          <button
            type="button"
            onClick={onRefresh}
            title={t("잔액 새로고침", "Refresh balance")}
            aria-label={t("잔액 새로고침", "Refresh balance")}
            className="ml-1 self-center w-8 h-8 grid place-items-center rounded-full text-[var(--color-ink-300)] hover:text-[var(--color-ink-700)] transition-colors"
          >
            <RotateCw size={14} className={cn(refreshing && "animate-spin")} />
          </button>
        </div>
        <p className="mt-4 text-md text-[var(--color-ink-500)] num">
          {balanceError ? (
            <span className="text-[var(--color-danger)]">{t("잔액을 못 읽었어요 — ", "Couldn't read the balance — ")}{balanceError}</span>
          ) : spent == null ? (
            t("오늘 쓴 돈을 못 읽었어요 · 결제는 한도 확인이 막아요", "Couldn't read today's spending · payments stay blocked by the limit check")
          ) : (
            t(`오늘 총 ${money(spent)} USDC 를 썼어요`, `Spent ${money(spent)} USDC in total today`)
          )}
        </p>

        {/* 게이지 둘 — 오늘 한도 · 작업 예산. */}
        <div className="mt-12 grid grid-cols-1 sm:grid-cols-2 gap-8">
          <Gauge
            title={t("오늘 한도", "Today's limit")}
            total={dailyKind === "set" ? money(daily) : dailyKind === "unlimited" ? t("없음", "none") : ""}
            value={
              dailyKind === "loading"
                ? "—"
                : dailyKind === "unknown"
                  ? t("한도 못 읽음 · 결제 막힘", "Limit unreadable · blocked")
                  : dailyKind === "unlimited"
                    ? t("한도 없이 써요", "No daily limit")
                    : spent == null
                      ? "—"
                      : t(`${minusDec(daily, spent)} 남음`, `${minusDec(daily, spent)} left`)
            }
            fill={dailyKind === "set" && spent != null ? ratio(spent, daily) : 0}
            warn={dailyKind === "unknown"}
          />
          <Gauge
            title={t("작업 예산", "Job budget")}
            total={job ? money(job.budget) : ""}
            value={
              locked
                ? t("긴급 잠금 중 · 결제 멈춤", "Emergency lock on · payments paused")
                : job
                  ? t(`${money(job.left)} 남음`, `${money(job.left)} left`)
                  : t("진행 중인 작업 없음", "No job in progress")
            }
            fill={job && !locked ? ratio(job.spent, job.budget) : 0}
            muted={!job || locked}
          />
        </div>

        {/* 띠(잠금·자율 결제·백업·설정 고장) — 있을 때만, 게이지 아래 조용히. */}
        {notices && <div className="mt-10 max-w-md flex flex-col gap-3">{notices}</div>}

        {/* 맨 아래 — 받는 주소 · SOL. */}
        <div className="mt-auto pt-10">
          <div>
            <AddressLine address={address} sol={balances?.sol} onReceive={onReceive} onSend={onSend} />
          </div>
        </div>
      </section>

      {panelOpen && (
        <JobPanel
          job={job}
          locked={locked}
          entries={history}
          onEnd={onEndJob}
          onOpenJob={onOpenJob}
          className="min-[880px]:h-screen"
        />
      )}
    </div>
  );
}

function AddressLine({
  address,
  sol,
  onReceive,
  onSend,
}: {
  address: string;
  sol: string | undefined;
  onReceive: () => void;
  onSend: () => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-x-8 gap-y-4 min-w-0">
      <div className="min-w-0">
        <span className="block text-sm text-[var(--color-ink-500)]">{t("받는 주소", "Receiving address")}</span>
        <span className="block mt-0.5 font-mono text-md text-[var(--color-ink-700)] dark:text-[#B5AFA2] truncate" title={address}>
          {shortenAddress(address)}
        </span>
      </div>
      {/* SOL — 결제엔 필요 없지만 솔라나 지갑이라 받고 보낼 수는 있다. */}
      <div className="min-w-0">
        <span className="block text-sm text-[var(--color-ink-500)]">SOL</span>
        <span className="block mt-0.5 num text-md text-[var(--color-ink-700)] dark:text-[#B5AFA2]">{sol != null ? fmtExact(sol) : "—"}</span>
      </div>
      <div className="ml-auto flex items-center gap-2 shrink-0">
        <GhostButton onClick={onReceive}>
          <QrCode size={14} />
          {t("받기", "Receive")}
        </GhostButton>
        <GhostButton onClick={onSend}>
          <Send size={14} />
          {t("보내기", "Send")}
        </GhostButton>
      </div>
    </div>
  );
}
