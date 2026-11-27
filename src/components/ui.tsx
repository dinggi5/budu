// 공용 UI 프리미티브 — 셸/카드/버튼 스타일 + 작은 컴포넌트 (디자인 시스템의 실체).

import { Bot, ChevronRight, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { isWindow } from "@/lib/win";

// 메뉴바 팝오버 셸 — 창이 고정 크기·투명이라 배경과 둥근 모서리를 여기서 칠하고,
// 넘치는 내용은 바깥이 아니라 이 안에서 스크롤한다. justify-between 은 내용이 넘칠 때
// 첫 아이템을 시작점에 두므로(space-between) 위가 잘리지 않는다.
export const shell = cn(
  "h-screen w-full overflow-y-auto flex flex-col items-center justify-between",
  "rounded-[var(--radius-window)]",
  "bg-[var(--color-ivory-100)] text-[var(--color-ink-900)]",
  "dark:bg-[var(--color-night-900)] dark:text-[#E8E5DD]",
  "px-6 py-8",
  // 큰 창: 보통 맥 창이라 모서리는 창이 둥글린다. 위는 신호등 자리(56px)를 비우고, 내용은
  // 팝오버 높이(640)만큼의 판을 창 한가운데에 둔다 — 가운데 맞춤(justify-center)이 아니라 **위아래 여백을
  // 늘려서**. 가운데 맞춤은 내용이 창보다 길 때 위가 잘려 스크롤로도 닿지 않는다(위 justify-between 주석).
  "win:rounded-none win:px-10",
  "win:pt-[max(3.5rem,calc((100vh_-_640px)/2))] win:pb-[max(2.5rem,calc((100vh_-_640px)/2))]",
);

/** 큰 창 레일 오른쪽에 뜨는 하위 화면(설정·연결·도움말·백업)의 셸 — 장부 페이지(`WindowPages` 의 PageFrame)와 같은 자리에
 *  제목을 두고 왼쪽 정렬한다. 팝오버에선 `shell` 과 같다. */
export const pageShell = cn(
  shell,
  "win:items-start win:justify-start win:gap-8 win:pt-14 win:pb-14 win:px-6 min-[1100px]:win:px-10",
);

/** 하위 화면 머리 — 팝오버는 작은 「아이콘 제목 · 닫기」 한 줄, 큰 창(레일이 있을 때)은 장부 제목(24px)과 한 줄 설명.
 *  큰 창엔 닫기가 없다 — 레일이 길이다(설정의 저장 안 한 변경은 WalletScreen 의 이탈 확인이 지킨다). */
export function ScreenHeader({
  icon,
  title,
  sub,
  onClose,
  closeLabel,
  closeDisabled,
  railed = true,
}: {
  icon: React.ReactNode;
  title: string;
  sub?: string;
  onClose: () => void;
  closeLabel?: string;
  closeDisabled?: boolean;
  /** 레일 안에 뜨는가 — 큰 창이어도 첫 실행 백업(레일 없음)은 닫기(«나중에»)가 있어야 한다. */
  railed?: boolean;
}) {
  if (isWindow && railed) {
    return (
      <header className="w-full max-w-md win:max-w-none">
        <h1 className="text-lg leading-tight font-bold tracking-[-0.01em] text-[var(--color-ink-900)] dark:text-[#E8E5DD]">{title}</h1>
        {sub && <p className="mt-1.5 text-md leading-relaxed text-[var(--color-ink-500)]">{sub}</p>}
      </header>
    );
  }
  return (
    <header className="w-full max-w-md flex items-center justify-between text-sm text-[var(--color-ink-500)]">
      <span className="flex items-center gap-2">
        <span className="text-[var(--color-ok)] inline-flex">{icon}</span>
        {title}
      </span>
      <button
        type="button"
        onClick={onClose}
        disabled={closeDisabled}
        className="hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors disabled:opacity-40 disabled:pointer-events-none"
      >
        {closeLabel ?? t("닫기", "Close")}
      </button>
    </header>
  );
}

/** 하위 화면의 열 폭 — 팝오버 448, 큰 창은 상한 없이 꽉. 넓어지면 설정은 두 단으로 흐른다. */
export const pageColumn = "w-full max-w-md win:max-w-none";

/** 큰 창 묶음 머리 — 15px 굵은 제목. 선은 긋지 않는다(묶음은 여백과 글자 위계로 나눈다). */
export function LedgerHead({ title, tag }: { title: string; tag?: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <h2 className="text-md font-semibold text-[var(--color-ink-900)] dark:text-[#E8E5DD]">{title}</h2>
      {tag}
    </div>
  );
}

// 화면 전체를 덮는 모달 배경 — 팝오버 모서리를 넘어 사각으로 칠해지지 않게 같이 둥글린다.
// items-start + 카드의 my-auto 조합인 이유: items-center 로 가운데 정렬하면 카드가 창보다
// 길 때 위아래가 동시에 잘리고 스크롤로도 닿지 않는다(플렉스 오버플로 함정). auto 마진은
// 여유가 있을 땐 가운데로, 넘칠 땐 0 으로 접혀 위부터 스크롤된다.
export const modalOverlay = cn(
  "fixed inset-0 z-50 overflow-y-auto flex items-start justify-center p-5",
  "rounded-[var(--radius-window)] bg-black/40 backdrop-blur-sm",
  "win:rounded-none",
);

export const cardBase = cn(
  "w-full",
  "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
  "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
  "rounded-[var(--radius-card)]",
  "shadow-[var(--shadow-soft)]",
);

// 모달 카드 공통 — 팝오버(420x640)보다 길어져도 닿을 수 있게 my-auto 를 반드시 함께 쓴다
// (modalOverlay 의 items-start 와 짝).
/** 큰 창 하위 화면의 판 — 팝오버에선 카드, 큰 창에선 카드를 걷어 낸 평평한 묶음 — 설정·연결·백업·도움말이
 *  다른 페이지와 같은 모양이 되게. 장부 페이지가 카드 없이 선과 여백으로 위계를 세우니 같은 문법으로. 모달의 카드(modalCard)는 그대로. */
export const flatCard = cn(
  cardBase,
  "win:bg-transparent win:dark:bg-transparent win:border-0 win:shadow-none win:rounded-none win:px-0 win:py-0",
);

export const modalCard = cn(cardBase, "relative w-full max-w-sm my-auto");

export const enter = {
  initial: { opacity: 0, y: 8 },
  animate: { opacity: 1, y: 0 },
  exit: { opacity: 0, y: -8 },
  transition: { duration: 0.32, ease: [0.4, 0, 0.2, 1] as const },
};

export const inputBase = cn(
  "w-full h-11 px-3.5 rounded-[var(--radius-card)]",
  "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
  "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
  "text-[var(--color-ink-900)] dark:text-[#E8E5DD]",
  "placeholder:text-[var(--color-ink-300)]",
  "outline-none transition-colors duration-[var(--duration-base)]",
  "focus:border-[var(--color-ok)]",
);

export const primaryBtn = cn(
  "w-full h-11 inline-flex items-center justify-center gap-2",
  "text-md tracking-tight",
  "rounded-[var(--radius-card)]",
  "bg-[var(--color-ok)] text-white",
  "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
  "hover:not-disabled:brightness-105 hover:not-disabled:translate-y-[-1px]",
  "hover:not-disabled:shadow-[var(--shadow-soft)]",
  "active:not-disabled:translate-y-0",
  "disabled:opacity-40 disabled:cursor-not-allowed",
);

export const secondaryBtn = cn(
  "h-10 inline-flex items-center justify-center gap-2",
  "text-md tracking-tight",
  "rounded-[var(--radius-card)]",
  "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
  "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
  "text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
  "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
  "hover:not-disabled:bg-[var(--color-ivory-200)]",
  "disabled:opacity-50",
);

export function FlowIcon({ children }: { children: React.ReactNode }) {
  return (
    <div className="flow-icon flex justify-center">
      <div className="w-12 h-12 rounded-full flex items-center justify-center bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)] text-[var(--color-ok)]">
        {children}
      </div>
    </div>
  );
}

export function FieldHint({ children }: { children: React.ReactNode }) {
  return <p className="mt-1.5 text-sm text-[var(--color-danger)]">{children}</p>;
}

export function PwInput({
  id,
  value,
  onChange,
  placeholder,
  autoFocus,
  onEnter,
}: {
  id?: string;
  value: string;
  onChange: (v: string) => void;
  placeholder: string;
  autoFocus?: boolean;
  onEnter?: () => void;
}) {
  return (
    <input
      id={id}
      type="password"
      value={value}
      autoFocus={autoFocus}
      onChange={(e) => onChange(e.target.value)}
      onKeyDown={(e) => e.key === "Enter" && onEnter?.()}
      placeholder={placeholder}
      className={cn(inputBase, "text-md tracking-wide")}
    />
  );
}

export function CloseButton({ onClose }: { onClose: () => void }) {
  return (
    <button
      type="button"
      onClick={onClose}
      className={cn(
        "absolute top-4 right-4 p-1.5 rounded-full",
        "text-[var(--color-ink-300)] hover:text-[var(--color-ink-700)]",
        "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
        "transition-colors duration-[var(--duration-base)]",
      )}
      aria-label={t("닫기", "Close")}
    >
      <X size={14} />
    </button>
  );
}

/** 헤더 우측 아이콘 버튼 (잠금·내역·백업·설정 공용). tone으로 강조색을 바꾼다. */
export function HeaderIconButton({
  onClick,
  label,
  tone = "default",
  children,
}: {
  onClick: () => void;
  label: string;
  tone?: "default" | "danger" | "warn";
  children: React.ReactNode;
}) {
  const toneClass =
    tone === "danger"
      ? "text-[var(--color-danger)] hover:text-[var(--color-danger-active)] hover:bg-[var(--color-danger-tint)]"
      : tone === "warn"
        ? "text-[var(--color-danger)] hover:text-[var(--color-danger-active)] hover:bg-[var(--color-danger-tint)]"
        : cn(
            "text-[var(--color-ink-500)] dark:text-[#B5AFA2]",
            "hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
            "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
          );
  return (
    <button
      type="button"
      onClick={onClick}
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex items-center justify-center w-7 h-7 rounded-full",
        "transition-colors duration-[var(--duration-base)]",
        toneClass,
      )}
    >
      {children}
    </button>
  );
}

export function ActionButton({
  icon,
  label,
  disabled,
  onClick,
  active,
}: {
  icon: React.ReactNode;
  label: string;
  disabled?: boolean;
  onClick?: () => void;
  active?: boolean;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "w-full h-11 px-4 inline-flex items-center justify-center gap-2",
        "text-md tracking-tight",
        "rounded-[var(--radius-card)]",
        "border",
        active
          ? "border-[var(--color-ok)] bg-[var(--color-ivory-50)] text-[var(--color-ok)]"
          : "border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)] bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)] text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
        "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
        "hover:not-disabled:bg-[var(--color-ivory-200)] hover:not-disabled:translate-y-[-1px]",
        "hover:not-disabled:shadow-[var(--shadow-soft)]",
        "active:not-disabled:translate-y-0",
        "disabled:opacity-50 disabled:cursor-not-allowed",
      )}
    >
      {icon}
      {label}
    </button>
  );
}

export function FaucetButton({
  icon,
  label,
  sub,
  onClick,
}: {
  icon: React.ReactNode;
  label: string;
  sub: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "h-12 inline-flex items-center justify-center gap-2",
        "rounded-[var(--radius-card)]",
        "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
        "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
        "text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
        "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
        "hover:bg-[var(--color-ivory-200)] hover:translate-y-[-1px]",
        "hover:shadow-[var(--shadow-soft)]",
        "active:translate-y-0",
      )}
    >
      <span className="text-[var(--color-ink-300)]">{icon}</span>
      <span className="text-md tracking-tight">{label}</span>
      <span className="text-sm text-[var(--color-ink-300)]">{sub}</span>
    </button>
  );
}

/** 클라이언트 이름을 보기 좋게. 모르면 "AI". */
function prettyClient(c: string): string {
  if (!c) return "AI";
  const l = c.toLowerCase();
  if (l.includes("claude")) return "Claude";
  if (l.includes("cursor")) return "Cursor";
  if (l.includes("cline")) return "Cline";
  if (l.includes("windsurf")) return "Windsurf";
  return c;
}

/** AI 연결 배지 — 제품 컨셉(AI 전용 지갑)의 시각화. 상태는 셋이다.
 *
 *   쓰는 중  — MCP 가 지금 떠 있다(하트비트). Claude 가 켜져 있고 Budu 를 물고 있다.
 *   준비됨   — 등록이 살아 있다. Claude 를 켜면 Claude 가 우리를 띄운다 — 사람이 할 일이 없다.
 *   연결 필요 — 등록이 없거나 다른 곳을 가리킨다. 이때만 「할 일」처럼 보인다.
 *
 *  🔴 예전엔 하트비트만 봐서 둘째를 셋째로 그렸다 — 등록이 멀쩡한데 Claude 를 안 켠 동안 내내
 *  「AI 연결 안 됨」. MCP 엔 상주 연결이 없으니 사람들은 매일 「연결」을 다시 누르게 됐다.
 *  ready 가 null 이면(첫 조회 전) 그 자리만 비워 둔다 — 「연결 필요」가 번쩍였다 사라지지 않게.
 *  onClick 을 주면 버튼이 된다 — AI 연결 화면 진입점. */
export function AgentBadge({
  connected,
  ready,
  client,
  onClick,
}: {
  connected: boolean;
  ready: boolean | null;
  client: string;
  onClick?: () => void;
}) {
  const name = prettyClient(client);
  if (!connected && ready === null) return <span className="h-[26px]" aria-hidden />;
  const state = connected ? "live" : ready ? "ready" : "needs";
  const title =
    state === "live"
      ? t(
          `${name}가 지금 이 지갑을 쓰고 있어요. 결제를 요청하면 승인 창이 떠요.`,
          `${name} is using this wallet right now. When it asks to pay, an approval window opens.`,
        )
      : state === "ready"
        ? t(
            "연결은 끝나 있어요. Claude를 켜면 알아서 붙고, Budu가 꺼져 있어도 결제를 요청할 때 깨워요.",
            "You're connected. Open Claude and it attaches on its own — it even wakes Budu when it needs to pay.",
          )
        : t(
            "아직 이 앱과 연결된 AI가 없어요. 누르면 한 번에 연결할 수 있어요.",
            "No AI is connected to this app yet. Tap to connect in one step.",
          );
  const className = cn(
    "inline-flex items-center gap-1.5 px-2.5 py-1 rounded-full",
    "text-sm tracking-tight border select-none",
    "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
    "transition-colors duration-[var(--duration-base)]",
    state === "live"
      ? "border-[var(--color-ok)] text-[var(--color-ok)]"
      : state === "ready"
        ? "border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)] text-[var(--color-ink-500)]"
        : "border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)] text-[var(--color-ink-300)]",
    onClick && "cursor-pointer hover:border-[var(--color-ok)] hover:text-[var(--color-ok)]",
  );
  const inner = (
    <>
      <Bot size={12} />
      {state === "live" ? (
        <>
          <span>{t(`${name} 쓰는 중`, `${name} in use`)}</span>
          <span className="relative flex w-1.5 h-1.5" aria-hidden>
            <span className="absolute inline-flex w-full h-full rounded-full bg-[var(--color-ok)] opacity-60 animate-ping" />
            <span className="relative inline-flex w-1.5 h-1.5 rounded-full bg-[var(--color-ok)]" />
          </span>
        </>
      ) : state === "ready" ? (
        <>
          <span className="inline-flex w-1.5 h-1.5 rounded-full bg-[var(--color-ok)] opacity-70" aria-hidden />
          <span>{t("준비됨 · Claude를 켜면 자동으로 연결돼요", "Ready · connects when you open Claude")}</span>
        </>
      ) : (
        <>
          <span>{t("AI 연결 필요", "Connect an AI")}</span>
          <ChevronRight size={12} aria-hidden />
        </>
      )}
    </>
  );
  return onClick ? (
    <button type="button" onClick={onClick} title={title} className={className}>
      {inner}
    </button>
  ) : (
    <span title={title} className={className}>
      {inner}
    </span>
  );
}

/** 공용 on/off 스위치 — 설정의 ToggleRow·업데이트 블록, 환영 투어의 연결 단계가 같은 스위치를 쓴다(개발 39 정리 전엔
 *  같은 마크업이 두 벌 복사돼 있었다). 접근성 라벨은 행 제목을 그대로 받는다. */
export function Switch({
  checked,
  onToggle,
  label,
}: {
  checked: boolean;
  onToggle: () => void;
  label: string;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      onClick={onToggle}
      className={cn(
        "shrink-0 relative w-10 h-6 rounded-full transition-colors duration-[var(--duration-base)]",
        checked
          ? "bg-[var(--color-ok)]"
          : "bg-[var(--color-ivory-400)] dark:bg-[var(--color-night-700)]",
      )}
    >
      <span
        className={cn(
          "absolute top-1 w-4 h-4 rounded-full bg-white shadow-sm transition-all duration-[var(--duration-base)]",
          checked ? "left-5" : "left-1",
        )}
      />
    </button>
  );
}

