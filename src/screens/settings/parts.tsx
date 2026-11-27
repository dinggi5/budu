// 설정 화면의 조각 — 묶음 틀(Section)·줄(ToggleRow·UnitInput)·한도 묶음·언어·정보·화이트리스트(옛 «신뢰 주소») 목록.

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { ChevronDown, ExternalLink, Info, Trash2 } from "lucide-react";
import { cn } from "@/lib/cn";
import { shortenAddress } from "@/lib/format";
import { GITHUB_URL } from "@/lib/helpContent";
import { cardBase, enter, inputBase, LedgerHead, Switch } from "@/components/ui";
import { isWindow } from "@/lib/win";
import { chooseLang, lang, t, type Lang } from "@/lib/i18n";

/** 큰 창에서 왼쪽 목록이 고른 묶음 이름(팝오버는 null = 전부). */
export const NavCtx = createContext<string | null>(null);

/** 설정 화면의 그룹 섹션 — 아이콘+제목 헤더를 단 카드. 길게 쌓이던 설정을 도메인별로 끊어 준다.
 *  delay = 카드별 미세 스태거(코다와리 — 위에서부터 차례로 떠오르게). */
export function Section({
  icon,
  title,
  delay = 0,
  children,
}: {
  icon: ReactNode;
  title: string;
  delay?: number;
  children: ReactNode;
}) {
  const sel = useContext(NavCtx);
  // 큰 창 — 카드 없이 제목과 여백만. 왼쪽 목록에서 고른 묶음만 그린다.
  if (isWindow) {
    if (sel && sel !== title) return null;
    return (
      // 넓으면 fieldset 이 두 단으로 흐른다 — 묶음 하나가 두 단에 걸쳐 쪼개지지 않게.
      <motion.section {...enter} transition={{ ...enter.transition, delay: 0 }} className="w-full mb-8">
        <LedgerHead title={title} />
        <div className="pt-3">{children}</div>
      </motion.section>
    );
  }
  return (
    <motion.section
      {...enter}
      transition={{ ...enter.transition, delay }}
      className={cn(cardBase, "px-6 py-5")}
    >
      <div className="flex items-center gap-1.5">
        {icon}
        <span className="text-md tracking-tight">{title}</span>
      </div>
      <div className="mt-3">{children}</div>
    </motion.section>
  );
}

/** 정보 섹션 — 버전 + "직접 감사할 수 있다"는 신뢰 프레이밍 + 소스 링크.
 *  키를 맡기는 앱이라 "믿어달라"가 아니라 "직접 읽어보라"가 유일하게 정직한 근거 —
 *  그래서 묻힌 링크가 아니라 설정 하단의 제 카드로 둔다. */
export function AboutSection() {
  // 버전은 tauri.conf.json(=설치된 앱 번들)이 진실 원천. 실패해도 섹션 자체는 뜬다.
  const [version, setVersion] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    getVersion()
      .then((v) => alive && setVersion(v))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  return (
    <Section icon={<Info size={13} className="text-[var(--color-ok)]" />} title={t("정보", "About")} delay={0.2}>
      <p className="text-md tracking-tight">Budu</p>
      <p className="mt-0.5 num font-mono text-sm text-[var(--color-ink-300)]">{version ? `v${version}` : "—"}</p>

      {/* 저장소가 공개라 쓸 수 있는 문장이다 — 다시 비공개로 돌리면 이 문장부터 고친다. */}
      <p className="mt-3 text-md leading-relaxed text-[var(--color-ink-500)]">
        {t("Budu는 코드가 오픈 소스로 공개되어 있습니다.", "Budu's code is open source.")}
      </p>
      <p className="mt-1 text-md leading-relaxed text-[var(--color-ink-500)]">
        {t(
          <>
            지갑 키·설정·거래 내역은 컴퓨터의 <span className="font-mono">~/.budu</span> 폴더에 저장됩니다.
          </>,
          <>
            Your wallet key, settings, and history are stored in the <span className="font-mono">~/.budu</span> folder on
            your computer.
          </>,
        )}
      </p>

      <div className="mt-4 flex flex-col items-start gap-1.5 text-sm">
        <button
          type="button"
          onClick={() => openUrl(GITHUB_URL).catch(() => {})}
          className="inline-flex items-center gap-1.5 text-[var(--color-ink-700)] dark:text-[#E8E5DD] hover:underline"
        >
          <ExternalLink size={12} />
          {t("소스 코드 보기", "View source code")}
        </button>
        <span className="text-[var(--color-ink-300)]">{t("MIT 라이선스", "MIT License")}</span>
        {/* 앱에 Pretendard 를 함께 담아 배포하므로 OFL 1.1 이 고지를 요구한다. */}
        <span className="text-[var(--color-ink-300)]">
          {t("글꼴 Pretendard — SIL Open Font License 1.1", "Font Pretendard — SIL Open Font License 1.1")}
        </span>
      </div>
    </Section>
  );
}

/** 화이트리스트 — 한 줄(«아직 없어요» / «N개»)을 누르면 아래로 목록이 펼쳐진다(결제 내역 줄 펼치기와 같은 방식).
 *  안에서 주소를 직접 넣고 뺀다(즉시 저장). 자동 학습 옵션은 다른 토글처럼 «저장하기» 로 적용된다. */
export function TrustedList({
  addrs,
  onAdd,
  onRevoke,
  autoLearn,
  onToggleAutoLearn,
}: {
  addrs: string[] | null;
  onAdd: (addr: string) => Promise<string | null>;
  onRevoke: (addr: string) => Promise<void>;
  autoLearn: boolean;
  onToggleAutoLearn: () => void;
}) {
  const [open, setOpen] = useState(false);
  // 빼기는 실수 방지로 행별 2단계 확인.
  const [confirming, setConfirming] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [err, setErr] = useState<string | null>(null);
  const [adding, setAdding] = useState(false);

  async function add() {
    if (!draft.trim() || adding) return;
    setAdding(true);
    const e = await onAdd(draft);
    setAdding(false);
    setErr(e);
    if (!e) setDraft("");
  }

  const summary =
    addrs === null ? t("불러오는 중…", "Loading…") : addrs.length === 0 ? t("아직 없어요", "None yet") : t(`${addrs.length}개`, `${addrs.length}`);

  return (
    <RowGroup>
      <button
        type="button"
        onClick={() => setOpen((v) => !v)}
        aria-expanded={open}
        className="w-full flex items-center justify-between px-4 py-3.5 text-left"
      >
        <span className="text-md tracking-tight">{t("화이트리스트", "Whitelist")}</span>
        <span className="flex items-center gap-1.5 text-sm text-[var(--color-ink-300)]">
          {summary}
          <ChevronDown size={14} className={cn("transition-transform duration-[var(--duration-base)]", open && "rotate-180")} />
        </span>
      </button>
      {open && (
        <div className="px-4 pb-4 win:pt-0">
          {addrs && addrs.length > 0 && (
            <ul className="-mx-2 mb-3 space-y-0.5">
              {addrs.map((addr) => (
                <li key={addr} className="flex items-center justify-between gap-2 rounded-lg px-2 py-1.5">
                  <span className="num font-mono text-sm text-[var(--color-ink-500)]" title={addr}>
                    {shortenAddress(addr)}
                  </span>
                  {confirming === addr ? (
                    <span className="flex items-center gap-2 shrink-0 text-sm">
                      <button
                        type="button"
                        onClick={() => {
                          setConfirming(null);
                          void onRevoke(addr);
                        }}
                        className="font-medium text-[var(--color-danger)] hover:underline"
                      >
                        {t("빼기", "Remove")}
                      </button>
                      <button type="button" onClick={() => setConfirming(null)} className="text-[var(--color-ink-300)] hover:underline">
                        {t("취소", "Cancel")}
                      </button>
                    </span>
                  ) : (
                    <button
                      type="button"
                      onClick={() => setConfirming(addr)}
                      aria-label={t(`${shortenAddress(addr)} 빼기`, `Remove ${shortenAddress(addr)}`)}
                      className="shrink-0 p-1 rounded-md text-[var(--color-ink-300)] hover:text-[var(--color-danger)] transition-colors"
                    >
                      <Trash2 size={13} />
                    </button>
                  )}
                </li>
              ))}
            </ul>
          )}
          <div className="flex gap-2">
            <input
              value={draft}
              onChange={(e) => {
                setDraft(e.target.value);
                setErr(null);
              }}
              onKeyDown={(e) => {
                if (e.key === "Enter") {
                  e.preventDefault();
                  void add();
                }
              }}
              placeholder={t("솔라나 주소", "Solana address")}
              spellCheck={false}
              autoCapitalize="off"
              className={cn(inputBase, "flex-1 min-w-0 font-mono text-sm")}
            />
            <button
              type="button"
              onClick={() => void add()}
              disabled={!draft.trim() || adding}
              className="shrink-0 px-3 text-sm text-[var(--color-ink-700)] dark:text-[#E8E5DD] disabled:text-[var(--color-ink-300)] hover:underline"
            >
              {t("추가", "Add")}
            </button>
          </div>
          {err && <p className="mt-1.5 text-sm text-[var(--color-danger)]">{err}</p>}
          <div className="mt-3 flex items-center justify-between">
            <p className="text-md tracking-tight pr-3">
              {t("결제 승인 시 자동으로 화이트리스트에 추가", "Add to the whitelist automatically when I approve a payment")}
            </p>
            <Switch
              checked={autoLearn}
              onToggle={onToggleAutoLearn}
              label={t("결제 승인 시 자동으로 화이트리스트에 추가", "Add to the whitelist automatically when I approve a payment")}
            />
          </div>
        </div>
      )}
    </RowGroup>
  );
}

/** 자동 잠금 — 목록에서 고른다. 목록 밖의 옛 값(1~9분)은 그 값을 한 칸 더 보여 준다. */
const LOCK_CHOICES: { mins: string; ko: string; en: string }[] = [
  { mins: "10", ko: "10분", en: "10 min" },
  { mins: "30", ko: "30분", en: "30 min" },
  { mins: "60", ko: "1시간", en: "1 hour" },
  { mins: "240", ko: "4시간", en: "4 hours" },
  { mins: "1440", ko: "1일", en: "1 day" },
  { mins: "10080", ko: "7일", en: "7 days" },
  { mins: "0", ko: "잠그지 않음", en: "Never" },
];

export function LockRow({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const v = value.trim();
  const known = LOCK_CHOICES.some((c) => c.mins === v);
  return (
    <div className="flex items-center justify-between px-4 py-3.5">
      <p className="text-md tracking-tight pr-3">{t("자동 잠금", "Auto-lock")}</p>
      <select
        value={v}
        onChange={(e) => onChange(e.target.value)}
        aria-label={t("자동 잠금", "Auto-lock")}
        className={cn(inputBase, "w-auto text-md py-1.5")}
      >
        {!known && <option value={v}>{t(`${v}분`, `${v} min`)}</option>}
        {LOCK_CHOICES.map((c) => (
          <option key={c.mins} value={c.mins}>
            {t(c.ko, c.en)}
          </option>
        ))}
      </select>
    </div>
  );
}

/** 관련 행들을 하나의 카드로 묶는 구분선 리스트.
 *  행마다 테두리 상자를 두르지 않고, 컨테이너 하나 + 헤어라인 구분선으로 접는다. */
export function RowGroup({ children }: { children: ReactNode }) {
  return (
    <div
      className={cn(
        "mt-4 rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]",
        "divide-y divide-[var(--color-ivory-300)] dark:divide-[var(--color-night-700)]",
        // 큰 창 — 상자도 줄 사이 선도 걷는다. 줄 안쪽 여백은 왼쪽 선에 맞춘다.
        "win:rounded-none win:border-0 win:divide-y-0 win:[&>*]:px-0 win:[&>*]:py-3",
      )}
    >
      {children}
    </div>
  );
}

/** 설정 화면 공용 on/off 토글 행 — RowGroup 안에서 쓴다(테두리는 그룹이 두른다). */
export function ToggleRow({
  title,
  desc,
  checked,
  onToggle,
}: {
  title: string;
  desc?: string;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center justify-between px-4 py-3.5">
      <div className="min-w-0 pr-3">
        <p className="text-md tracking-tight">{title}</p>
        {desc && <p className="mt-0.5 text-sm leading-snug text-[var(--color-ink-300)]">{desc}</p>}
      </div>
      <Switch checked={checked} onToggle={onToggle} label={title} />
    </div>
  );
}

/** 언어 선택 행 — 고른 즉시 저장하고 창을 새 언어로 다시 읽는다.
 *
 *  "저장하고 닫기"에 묶지 않는다: 언어는 폼 값이 아니라 앱 관리 필드고(자동 시작·업데이트
 *  확인과 같은 결), 저장 버튼을 누르기 전까지 옛 언어로 남아 있으면 뭘 고른 건지 알 수 없다.
 *  저장이 실패하면 언어를 바꾸지 않고 그 자리에서 말해 준다 — 화면만 바뀌고 다음 실행에
 *  되돌아오는 게 제일 나쁘다. */
export function LanguageRow({ dirty }: { dirty: boolean }) {
  const [busy, setBusy] = useState(false);
  const [failed, setFailed] = useState(false);
  const current = lang();

  function pick(next: Lang) {
    if (next === current || busy || dirty) return;
    setBusy(true);
    setFailed(false);
    // 성공하면 창을 다시 읽으므로 이 컴포넌트는 그대로 사라진다(busy 를 되돌릴 필요 없음).
    chooseLang(next, "settings:app").catch(() => {
      setBusy(false);
      setFailed(true);
    });
  }

  return (
    <div className="flex items-center justify-between px-4 py-3.5">
      <div className="min-w-0 pr-3">
        <p className="text-md tracking-tight">{t("언어", "Language")}</p>
        {(failed || dirty) && (
          <p
            className={cn(
              "mt-0.5 text-sm leading-snug",
              failed ? "text-[var(--color-danger)]" : "text-[var(--color-ink-300)]",
            )}
          >
            {failed
              ? t("언어를 저장하지 못했어요. 그대로 뒀어요.", "Couldn't save the language, so nothing changed.")
              : t("저장 안 한 변경이 있어요. 먼저 저장한 뒤에 바꿔주세요.", "You have unsaved changes. Save them first, then switch.")}
          </p>
        )}
      </div>
      <div className="shrink-0 flex gap-1 p-1 rounded-[var(--radius-pill)] bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-900)]">
        {(
          [
            { code: "ko", label: "한국어" },
            { code: "en", label: "English" },
          ] as { code: Lang; label: string }[]
        ).map((o) => (
          <button
            key={o.code}
            type="button"
            onClick={() => pick(o.code)}
            aria-pressed={o.code === current}
            disabled={dirty}
            className={cn(
              "h-7 px-3 rounded-[var(--radius-pill)] text-sm tracking-tight",
              "transition-colors duration-[var(--duration-base)]",
              "disabled:opacity-40 disabled:cursor-not-allowed",
              o.code === current
                ? "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-700)] text-[var(--color-ink-900)] dark:text-[#E8E5DD] shadow-[var(--shadow-soft)]"
                : "text-[var(--color-ink-500)] hover:text-[var(--color-ink-700)]",
            )}
          >
            {o.label}
          </button>
        ))}
      </div>
    </div>
  );
}

/** 단위가 붙는 숫자 입력 — 단위를 필드 안 오른쪽에 고정해 표기·정렬을 통일한다.
 *  전엔 라벨에 "(USDC)" "(분)" 처럼 괄호로 섞여 붙어 있었다. */
export function UnitInput({
  label,
  unit,
  value,
  onChange,
  intOnly = false,
}: {
  label: string;
  unit: string;
  value: string;
  onChange: (v: string) => void;
  intOnly?: boolean;
}) {
  return (
    <label className="block">
      <span className="text-sm text-[var(--color-ink-500)]">{label}</span>
      <span className="relative block mt-1.5">
        <input
          value={value}
          onChange={(e) => onChange(e.target.value)}
          inputMode={intOnly ? "numeric" : "decimal"}
          className={cn(inputBase, "num text-md pr-14")}
        />
        <span
          aria-hidden
          className="pointer-events-none absolute right-3.5 top-1/2 -translate-y-1/2 text-sm text-[var(--color-ink-300)]"
        >
          {unit}
        </span>
      </span>
    </label>
  );
}

export function LimitGroup({
  token,
  single,
  daily,
  onSingle,
  onDaily,
}: {
  token: string;
  single: string;
  daily: string;
  onSingle: (v: string) => void;
  onDaily: (v: string) => void;
}) {
  return (
    <div className="grid grid-cols-2 gap-3">
      <UnitInput label={t("결제당 한도", "Per payment")} unit={token} value={single} onChange={onSingle} />
      <UnitInput label={t("하루 한도", "Per day")} unit={token} value={daily} onChange={onDaily} />
    </div>
  );
}
