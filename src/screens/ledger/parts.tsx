// 장부 페이지들이 같이 쓰는 조각 — 결제 줄·상세·목록+상세 틀·영수증 내보내기·빈 화면.
// 데이터는 WalletScreen 이 읽는 내역(`get_history`)과 작업 예산(`job_status`)뿐이다. 금액·상태 판정은 lib/ledger.ts 한 곳.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl, revealItemInDir } from "@tauri-apps/plugin-opener";
import { AlertTriangle, Briefcase, Check, Copy, ExternalLink, PanelLeft, X } from "lucide-react";
import { cn } from "@/lib/cn";
import { explorerTxUrl, useChain } from "@/lib/chain";
import { shortenAddress } from "@/lib/format";
import { locale, t } from "@/lib/i18n";
import { useJobNames } from "@/lib/jobNames";
import { dayLabel, entryFacts, hhmm, money, shortReason, type Group } from "@/lib/ledger";
import { useCopy } from "@/lib/useCopy";
import type { HistoryEntry } from "@/lib/types";
import { FactRow, IconButton, Selectable, Tag, faint, hairline, ink, inkSoft, muted, sunken, useWidth } from "@/components/win";

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
    if (err?.job === job) return <span className="text-red-500">{err.msg}</span>;
    if (saved === job) return t("영수증을 다운로드 폴더에 저장했어요", "Receipt saved to Downloads");
    return null;
  }
  return { save, note };
}

export function Loading() {
  return <p className={cn("text-[13px]", faint)}>{t("불러오는 중…", "Loading…")}</p>;
}

export function Empty({ title, body }: { title: string; body: string }) {
  return (
    <p className={cn("text-[14px] leading-relaxed", muted)}>
      {title}
      <br />
      <span className={cn("text-[13px]", faint)}>{body}</span>
    </p>
  );
}

/** 합계가 잘린 내역 기준일 때 한 줄 — 영수증 CSV 는 전체를 읽으니 화면 합계와 다를 수 있다. */
export function PartialNote({ n, onMore }: { n?: number; onMore?: () => void }) {
  if (!n) return null;
  return (
    <p className={cn("text-[12px] leading-relaxed", muted)}>
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

/** 가운데 단의 결제 한 줄 — 무엇 + 금액 / 두 줄 미리보기 / 꼬리표. */
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
  // 미리보기는 짧게 — 사유는 앞머리를 떼고, 주소·포트는 상세 단에만.
  const preview = f.undelivered || f.tone === "bad" ? shortReason(f.why) : hhmm(entry.ts);
  return (
    <li className={cn("border-b last:border-b-0", hairline)}>
      <Selectable on={on} onClick={onPick} className="px-3 py-3 my-0.5">
        <span className="flex items-baseline justify-between gap-3">
          <span className={cn("min-w-0 truncate text-[14px] font-medium", ink)}>
            {f.what}
          </span>
          <span
            className={cn(
              "num shrink-0 text-[14px]",
              f.struck ? "line-through text-[var(--color-ink-300)]" : f.tone === "bad" ? "text-red-500" : ink,
            )}
          >
            {money(entry.amount)}
          </span>
        </span>
        <span
          className={cn(
            "mt-0.5 block text-[12px] leading-snug line-clamp-2 break-words",
            f.undelivered || f.tone === "bad" ? "text-red-500/90" : muted,
          )}
        >
          {f.undelivered && <AlertTriangle size={11} className="inline -mt-0.5 mr-1" />}
          {preview}
        </span>
        <span className="mt-1.5 flex items-center gap-1.5 min-w-0">
          <Tag tone={f.tone}>{f.state}</Tag>
          {jobLabel && <span className={cn("truncate text-[11px]", faint)}>{jobLabel}</span>}
        </span>
      </Selectable>
    </li>
  );
}

/** 오른쪽 단 — 고른 결제 하나의 모든 사실. */
export function PaymentDetail({ entry, onClose, overlay }: { entry: HistoryEntry; onClose: () => void; overlay: boolean }) {
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
    <aside
      className={cn(
        "h-full w-[320px] shrink-0 overflow-y-auto px-6 pt-14 pb-8",
        "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
        overlay ? "absolute right-0 top-0 z-20 shadow-[var(--shadow-lift)]" : `border-l ${hairline}`,
      )}
      aria-label={t("결제 상세", "Payment details")}
    >
      <div className="flex items-center justify-between">
        <Tag tone={f.tone}>{f.state}</Tag>
        <IconButton label={t("닫기", "Close")} onClick={onClose}>
          <X size={15} />
        </IconButton>
      </div>
      <div className="mt-3 flex items-baseline gap-1.5">
        <span className={cn("num text-[32px] font-bold leading-none tracking-[-0.02em]", f.struck ? "line-through text-[var(--color-ink-300)]" : ink)}>
          {money(entry.amount)}
        </span>
        <span className={cn("text-[15px] font-semibold", muted)}>USDC</span>
      </div>
      <p className={cn("mt-2 text-[14px] font-medium break-words", ink)}>{f.what}</p>

      {(f.undelivered || reason) && (
        <p className="mt-4 rounded-[10px] bg-red-500/[0.07] px-3 py-2.5 text-[12px] leading-relaxed text-red-600 dark:text-red-400">
          {f.undelivered ? f.why : reason}
        </p>
      )}

      <dl className="mt-5">
        <FactRow name={t("시각", "Time")}>
          <span className="num">{full}</span>
        </FactRow>
        {entry.resource && (
          <FactRow name={t("결제 대상", "Paid for")}>
            <span className="font-mono text-[12px] break-all">{entry.resource}</span>
          </FactRow>
        )}
        <FactRow name={t("받는 주소", "Paid to")}>
          <span className="flex items-center gap-1">
            <span className="font-mono text-[12px] break-all">{entry.to}</span>
            <IconButton label={t("주소 복사", "Copy address")} onClick={() => copyAddr(entry.to)}>
              {addrCopied ? <Check size={13} className="text-[var(--color-accent)]" /> : <Copy size={13} />}
            </IconButton>
          </span>
        </FactRow>
        {job && (
          <FactRow name={t("작업", "Job")}>
            {names.label(job)}
            {names.renamed(job) && <span className={cn("block text-[12px]", muted)}>{t(`AI 이름표 ${job}`, `AI label ${job}`)}</span>}
          </FactRow>
        )}
        {entry.settle_tx && (
          <FactRow name={t("정산 거래", "Settlement tx")}>
            <span className="flex items-center gap-1">
              <span className="font-mono text-[12px]">{shortenAddress(entry.settle_tx)}</span>
              <IconButton label={t("거래 복사", "Copy tx")} onClick={() => copyTx(entry.settle_tx!)}>
                {txCopied ? <Check size={13} className="text-[var(--color-accent)]" /> : <Copy size={13} />}
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
    </aside>
  );
}

/** 가운데 + 오른쪽 단 — 결제 목록과 고른 결제의 상세. 넓으면(≥900) 나란히, 좁으면 상세가 목록 위에 겹쳐 뜬다. */
export function ListWithDetail({
  head,
  entries,
  allEntries,
  groupDays,
  showJob,
  footer,
  onDetail,
}: {
  head: React.ReactNode;
  entries: HistoryEntry[] | null;
  /** 고른 결제를 찾을 전체(새로 읽혀도 고른 것이 살아 있게). */
  allEntries: HistoryEntry[] | null;
  groupDays?: boolean;
  showJob?: boolean;
  footer?: React.ReactNode;
  /** 상세가 열렸나 — 좁은 창이면 부르는 쪽이 왼쪽 단을 접어 가운데 자리를 낸다. */
  onDetail?: (open: boolean) => void;
}) {
  const names = useJobNames();
  // 목록 최소 ~340 + 상세 320 이 들어가면 나란히, 아니면 상세가 겹쳐 뜬다.
  const [box, boxW] = useWidth<HTMLDivElement>();
  const inline = boxW >= 660;
  const [picked, setPicked] = useState<string | null>(null);
  const sel = picked ? (allEntries ?? []).find((e) => entryKey(e) === picked) ?? null : null;
  const detailOpen = sel != null;
  useEffect(() => {
    onDetail?.(detailOpen);
  }, [detailOpen, onDetail]);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      // 입력 칸의 Esc(이름 고치기 취소)나 대화 상자(승인 시트)가 떠 있을 때는 뒤의 상세를 건드리지 않는다.
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
    <div ref={box} className="relative flex-1 min-w-0 flex h-full">
      <section className="flex-1 min-w-0 h-full overflow-y-auto px-6 min-[1100px]:px-10 pt-14 pb-10">
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
      </section>
      {sel && <PaymentDetail key={picked} entry={sel} onClose={() => setPicked(null)} overlay={!inline} />}
    </div>
  );
}

export function FragmentWithDay({ day, first, children }: { day: string | null; first: boolean; children: React.ReactNode }) {
  return (
    <>
      {day && (
        <li aria-hidden className={cn("px-3 pb-1 text-[12px] font-medium", muted, first ? "pt-0" : "pt-6")}>
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
          {accent && <Briefcase size={13} className="shrink-0 text-[var(--color-accent)] dark:text-[var(--color-accent-on-dark)]" />}
          <span className={cn("truncate text-[14px]", on ? cn(ink, "font-semibold") : inkSoft)} title={name}>
            {name}
          </span>
        </span>
        <span className={cn("block text-[12px] num truncate", muted)}>{meta}</span>
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
      <div className={cn("text-[12px] mb-2", muted)}>{title}</div>
      <div className={cn("num text-[17px] font-semibold", ink)}>{value}</div>
    </div>
  );
}
