// 설정 화면의 조각 — 묶음 틀(Section)·줄(ToggleRow·UnitInput)·한도 묶음·언어·정보·화이트리스트(옛 «신뢰 주소») 목록.

import { createContext, useContext, useEffect, useState, type ReactNode } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { ExternalLink, Info, ShieldCheck, Trash2, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { fmtExact, shortenAddress } from "@/lib/format";
import { GITHUB_URL } from "@/lib/helpContent";
import { cardBase, enter, inputBase, LedgerHead, modalCard, modalOverlay, Switch } from "@/components/ui";
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
      <div className="flex items-baseline justify-between">
        <span className="text-md tracking-tight">Budu</span>
        <span className="num font-mono text-sm text-[var(--color-ink-300)]">
          {version ? `v${version}` : "—"}
        </span>
      </div>

      {/* 개발 29에서 현재형으로 복원 — 저장소가 실제로 공개됐다. 다시 비공개로 돌리는 일이
          생기면 이 문장부터 미래형으로 되돌릴 것. 신뢰의 근거로 내세운 문장이라, 사실과
          어긋나면 뒤에 단서를 붙이는 걸로는 못 고친다. */}
      <p className="mt-3 text-md leading-relaxed text-[var(--color-ink-500)]">
        {t(
          "Budu는 소스 코드가 MIT 라이선스로 공개돼 있어요. 열쇠와 돈을 다루는 코드를 직접 읽어 확인하고, 직접 빌드해서 쓸 수 있어요.",
          "Budu's source is public under the MIT license. You can read the code that handles your key and your money, and build it yourself.",
        )}
      </p>
      {/* 저장(로컬)과 전송(RPC)을 나눠 쓴 건 개발 27 판단 그대로. 개발 29 에서 한 겹 더 —
          "내역이 ~/.budu 에만 있다"가 "내 거래가 비공개다"로 읽히면 안 된다. 보낸 거래는
          공개 체인에 영구히 남는다. 지갑에서 이 오해는 값이 비싸다. */}
      <p className="mt-1.5 text-sm leading-snug text-[var(--color-ink-300)]">
        {t(
          <>
            지갑 키·설정·거래 내역은 이 컴퓨터의 <span className="font-mono">~/.budu</span>{" "}
            폴더에만 저장돼요. 잔액을 확인하고 결제를 준비할 때만 위에서 고른 RPC 서버로 요청이 나가요 —
            그쪽은 내 주소를 볼 수 있고, 보낸 거래는 공개 블록체인에 남아요.
          </>,
          <>
            Your key, settings, and history stay in the <span className="font-mono">~/.budu</span>{" "}
            folder on this computer. Requests only leave for the RPC server you picked above, to read
            balances and send payments — that server can see your address, and payments you send stay
            on a public blockchain.
          </>,
        )}
      </p>

      <div className="mt-4 flex items-center justify-between">
        <button
          type="button"
          onClick={() => openUrl(GITHUB_URL).catch(() => {})}
          className="inline-flex items-center gap-1.5 text-sm text-[var(--color-ok)] hover:underline transition-colors"
        >
          <ExternalLink size={12} />
          {t("소스 코드 보기", "Read the source")}
        </button>
        <span className="text-sm text-[var(--color-ink-300)]">{t("MIT 라이선스", "MIT license")}</span>
      </div>
      {/* 글꼴 고지 — 앱에 Pretendard 를 함께 담아 배포하므로 OFL 1.1 이 고지를 요구한다.
          라이선스 전문은 번들 안(fonts/LICENSE-Pretendard.txt)에도 같이 들어간다. */}
      <p className="mt-3 text-sm text-[var(--color-ink-300)]">
        {t("글꼴 Pretendard — SIL Open Font License 1.1", "Pretendard typeface — SIL Open Font License 1.1")}
      </p>
    </Section>
  );
}

/** 화이트리스트 주소 목록 모달 — 주소가 많아져도 설정 화면이 안 길어지게 분리. */
export function TrustedAddrsModal({
  addrs,
  onRevoke,
  onClose,
}: {
  addrs: string[];
  onRevoke: (addr: string) => Promise<void>;
  onClose: () => void;
}) {
  // 철회는 실수 방지로 행별 2단계 확인 (휴지통 → "철회할까요?" → 철회).
  const [confirming, setConfirming] = useState<string | null>(null);

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      onClick={onClose}
      className={modalOverlay}
    >
      <motion.section
        initial={{ scale: 0.9, opacity: 0, y: 12 }}
        animate={{ scale: 1, opacity: 1, y: 0 }}
        exit={{ scale: 0.95, opacity: 0 }}
        transition={{ type: "spring", stiffness: 340, damping: 24 }}
        onClick={(e) => e.stopPropagation()}
        className={cn(modalCard, "px-7 py-6")}
      >
        <div className="flex items-center justify-between">
          <span className="flex items-center gap-1.5 text-sm tracking-[0.04em] text-[var(--color-ok)]">
            <ShieldCheck size={13} />
            {t("화이트리스트", "Whitelist")}
          </span>
          <button
            type="button"
            onClick={onClose}
            aria-label={t("닫기", "Close")}
            className="p-1 text-[var(--color-ink-300)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
          >
            <X size={14} />
          </button>
        </div>
        <p className="mt-3 text-sm leading-relaxed text-[var(--color-ink-500)]">
          {t(
            "비번으로 승인하면 자동으로 학습돼요. 철회하면 다음 결제 때 다시 비번을 받아요.",
            "Approving with your password adds an address here. Remove one and the next payment to it asks again.",
          )}
        </p>

        {addrs.length === 0 ? (
          <p className="mt-5 mb-1 text-sm text-[var(--color-ink-300)]">
            {t(
              "모두 철회했어요. 비번으로 승인하면 다시 학습돼요.",
              "All removed. Approving with your password adds them back.",
            )}
          </p>
        ) : (
          <ul className="mt-4 max-h-72 overflow-y-auto space-y-0.5 -mx-2">
            {addrs.map((addr) => (
              <li
                key={addr}
                className="flex items-center justify-between gap-2 rounded-lg px-2 py-2 hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]/50 transition-colors"
              >
                <span className="num font-mono text-sm text-[var(--color-ink-500)]" title={addr}>
                  {shortenAddress(addr)}
                </span>
                {confirming === addr ? (
                  <span className="flex items-center gap-2 shrink-0">
                    <span className="text-sm text-[var(--color-ink-500)]">{t("철회할까요?", "Remove it?")}</span>
                    <button
                      type="button"
                      onClick={() => {
                        setConfirming(null);
                        void onRevoke(addr);
                      }}
                      className="text-sm font-medium text-[var(--color-danger)] hover:underline"
                    >
                      {t("철회", "Remove")}
                    </button>
                    <button
                      type="button"
                      onClick={() => setConfirming(null)}
                      className="text-sm text-[var(--color-ink-300)] hover:underline"
                    >
                      {t("취소", "Cancel")}
                    </button>
                  </span>
                ) : (
                  <button
                    type="button"
                    onClick={() => setConfirming(addr)}
                    aria-label={t(
                      `${shortenAddress(addr)} 철회`,
                      `Remove ${shortenAddress(addr)}`,
                    )}
                    title={t("화이트리스트에서 빼기", "Remove from the whitelist")}
                    className="shrink-0 p-1 rounded-md text-[var(--color-ink-300)] hover:text-[var(--color-danger-active)] transition-colors"
                  >
                    <Trash2 size={13} />
                  </button>
                )}
              </li>
            ))}
          </ul>
        )}
      </motion.section>
    </motion.div>
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
  desc: string;
  checked: boolean;
  onToggle: () => void;
}) {
  return (
    <div className="flex items-center justify-between px-4 py-3.5">
      <div className="min-w-0 pr-3">
        <p className="text-md tracking-tight">{title}</p>
        <p className="mt-0.5 text-sm leading-snug text-[var(--color-ink-300)]">{desc}</p>
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
        <p
          className={cn(
            "mt-0.5 text-sm leading-snug",
            failed ? "text-[var(--color-danger)]" : "text-[var(--color-ink-300)]",
          )}
        >
          {failed
            ? t("언어를 저장하지 못했어요. 그대로 뒀어요.", "Couldn't save the language, so nothing changed.")
            : dirty
              ? t(
                  "저장 안 한 변경이 있어요. 먼저 저장한 뒤에 바꿔주세요.",
                  "You have unsaved changes. Save them first, then switch.",
                )
              : t("고르면 이 화면이 새 언어로 다시 열려요.", "Picking one reopens this screen in that language.")}
        </p>
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
  usedToday,
}: {
  token: string;
  single: string;
  daily: string;
  onSingle: (v: string) => void;
  onDaily: (v: string) => void;
  usedToday: string | undefined;
}) {
  const used = fmtExact(usedToday); // 정확한 값 — 반올림하면 한도 계산과 어긋나 보인다. ETH 갈래는 개발 6 에 걷어 냄.
  return (
    <div className="rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-4 win:border-0 win:p-0">
      <div className="flex items-center justify-between">
        <span className="text-md tracking-tight">{token}</span>
        <span className="text-sm text-[var(--color-ink-300)] num">
          {/* 정확한 값 — 0.045 를 「0.05 사용」으로 반올림하면 한도 계산과 어긋나 보인다 */}
          {t(`오늘 ${used} 사용`, `${used} used today`)}
        </span>
      </div>
      <div className="mt-3 grid grid-cols-2 gap-3">
        <UnitInput
          label={t("단일 거래 한도", "Per payment")}
          unit={token}
          value={single}
          onChange={onSingle}
        />
        <UnitInput
          label={t("하루 누적 한도", "Per day")}
          unit={token}
          value={daily}
          onChange={onDaily}
        />
      </div>
    </div>
  );
}
