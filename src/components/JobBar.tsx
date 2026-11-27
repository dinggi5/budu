// 작업 예산 바 — 사람이 승인 창에서 연 작업 예산을 메뉴바 팝오버 한 줄로. «오슬로호 입항 준비 · 0.07 / 0.20 USDC».
// 끝내기 = 예산과 메모리의 키를 지금 버린다(그 뒤 같은 작업의 결제는 다시 승인 창으로).

import { motion } from "framer-motion";
import { Briefcase } from "lucide-react";
import { cn } from "@/lib/cn";
import { money } from "@/lib/ledger";
import { useJobNames } from "@/lib/jobNames";
import type { JobView } from "@/lib/types";
import { enter } from "@/components/ui";
import { t } from "@/lib/i18n";

export function JobBar({ job, onEnd }: { job: JobView; onEnd: () => void }) {
  const names = useJobNames();
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)] border border-[var(--color-ok)]",
        "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
      )}
    >
      <Briefcase size={16} className="shrink-0 text-[var(--color-ok)]" />
      <div className="flex-1 min-w-0">
        <p className="text-sm leading-snug tracking-tight text-[var(--color-ink-900)] dark:text-[#E8E5DD] truncate" title={job.job}>
          {names.label(job.job)}
        </p>
        <p className="mt-0.5 text-sm text-[var(--color-ink-300)] num">
          {t(
            `프로젝트 예산 ${money(job.spent)} / ${money(job.budget)} USDC`,
            `Project budget ${money(job.spent)} / ${money(job.budget)} USDC`,
          )}
        </p>
      </div>
      <button
        type="button"
        onClick={onEnd}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-sm tracking-tight",
          "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
          "text-[var(--color-ink-500)] dark:text-[#B5AFA2]",
          "hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        {t("끝내기", "End")}
      </button>
    </motion.div>
  );
}
