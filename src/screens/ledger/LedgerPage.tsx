// 큰 창의 «작업 내역» — 2단: 왼쪽 = 작업 목록(한눈에 보기 · 진행 중인 작업 · 지난 작업 · 작업 없음) /
// 오른쪽 = 고른 범위의 결제. 결제를 누르면 그 줄 아래로 세부가 펼쳐진다(옆에 셋째 단을 만들지 않는다).
// 고른 항목은 회색 둥근 면, 목록 사이 선은 아주 옅게, 묶음 머리엔 선이 없다.

import { useEffect, useMemo } from "react";
import { Download, PanelLeft } from "lucide-react";
import { cn } from "@/lib/cn";
import { t } from "@/lib/i18n";
import { renameJob, useJobNames } from "@/lib/jobNames";
import { fromUnits, groupEntries, money, ratio, type Group } from "@/lib/ledger";
import type { HistoryEntry, JobView } from "@/lib/types";
import { Eyebrow, Gauge, GhostButton, IconButton, PageTitle, RenameField, faint, ink, muted, usePref, useWidth } from "@/components/win";
import { Fact, Library, LibraryItem, PartialNote, PaymentList, groupMeta, useReceipt, type Scope } from "./parts";

// ── 왼쪽 단 ───────────────────────────────────────────────────────────────────────────



// ── 내역(장부) ─────────────────────────────────────────────────────────────────────────

export function LedgerPage({
  entries,
  job,
  locked,
  scope,
  onScope,
  onEnd,
  onMore,
  partial,
}: {
  entries: HistoryEntry[] | null;
  job: JobView | null;
  locked: boolean;
  scope: Scope;
  onScope: (s: Scope) => void;
  onEnd: () => void;
  onMore?: () => void;
  partial?: number;
}) {
  const names = useJobNames();
  const receipt = useReceipt();
  const [page, pageW] = useWidth<HTMLDivElement>();
  const wide = pageW >= 690;
  const [lib, setLib] = usePref<"open" | "closed">("ledger.lib", "open");
  const showLib = wide && lib === "open";

  const groups = useMemo(() => (entries ? groupEntries(entries) : []), [entries]);
  const jobGroups = groups.filter((g) => g.job);
  const noJob = useMemo(() => (entries ?? []).filter((e) => !(e.job ?? "").trim()).sort((a, b) => b.ts - a.ts), [entries]);
  const all = useMemo(() => (entries ? [...entries].sort((a, b) => b.ts - a.ts) : null), [entries]);
  const openLabel = job?.job ?? null;
  const past = jobGroups.filter((g) => g.job !== openLabel);
  const paidAll = groups.reduce((s, g) => s + g.paid, 0n);
  const paidNoJob = groups.filter((g) => !g.job).reduce((s, g) => s + g.paid, 0n);

  // 고른 작업이 사라졌으면(끝내고 기록에도 없음) «모든 결제» 로.
  const scopeJob = scope.startsWith("job:") ? scope.slice(4) : null;
  const scopeLost = scopeJob != null && scopeJob !== openLabel && !jobGroups.some((g) => g.job === scopeJob) && entries != null;
  useEffect(() => {
    if (scopeLost) onScope("all");
  }, [scopeLost, onScope]);

  const listEntries: HistoryEntry[] | null =
    entries == null ? null : scope === "all" ? all : scope === "nojob" ? noJob : (jobGroups.find((g) => g.job === scopeJob)?.rows ?? []);

  const jobMeta = (g?: Group) => (g ? t(`${g.rows.length}건 · ${fromUnits(g.paid)} USDC`, `${g.rows.length} · ${fromUnits(g.paid)} USDC`) : t("아직 결제 없음", "No payments yet"));

  const library = (
    <Library onCollapse={() => setLib("closed")}>
      <ul className="flex flex-col gap-0.5">
        <LibraryItem
          on={scope === "all"}
          onPick={() => onScope("all")}
          name={t("한눈에 보기", "Overview")}
          meta={entries ? t(`${entries.length}건 · ${fromUnits(paidAll)} USDC`, `${entries.length} · ${fromUnits(paidAll)} USDC`) : "…"}
        />
      </ul>
      {openLabel && (
        <>
          <Eyebrow className="px-3 mt-6 mb-1">{t("진행 중인 프로젝트", "Project in progress")}</Eyebrow>
          <ul>
            <LibraryItem
              accent={!locked}
              on={scope === `job:${openLabel}`}
              onPick={() => onScope(`job:${openLabel}`)}
              name={names.label(openLabel)}
              meta={locked ? t("긴급 잠금 중", "Emergency lock on") : t(`${money(job!.left)} 남음`, `${money(job!.left)} left`)}
            />
          </ul>
        </>
      )}
      <Eyebrow className="px-3 mt-6 mb-1">{t("지난 프로젝트", "Past projects")}</Eyebrow>
      {entries == null ? (
        <p className={cn("px-3 text-sm", faint)}>…</p>
      ) : past.length === 0 ? (
        <p className={cn("px-3 text-sm", faint)}>{t("아직 없어요", "None yet")}</p>
      ) : (
        <ul className="flex flex-col gap-0.5">
          {past.map((g) => (
            <LibraryItem key={g.key} on={scope === `job:${g.job}`} onPick={() => onScope(`job:${g.job}`)} name={names.label(g.job)} meta={jobMeta(g)} />
          ))}
        </ul>
      )}
      {noJob.length > 0 && (
        <ul className="mt-6">
          <LibraryItem
            on={scope === "nojob"}
            onPick={() => onScope("nojob")}
            name={t("프로젝트 없음", "No project")}
            meta={t(`${noJob.length}건 · ${fromUnits(paidNoJob)} USDC`, `${noJob.length} · ${fromUnits(paidNoJob)} USDC`)}
          />
        </ul>
      )}
    </Library>
  );

  // 좁을 때(왼쪽 단이 없을 때) 고르는 칸.
  const scopeSelect = !showLib && (
    <div className="mb-5 flex items-center gap-2">
      {wide && lib === "closed" && (
        <IconButton label={t("목록 펼치기", "Show list")} onClick={() => setLib("open")}>
          <PanelLeft size={15} />
        </IconButton>
      )}
      <select
        value={scope}
        onChange={(e) => onScope(e.target.value as Scope)}
        aria-label={t("볼 프로젝트", "Project to show")}
        className={cn(
          "h-8 max-w-full rounded-full px-3 text-md outline-none",
          "bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)]",
          ink,
        )}
      >
        <option value="all">{t("한눈에 보기", "Overview")}</option>
        {openLabel && <option value={`job:${openLabel}`}>{names.label(openLabel)}</option>}
        {past.map((g) => (
          <option key={g.key} value={`job:${g.job}`}>
            {names.label(g.job)}
          </option>
        ))}
        {noJob.length > 0 && <option value="nojob">{t("프로젝트 없음", "No project")}</option>}
      </select>
    </div>
  );

  let head: React.ReactNode;
  if (scope === "all") {
    head = (
      <PageTitle
        title={t("한눈에 보기", "Overview")}
      />
    );
  } else if (scope === "nojob") {
    head = (
      <PageTitle
        title={t("프로젝트 없음", "No project")}
        sub={t("AI 가 프로젝트 이름표를 붙이지 않은 결제예요.", "Payments the AI didn't tag with a project.")}
      />
    );
  } else {
    const label = scopeJob!;
    const g = jobGroups.find((x) => x.job === label);
    const isOpen = label === openLabel && job != null;
    const mins = isOpen ? Math.max(0, Math.ceil(job!.secs_left / 60)) : 0;
    const note = receipt.note(label);
    head = (
      <div>
        <Eyebrow className="mb-1">{isOpen ? t("진행 중인 프로젝트", "Project in progress") : t("지난 프로젝트", "Past project")}</Eyebrow>
        <RenameField key={label} value={names.label(label)} placeholder={label} onSave={(n) => renameJob(label, n)} />
        <div className="mt-2 flex flex-wrap items-center justify-between gap-x-4 gap-y-3">
          <p className={cn("min-w-0 text-md num", muted)}>
            {note ??
              [names.renamed(label) ? t(`AI 이름표 ${label}`, `AI label ${label}`) : null, g ? `${groupMeta(g)} · ${fromUnits(g.paid)} USDC` : null]
                .filter(Boolean)
                .join("  ·  ")}
          </p>
          <div className="shrink-0 flex items-center gap-2">
            <GhostButton onClick={() => void receipt.save(label)}>
              <Download size={13} />
              CSV
            </GhostButton>
            {isOpen && <GhostButton onClick={onEnd}>{t("끝내기", "End")}</GhostButton>}
          </div>
        </div>
        {isOpen && (
          <>
            <div className="mt-8 grid grid-cols-1 min-[560px]:grid-cols-3 gap-8">
              <Gauge
                title={t("예산", "Budget")}
                total={money(job!.budget)}
                value={locked ? t("긴급 잠금 중, 결제 멈춤", "Emergency lock on, paused") : t(`${money(job!.left)} 남음`, `${money(job!.left)} left`)}
                fill={locked ? 0 : ratio(job!.spent, job!.budget)}
                muted={locked}
              />
              <Fact title={t("남은 시간", "Time left")} value={t(`${mins}분`, `${mins} min`)} />
              <Fact title={t("승인한 판매자", "Approved sellers")} value={t(`${job!.sellers}곳`, `${job!.sellers}`)} />
            </div>
          </>
        )}
      </div>
    );
  }

  return (
    <div ref={page} className="flex h-screen min-w-0 flex-1 bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]">
      {showLib && library}
      <PaymentList
        key={scope}
        head={
          <>
            {scopeSelect}
            {head}
          </>
        }
        entries={listEntries}
        groupDays
        showJob={scope === "all"}
        footer={<PartialNote n={partial} onMore={onMore} />}
      />
    </div>
  );
}
