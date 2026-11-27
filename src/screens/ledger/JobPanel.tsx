// 오른쪽 작업 패널 — 열린 작업 + 일지. 홈에만 있다.

import { useState } from "react";
import { AnimatePresence, motion } from "framer-motion";
import { ChevronDown, Download } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { renameJob, useJobNames } from "@/lib/jobNames";
import { entryFacts, journalLine, money, when } from "@/lib/ledger";
import type { HistoryEntry, JobView } from "@/lib/types";
import { GhostButton, RenameField, faint, ink, inkSoft, muted, sunken, usePref } from "@/components/win";
import { useReceipt } from "./parts";

// ── 오른쪽 작업 패널 ──────────────────────────────────────────────────────────────────

/** 일지에 그리는 줄 수 — 판 높이(720 창)에 넘치지 않을 만큼. 나머지는 레일의 장부에서. */
const JOURNAL_ROWS = 7;

/** 오른쪽 패널 — 열린 작업 + 일지. 홈에만 있고, 닫고 여는 단추는 홈(WindowHome)이 쥔다. */
export function JobPanel({
  job,
  locked,
  entries,
  onEnd,
  onOpenJob,
  className,
}: {
  job: JobView | null;
  locked: boolean;
  entries: HistoryEntry[] | null;
  onEnd: () => void;
  /** 작업 이름을 누르면 장부의 그 작업으로. */
  onOpenJob: (label: string) => void;
  className?: string;
}) {
  const names = useJobNames();
  const receipt = useReceipt();
  const rows = (entries ?? []).slice(0, JOURNAL_ROWS);
  const [journal, setJournal] = usePref<"open" | "closed">("journal", "open");
  const mins = job ? Math.max(0, Math.ceil(job.secs_left / 60)) : 0;
  return (
    <aside className={cn("min-w-0 px-6 pt-14 pb-6 overflow-y-auto", sunken, className)} aria-label={t("작업과 일지", "Job and journal")}>
      <div className="flex items-center justify-between mb-3 min-h-8">
        <h2 className={cn("text-[13px] font-semibold", inkSoft)}>{t("진행 중인 작업", "Job in progress")}</h2>
      </div>
      {job ? (
        <div>
          <RenameField size="sm" key={job.job} value={names.label(job.job)} placeholder={job.job} onSave={(n) => renameJob(job.job, n)} />
          <p className={cn("mt-1 text-[12px] num", muted)}>
            {receipt.note(job.job) ??
              (locked
                ? t("긴급 잠금 중, 결제 멈춤", "Emergency lock on, payments paused")
                : t(`${mins}분 남음 · 판매자 ${job.sellers}곳`, `${mins} min left · ${job.sellers} seller(s)`))}
          </p>
          <div className="mt-4 flex flex-wrap gap-2">
            <GhostButton onClick={() => onOpenJob(job.job)}>{t("결제 보기", "Payments")}</GhostButton>
            <GhostButton onClick={() => void receipt.save(job.job)}>
              <Download size={13} />
              CSV
            </GhostButton>
            <GhostButton onClick={onEnd}>{t("끝내기", "End")}</GhostButton>
          </div>
        </div>
      ) : (
        <p className={cn("text-[13px] leading-relaxed", muted)}>
          {locked ? t("긴급 잠금 중이에요.", "Emergency lock is on.") : t("진행 중인 작업이 없어요.", "No job in progress.")}
        </p>
      )}

      {/* 일지 — 아래로 열고 닫는다. 접은 상태는 기억한다. */}
      <button
        type="button"
        onClick={() => setJournal(journal === "open" ? "closed" : "open")}
        aria-expanded={journal === "open"}
        className={cn("mt-10 mb-[18px] w-full flex items-center justify-between text-[13px] font-semibold", inkSoft)}
      >
        <span>{t("일지", "Journal")}</span>
        <ChevronDown
          size={15}
          className={cn("transition-transform duration-[var(--duration-fast)]", muted, journal === "closed" && "-rotate-90")}
          aria-hidden
        />
      </button>
      <AnimatePresence initial={false}>
        {journal === "open" && (
          <motion.div
            key="journal"
            initial={{ height: 0, opacity: 0 }}
            animate={{ height: "auto", opacity: 1 }}
            exit={{ height: 0, opacity: 0 }}
            transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }}
            className="overflow-hidden"
          >
            {entries == null ? (
              <p className={cn("text-[13px]", faint)}>{t("불러오는 중…", "Loading…")}</p>
            ) : rows.length === 0 ? (
              <p className={cn("text-[13px] leading-relaxed", muted)}>{t("아직 결제가 없어요.", "No payments yet.")}</p>
            ) : (
              <ol>
                {rows.map((e, i) => (
                  <JournalRow key={`${e.ts}-${i}`} e={e} />
                ))}
              </ol>
            )}
          </motion.div>
        )}
      </AnimatePresence>
    </aside>
  );
}

/** 일지 한 줄 — 점 · 시각·상태 / «조석 정보를 받았어요» + 금액. 누르면 세부 정보(결제 대상 주소·사유·tx)가 열린다.
 *  명령어(`/tide`)·localhost 포트·주소는 여기서만 보인다. */
function JournalRow({ e }: { e: HistoryEntry }) {
  const [open, setOpen] = useState(false);
  const f = entryFacts(e);
  const reason = e.status === "blocked" || e.status === "failed" ? e.detail : f.undelivered ? f.why : "";
  return (
    <li className="pb-4">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        title={open ? t("세부 정보 닫기", "Hide details") : t("세부 정보 보기", "Show details")}
        className="relative w-full text-left pl-[22px] rounded-[8px]"
      >
        <span
          className={cn(
            "absolute left-0 top-1 w-[9px] h-[9px] rounded-full bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)] ring-2 ring-inset",
            f.tone === "ok" && "ring-emerald-500",
            f.tone === "bad" && "ring-red-500",
            f.tone === "idle" && "ring-[var(--color-ink-300)]",
          )}
          aria-hidden
        />
        <span className={cn("block text-[12px] num", muted)}>
          {when(e.ts)} · {f.state}
        </span>
        <span className={cn("mt-0.5 flex justify-between gap-3 text-[14px] font-medium", ink)}>
          <span className="min-w-0 truncate">{journalLine(e)}</span>
          <span className={cn("num shrink-0", f.tone === "bad" && !f.struck && "text-red-500", f.struck && "line-through text-[var(--color-ink-300)]")}>
            {money(e.amount)}
          </span>
        </span>
        {/* «다시 내지 마세요» 같은 경고는 누르지 않아도 보인다. */}
        {f.undelivered && !open && <span className="mt-0.5 block text-[12px] text-red-500/90">{f.why}</span>}
      </button>
      {open && (
        <dl className={cn("mt-2 ml-[22px] flex flex-col gap-1.5 text-[12px] leading-relaxed break-all", muted)}>
          {reason && <dd className={f.tone === "bad" ? "text-red-500/90" : ""}>{reason}</dd>}
          <dd className="font-mono text-[11px]">{e.resource || e.to}</dd>
          {e.settle_tx && <dd className="font-mono text-[11px]">tx {e.settle_tx}</dd>}
        </dl>
      )}
    </li>
  );
}
