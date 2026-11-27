// 큰 창 공용 조각 — 홈 기준으로 통일, 창을 꽉 채운다.
//
// 문법 하나로 모든 페이지를 그린다:
//   - 고른 항목은 선이 아니라 **회색 둥근 면**(`Selectable`). 목록 사이 구분선은 아주 옅게, 묶음 머리엔 선을 긋지 않는다.
//   - 상태는 작은 색 꼬리표(`Tag`) — 초록(나감)·빨강(문제)·회색(대기·안 나감). 보라는 돈이 걸린 곳(게이지·열린 작업)에만.
//   - 도구는 둥근 아이콘 단추(`IconButton`).
//   - 열 폭에 상한을 두지 않는다 — 넓어지면 목록·상세 단이 늘어난다(기본 960×720 의 모습은 그대로).

import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { Check, Pencil, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import type { Tone } from "@/lib/ledger";

/** 창 폭 조건 — 단을 접고 펼 때. 팝오버(420)는 늘 false 쪽. */
export function useMedia(query: string): boolean {
  const [on, setOn] = useState(() => window.matchMedia(query).matches);
  useEffect(() => {
    const mq = window.matchMedia(query);
    const f = () => setOn(mq.matches);
    f();
    mq.addEventListener("change", f);
    return () => mq.removeEventListener("change", f);
  }, [query]);
  return on;
}

/** 요소의 실제 폭 — 창 폭이 아니라 레일·오른쪽 패널을 뺀 자리로 단을 접고 편다(패널을 열면 창 폭 기준이 거짓이 된다). */
export function useWidth<T extends HTMLElement>(): [(el: T | null) => void, number] {
  // 콜백 ref — 이른 return 으로 요소가 나중에 생기거나 바뀌어도 다시 잰다. 그리기 전에 재서 첫 프레임이 튀지 않게.
  const [el, setEl] = useState<T | null>(null);
  const [w, setW] = useState(() => window.innerWidth);
  useLayoutEffect(() => {
    if (!el) return;
    setW(el.getBoundingClientRect().width);
    const ro = new ResizeObserver(([e]) => setW(e.contentRect.width));
    ro.observe(el);
    return () => ro.disconnect();
  }, [el]);
  return [setEl, w];
}

/** 브라우저 저장소에 든 작은 화면 선호(단 접기 등). 못 읽으면 기본값 — 화면 모양일 뿐 돈과 무관하다. */
export function usePref<T extends string>(key: string, fallback: T): [T, (v: T) => void] {
  const [v, setV] = useState<T>(() => {
    try {
      return (localStorage.getItem(`budu.${key}`) as T | null) ?? fallback;
    } catch {
      return fallback;
    }
  });
  const save = (next: T) => {
    setV(next);
    try {
      localStorage.setItem(`budu.${key}`, next);
    } catch {
      /* 저장 못 해도 이번 화면에선 적용된다 */
    }
  };
  return [v, save];
}

export const ink = "text-[var(--color-ink-900)] dark:text-[#E8E5DD]";
export const inkSoft = "text-[var(--color-ink-700)] dark:text-[#B5AFA2]";
export const muted = "text-[var(--color-ink-500)]";
export const faint = "text-[var(--color-ink-300)]";
/** 가라앉은 판(왼쪽 목록 단·오른쪽 패널) — 홈의 일지 판과 같은 색. */
export const sunken = "bg-[var(--color-ivory-200)]/60 dark:bg-[var(--color-night-800)]";
/** 아주 옅은 목록 구분선. */
export const hairline = "border-[var(--color-ivory-200)] dark:border-[var(--color-night-700)]/70";

/** 페이지 제목 — 24px 굵게 + 한 줄 설명. 홈의 잔액 눈썹과 같은 왼쪽 선에 선다. */
export function PageTitle({ title, sub, aside }: { title: React.ReactNode; sub?: React.ReactNode; aside?: React.ReactNode }) {
  return (
    <div className="flex items-start justify-between gap-4">
      <div className="min-w-0">
        <h1 className={cn("text-lg leading-tight font-bold tracking-[-0.01em]", ink)}>{title}</h1>
        {sub && <p className={cn("mt-1.5 text-md leading-relaxed", muted)}>{sub}</p>}
      </div>
      {aside && <div className="shrink-0 flex items-center gap-1.5">{aside}</div>}
    </div>
  );
}

/** 작은 묶음 이름 — 선 없이 글자 위계로만(12px, 회색). */
export function Eyebrow({ children, className }: { children: React.ReactNode; className?: string }) {
  return <div className={cn("text-sm font-medium", muted, className)}>{children}</div>;
}

/** 고를 수 있는 줄 — 고르면 회색 둥근 면. */
export function Selectable({
  on,
  onClick,
  className,
  children,
  label,
}: {
  on: boolean;
  onClick: () => void;
  className?: string;
  children: React.ReactNode;
  label?: string;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={on}
      aria-label={label}
      className={cn(
        "w-full text-left rounded-[10px] transition-colors duration-[var(--duration-fast)]",
        on
          ? "bg-[var(--color-ivory-300)]/70 dark:bg-[var(--color-night-600)]"
          : "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]/60",
        className,
      )}
    >
      {children}
    </button>
  );
}

/** 상태 꼬리표 — Ulysses 의 «초고·퇴고» 같은 작은 색 표. */
export function Tag({ tone, children, className }: { tone: Tone | "accent"; children: React.ReactNode; className?: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 h-5 px-1.5 rounded-[6px] text-sm font-medium whitespace-nowrap",
        tone === "ok" && "text-[var(--color-ok)] bg-[var(--color-ok-tint)]",
        tone === "bad" && "text-[var(--color-danger)] bg-[var(--color-danger-tint)]0/10",
        tone === "idle" && "text-[var(--color-ink-500)] bg-[var(--color-ivory-300)]/60 dark:bg-[var(--color-night-600)] dark:text-[#B5AFA2]",
        tone === "accent" && "text-[var(--color-ok)] bg-[var(--color-ok-tint)] dark:bg-[var(--color-ok)]/15",
        className,
      )}
    >
      {children}
    </span>
  );
}

/** 둥근 아이콘 단추. */
export function IconButton({
  label,
  onClick,
  on,
  children,
}: {
  label: string;
  onClick: () => void;
  on?: boolean;
  children: React.ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      title={label}
      aria-label={label}
      aria-pressed={on}
      className={cn(
        "w-8 h-8 shrink-0 rounded-full grid place-items-center transition-colors duration-[var(--duration-fast)]",
        on
          ? "bg-[var(--color-ivory-300)]/70 text-[var(--color-ink-900)] dark:bg-[var(--color-night-600)] dark:text-[#E8E5DD]"
          : "text-[var(--color-ink-500)] hover:bg-[var(--color-ivory-200)] hover:text-[var(--color-ink-900)] dark:hover:bg-[var(--color-night-700)] dark:hover:text-[#E8E5DD]",
      )}
    >
      {children}
    </button>
  );
}

/** 이름 고치기 — 평소엔 글자 + 연필, 누르면 그 자리에서 입력 칸. Enter 저장 · Esc 취소. 빈 값 저장 = 원래 이름으로. */
export function RenameField({
  value,
  placeholder,
  onSave,
  className,
  size = "lg",
}: {
  value: string;
  /** 빈 값으로 저장하면 돌아갈 이름(AI 가 붙인 이름표). */
  placeholder: string;
  onSave: (next: string) => Promise<void>;
  className?: string;
  size?: "lg" | "sm";
}) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(value);
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => {
    if (editing) input.current?.select();
  }, [editing]);
  const text = size === "lg" ? "text-lg font-bold leading-tight tracking-[-0.01em]" : "text-md font-semibold";
  async function save() {
    if (busy) return;
    setBusy(true);
    setErr(null);
    try {
      await onSave(draft);
      setEditing(false);
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  }
  if (!editing) {
    return (
      <div className={cn("group flex items-center gap-1.5 min-w-0", className)}>
        <span className={cn(size === "lg" ? "break-words min-w-0" : "truncate", text, ink)} title={value}>
          {value}
        </span>
        <button
          type="button"
          onClick={() => {
            setDraft(value);
            setEditing(true);
          }}
          title={t("이름 바꾸기", "Rename")}
          aria-label={t("이름 바꾸기", "Rename")}
          className={cn(
            "shrink-0 w-7 h-7 rounded-full grid place-items-center opacity-50 group-hover:opacity-100 focus-visible:opacity-100",
            "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)] transition",
            muted,
          )}
        >
          <Pencil size={13} />
        </button>
      </div>
    );
  }
  return (
    <div className={cn("min-w-0", className)}>
      <div className="flex items-center gap-1.5">
        <input
          ref={input}
          value={draft}
          maxLength={40}
          placeholder={placeholder}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            // 한글 조합 중의 Enter 는 글자 확정이지 저장이 아니다.
            if (e.nativeEvent.isComposing || e.keyCode === 229) return;
            if (e.key === "Enter") void save();
            if (e.key === "Escape") {
              e.preventDefault();
              setEditing(false);
            }
          }}
          disabled={busy}
          aria-label={t("프로젝트 이름", "Project name")}
          className={cn(
            "min-w-0 flex-1 bg-transparent outline-none rounded-[8px] px-1.5 -mx-1.5",
            "shadow-[inset_0_0_0_1.5px_var(--color-ok)]",
            text,
            ink,
          )}
        />
        <IconButton label={t("저장", "Save")} onClick={() => void save()}>
          <Check size={15} />
        </IconButton>
        <IconButton label={t("취소", "Cancel")} onClick={() => setEditing(false)}>
          <X size={15} />
        </IconButton>
      </div>
      {err && <p className="mt-1 text-sm text-[var(--color-danger)]">{err}</p>}
    </div>
  );
}

/** 사실 한 줄 — 상세 단의 «이름 / 값». */
export function FactRow({ name, children }: { name: string; children: React.ReactNode }) {
  return (
    <div className="py-2">
      <dt className={cn("text-sm", muted)}>{name}</dt>
      <dd className={cn("mt-0.5 text-md break-words", ink)}>{children}</dd>
    </div>
  );
}

// ── 홈에서 옮겨 온 것 — 게이지·흰 단추. 모든 페이지가 같은 걸 쓴다. ──

export function Gauge({
  title,
  total,
  value,
  fill,
  muted,
  warn,
}: {
  title: string;
  total: string;
  value: string;
  fill: number;
  muted?: boolean;
  warn?: boolean;
}) {
  return (
    <div className="min-w-0">
      <div className="flex justify-between text-sm text-[var(--color-ink-500)] mb-2">
        <span>{title}</span>
        <span className="num">{total}</span>
      </div>
      <div
        className={cn(
          "num text-lg font-semibold mb-2.5",
          warn
            ? "text-[var(--color-danger)] text-md"
            : muted
              ? "text-[var(--color-ink-300)]"
              : "text-[var(--color-ink-900)] dark:text-[#E8E5DD]",
        )}
      >
        {value}
      </div>
      <div
        role="progressbar"
        aria-label={title}
        aria-valuemin={0}
        aria-valuemax={100}
        aria-valuenow={Math.round(fill * 100)}
        aria-valuetext={value}
        // 빈 게이지엔 회색 바를 긋지 않는다 — 쓴 만큼만 보라 선이 선다.
        className={cn("h-1 rounded-full overflow-hidden", fill > 0 && "bg-[var(--color-ivory-200)]/70 dark:bg-[var(--color-night-700)]")}
      >
        {fill > 0 && (
          <div
            className="h-full rounded-full bg-[var(--color-ok)] transition-[width] duration-[var(--duration-slow)] ease-[var(--ease-standard)]"
            // 아주 적게 썼어도 «쓰긴 썼다» 가 보이게 최소 6px.
            style={{ width: `${fill * 100}%`, minWidth: 6 }}
          />
        )}
      </div>
    </div>
  );
}

export function GhostButton({ onClick, children, disabled }: { onClick: () => void; children: React.ReactNode; disabled?: boolean }) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={disabled}
      className={cn(
        "disabled:opacity-50 disabled:pointer-events-none",
        "h-9 px-3.5 rounded-[10px] inline-flex items-center gap-1.5 text-md font-medium",
        "bg-white text-[var(--color-ink-700)] shadow-[inset_0_0_0_1px_var(--color-ivory-300)] hover:text-[var(--color-ink-900)]",
        "dark:bg-[var(--color-night-800)] dark:text-[#B5AFA2] dark:shadow-[inset_0_0_0_1px_var(--color-night-600)] dark:hover:text-[#E8E5DD]",
        "transition-colors duration-[var(--duration-fast)]",
      )}
    >
      {children}
    </button>
  );
}
