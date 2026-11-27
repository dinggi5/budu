// 장부 페이지들이 같이 쓰는 조각 — 결제 줄(누르면 아래로 펼쳐지는 세부)·목록 틀·영수증 내보내기·빈 화면.
// 데이터는 WalletScreen 이 읽는 내역(`get_history`)과 작업 예산(`job_status`)뿐이다. 금액·상태 판정은 lib/ledger.ts 한 곳.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { AnimatePresence, motion } from "framer-motion";
import { AlertTriangle, Briefcase, Check, Copy, ExternalLink, PanelLeft } from "lucide-react";
import { cn } from "@/lib/cn";
import { explorerTxUrl, useChain } from "@/lib/chain";
import { shortenAddress } from "@/lib/format";
import { locale, t } from "@/lib/i18n";
import { useJobNames } from "@/lib/jobNames";
import { dayLabel, entryFacts, hhmm, money, shortReason, type Group } from "@/lib/ledger";
import { useCopy } from "@/lib/useCopy";
import type { HistoryEntry } from "@/lib/types";
import { FactRow, IconButton, Selectable, Tag, faint, hairline, ink, inkSoft, muted, sunken } from "@/components/win";

// ── 공용 ──────────────────────────────────────────────────────────────────────────────

/** 장부의 «어디를 보나» — 모든 결제 · 작업 하나 · 작업 없는 결제. */
export type Scope = "all" | "nojob" | `job:${string}`;

/** 결제 한 줄의 열쇠 — 서명(detail)이 결제마다 다르다. 차단·실패는 사유라 겹칠 수 있어 시각·금액·주소를 같이. */
export function entryKey(e: HistoryEntry): string {
  // 상태는 넣지 않는다 — 보는 중에 «정산 대기» 가 «체인 확인» 으로 바뀌어도 상세가 닫히지 않게. detail 은 서명이라 정산 전후 같다.
  return `${e.ts}|${e.amount}|${e.to}|${e.detail}`;
}

/** 영수증 CSV 내려받기 — 작업 하나. 결과(저장됨·오류)는 부르는 쪽이 한 줄로 보인다. */
export function useReceipt() {
  const [saved, setSaved] = useState<string | null>(null);
  const [err, setErr] = useState<{ job: string; msg: string } | null>(null);
  // 안내는 잠깐만 — 계속 남으면 그 자리의 «긴급 잠금 중» 같은 상태 줄을 영영 가린다.
  // 오류는 다음 시도까지 남긴다 — 긴 경로·권한 메시지를 다 읽기 전에 사라지면 안 된다.
  useEffect(() => {
    if (!saved) return;
    const h = setTimeout(() => setSaved(null), 5000);
    return () => clearTimeout(h);
  }, [saved]);
  async function save(job: string) {
    setErr(null);
    setSaved(null);
    try {
      const path = await invoke<string>("export_job_receipt", { job });
      setSaved(job);
      revealItemInDir(path).catch(() => {});
    } catch (e) {
      setErr({ job, msg: String(e) });
    }
  }
  function note(job: string): React.ReactNode {
    if (err?.job === job) return <span className="text-[var(--color-danger)]">{err.msg}</span>;
    if (saved === job) return t("영수증을 다운로드 폴더에 저장했어요", "Receipt saved to Downloads");
    return null;
  }
  return { save, note };
}

export function Loading() {
  return <p className={cn("text-md", faint)}>{t("불러오는 중…", "Loading…")}</p>;
}

export function Empty({ title, body }: { title: string; body: string }) {
  return (
    <p className={cn("text-md leading-relaxed", muted)}>
      {title}
      <br />
      <span className={cn("text-md", faint)}>{body}</span>
    </p>
  );
}

/** 합계가 잘린 내역 기준일 때 한 줄 — 영수증 CSV 는 전체를 읽으니 화면 합계와 다를 수 있다. */
export function PartialNote({ n, onMore }: { n?: number; onMore?: () => void }) {
  if (!n) return null;
  return (
    <p className={cn("text-sm leading-relaxed", muted)}>
      {t(`최근 ${n}건 기준이에요. 영수증 CSV 는 전체로 셉니다.`, `Based on the latest ${n} records. Receipt CSVs count everything.`)}{" "}
      {onMore && (
        <button type="button" onClick={onMore} className="underline underline-offset-2 hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]">
          {t("더 오래된 기록 읽기", "Load older")}
        </button>
      )}
    </p>
  );
}

export function groupMeta(g: Pick<Group, "rows" | "first" | "last" | "pending">): string {
  const days = dayLabel(g.first) === dayLabel(g.last) ? dayLabel(g.last) : `${dayLabel(g.first)} – ${dayLabel(g.last)}`;
  const pending = g.pending > 0 ? t(`, 대기 ${g.pending}건`, `, ${g.pending} pending`) : "";
  return t(`${g.rows.length}건 ${days}${pending}`, `${g.rows.length} on ${days}${pending}`);
}

// ── 결제 한 줄 · 상세 ─────────────────────────────────────────────────────────────────

/** 결제 한 줄 — 무엇 + 금액 / 두 줄 미리보기 / 꼬리표. 누르면 그 줄 바로 아래에 세부가 펼쳐진다(옆 패널 없음). */
export function PaymentRow({
  entry,
  on,
  onPick,
  jobLabel,
}: {
  entry: HistoryEntry;
  on: boolean;
  onPick: () => void;
  /** «모든 결제» 에서만 — 어느 작업인지 꼬리표 옆에. */
  jobLabel?: string;
}) {
  const f = entryFacts(entry);
  // 미리보기는 짧게 — 사유는 앞머리를 떼고, 주소·포트는 세부에만.
  const preview = f.undelivered || f.tone === "bad" ? shortReason(f.why) : hhmm(entry.ts);
  return (
    <li className={cn("border-b last:border-b-0", hairline)}>
      <Selectable on={on} onClick={onPick} className="px-3 py-3 my-0.5">
        <span className="flex items-baseline justify-between gap-3">
          <span className={cn("min-w-0 truncate text-md font-medium", ink)}>{f.what}</span>
          <span
            className={cn(
              "num shrink-0 text-md",
              f.struck ? "line-through text-[var(--color-ink-300)]" : f.tone === "bad" ? "text-[var(--color-danger)]" : ink,
            )}
          >
            {money(entry.amount)}
          </span>
        </span>
        <span
          className={cn(
            "mt-0.5 block text-sm leading-snug line-clamp-2 break-words",
            f.undelivered || f.tone === "bad" ? "text-[var(--color-danger)]" : muted,
          )}
        >
          {f.undelivered && <AlertTriangle size={11} className="inline -mt-0.5 mr-1" />}
          {preview}
        </span>
        <span className="mt-1.5 flex items-center gap-1.5 min-w-0">
          <Tag tone={f.tone}>{f.state}</Tag>
          {jobLabel && <span className={cn("truncate text-sm", faint)}>{jobLabel}</span>}
        </span>
      </Selectable>
      <AnimatePresence initial={false}>
        {on && (
          <motion.div
            key="detail"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }}
            className="overflow-hidden"
          >
            <PaymentDetail entry={entry} />
          </motion.div>
        )}
      </AnimatePresence>
    </li>
  );
}

/** 줄 아래에 펼쳐지는 세부 — 고른 결제 하나의 모든 사실. 금액·무엇·상태는 위 줄에 이미 있어 되풀이하지 않는다. */
export function PaymentDetail({ entry }: { entry: HistoryEntry }) {
  const chain = useChain();
  const names = useJobNames();
  const f = entryFacts(entry);
  const [addrCopied, copyAddr] = useCopy();
  const [txCopied, copyTx] = useCopy();
  const reason = entry.status === "blocked" || entry.status === "failed" ? entry.detail : "";
  const job = (entry.job ?? "").trim();
  const full = new Date(entry.ts * 1000).toLocaleString(locale(), {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
  return (
    <div className="px-3 pb-4 pt-1" aria-label={t("결제 상세", "Payment details")}>
      {(f.undelivered || reason) && (
        <p className="mb-2 rounded-[10px] bg-[var(--color-danger-tint)] px-3 py-2.5 text-sm leading-relaxed text-[var(--color-danger)]">
          {f.undelivered ? f.why : reason}
        </p>
      )}
      <dl className={cn("rounded-[12px] px-4 py-2", sunken)}>
        <FactRow name={t("시각", "Time")}>
          <span className="num">{full}</span>
        </FactRow>
        {entry.resource && (
          <FactRow name={t("결제 대상", "Paid for")}>
            <span className="font-mono text-sm break-all">{entry.resource}</span>
          </FactRow>
        )}
        <FactRow name={t("받는 주소", "Paid to")}>
          <span className="flex items-center gap-1">
            <span className="font-mono text-sm break-all">{entry.to}</span>
            <IconButton label={t("주소 복사", "Copy address")} onClick={() => copyAddr(entry.to)}>
              {addrCopied ? <Check size={13} className="text-[var(--color-ok)]" /> : <Copy size={13} />}
            </IconButton>
          </span>
        </FactRow>
        {job && (
          <FactRow name={t("작업", "Job")}>
            {names.label(job)}
            {names.renamed(job) && <span className={cn("block text-sm", muted)}>{t(`AI 이름표 ${job}`, `AI label ${job}`)}</span>}
          </FactRow>
        )}
        {entry.settle_tx && (
          <FactRow name={t("정산 거래", "Settlement tx")}>
            <span className="flex items-center gap-1">
              <span className="font-mono text-sm">{shortenAddress(entry.settle_tx)}</span>
              <IconButton label={t("거래 복사", "Copy tx")} onClick={() => copyTx(entry.settle_tx!)}>
                {txCopied ? <Check size={13} className="text-[var(--color-ok)]" /> : <Copy size={13} />}
              </IconButton>
              {f.link && (
                <IconButton label={t("탐색기에서 보기", "View in explorer")} onClick={() => void openUrl(explorerTxUrl(chain, f.link)).catch(() => {})}>
                  <ExternalLink size={13} />
                </IconButton>
              )}
            </span>
          </FactRow>
        )}
        <FactRow name={t("확인", "Checked")}>
          {entry.checked
            ? t("체인에서 확인했어요", "Confirmed on-chain")
            : entry.status === "settled"
              ? t("판매자 보고뿐이에요. 체인 확인을 기다려요", "Seller's word only, waiting for the chain")
              : t("해당 없음", "n/a")}
        </FactRow>
      </dl>
    </div>
  );
}

/** 결제 목록 — 줄을 누르면 그 아래로 세부가 펼쳐진다(한 번에 하나, 다시 누르거나 Esc 로 접힘). */
export function PaymentList({
  head,
  entries,
  groupDays,
  showJob,
  footer,
}: {
  head: React.ReactNode;
  entries: HistoryEntry[] | null;
  groupDays?: boolean;
  showJob?: boolean;
  footer?: React.ReactNode;
}) {
  const names = useJobNames();
  const [picked, setPicked] = useState<string | null>(null);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // 입력 칸의 Esc(이름 고치기 취소)나 대화 상자(승인 시트)가 떠 있을 때는 펼친 세부를 건드리지 않는다.
      if (e.key !== "Escape" || e.defaultPrevented) return;
      const el = e.target as HTMLElement | null;
      if (el && (el.tagName === "INPUT" || el.tagName === "TEXTAREA" || el.tagName === "SELECT")) return;
      if (document.querySelector('[role="dialog"],[role="alertdialog"],[aria-modal="true"]')) return;
      setPicked(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  return (
    <section className="flex-1 min-w-0 h-full overflow-y-auto px-6 min-[1100px]:px-10 pt-14 pb-10">
      <div className="mx-auto max-w-[760px]">
        {head}
        <div className="mt-7">
          {entries == null ? (
            <Loading />
          ) : entries.length === 0 ? (
            <Empty
              title={t("아직 결제가 없어요.", "No payments yet.")}
              body={t("AI 가 한 결제와 막힌 시도가 여기 쌓여요.", "Payments the AI makes and blocked attempts land here.")}
            />
          ) : (
            <ul className="-mx-3">
              {entries.map((e, i) => {
                const newDay = groupDays && (i === 0 || dayLabel(entries[i - 1].ts) !== dayLabel(e.ts));
                const k = entryKey(e);
                const job = (e.job ?? "").trim();
                return (
                  <FragmentWithDay key={`${k}-${i}`} day={newDay ? dayLabel(e.ts) : null} first={i === 0}>
                    <PaymentRow
                      entry={e}
                      on={picked === k}
                      onPick={() => setPicked(picked === k ? null : k)}
                      jobLabel={showJob && job ? names.label(job) : undefined}
                    />
                  </FragmentWithDay>
                );
              })}
            </ul>
          )}
          {footer && <div className="mt-6">{footer}</div>}
        </div>
      </div>
    </section>
  );
}

export function FragmentWithDay({ day, first, children }: { day: string | null; first: boolean; children: React.ReactNode }) {
  return (
    <>
      {day && (
        <li aria-hidden className={cn("px-3 pb-1 text-sm font-medium", muted, first ? "pt-0" : "pt-6")}>
          {day}
        </li>
      )}
      {children}
    </>
  );
}

export function LibraryItem({
  on,
  onPick,
  name,
  meta,
  accent,
}: {
  on: boolean;
  onPick: () => void;
  name: string;
  meta: string;
  accent?: boolean;
}) {
  return (
    <li>
      <Selectable on={on} onClick={onPick} className="px-3 py-2">
        <span className="flex items-center gap-2 min-w-0">
          {accent && <Briefcase size={13} className="shrink-0 text-[var(--color-ok)]" />}
          <span className={cn("truncate text-md", on ? cn(ink, "font-semibold") : inkSoft)} title={name}>
            {name}
          </span>
        </span>
        <span className={cn("block text-sm num truncate", muted)}>{meta}</span>
      </Selectable>
    </li>
  );
}

/** 왼쪽 단의 틀 — 가라앉은 판, 위 56px 는 신호등 아래 끌기 띠. */
export function Library({ children, onCollapse }: { children: React.ReactNode; onCollapse: () => void }) {
  return (
    <nav className={cn("h-full w-[228px] shrink-0 overflow-y-auto px-3 pt-12 pb-6", sunken)}>
      <div className="flex justify-end px-1 mb-1">
        <IconButton label={t("목록 접기", "Hide list")} onClick={onCollapse}>
          <PanelLeft size={15} />
        </IconButton>
      </div>
      {children}
    </nav>
  );
}

export function Fact({ title, value }: { title: string; value: string }) {
  return (
    <div className="min-w-0">
      <div className={cn("text-sm mb-2", muted)}>{title}</div>
      <div className={cn("num text-lg font-semibold", ink)}>{value}</div>
    </div>
  );
}
