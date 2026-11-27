// 큰 창의 판매자 — 내역을 호스트별로 묶은 목록 / 고른 판매자의 산 것·결제 2단. 결제를 누르면 그 줄 아래로 세부가 펼쳐진다.
// 판매자의 카탈로그를 읽어 오지 않는다 — 큰 창이 판매자에게 요청을 보내는 길을 새로 열지 않는다.

import { useMemo, useState } from "react";
import { AlertTriangle, PanelLeft } from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { shortenAddress } from "@/lib/format";
import { t } from "@/lib/i18n";
import { useJobNames } from "@/lib/jobNames";
import { dayLabel, fromUnits, money, sellersOf } from "@/lib/ledger";
import type { HistoryEntry, JobView } from "@/lib/types";
import { Eyebrow, IconButton, PageTitle, Tag, hairline, ink, muted, usePref, useWidth } from "@/components/win";
import { Empty, Library, LibraryItem, Loading, PartialNote, PaymentList } from "./parts";


// ── 판매자 ────────────────────────────────────────────────────────────────────────────

export function SellersPage({
  entries,
  job,
  partial,
  address,
}: {
  entries: HistoryEntry[] | null;
  job: JobView | null;
  /** 활성 계정 주소 — 예산은 연 계정에서만 쓰인다. */
  address: string;
  /** 읽어 온 내역이 요청한 줄 수만큼 꽉 찼다 — 더 오래된 기록이 있을 수 있어 합계가 «최근 N건 기준» 이다. */
  partial?: number;
}) {
  const chain = useChain();
  const names = useJobNames();
  const [page, pageW] = useWidth<HTMLDivElement>();
  const wide = pageW >= 690;
  const [lib, setLib] = usePref<"open" | "closed">("sellers.lib", "open");
  const showLib = wide && lib === "open";
  const sellers = useMemo(() => (entries ? sellersOf(entries) : []), [entries]);
  const [pickedKey, setPickedKey] = useState<string | null>(null);
  const s = sellers.find((x) => x.key === pickedKey) ?? sellers[0] ?? null;
  // 예산은 같은 체인·같은 주소(payTo)일 때만 쓰인다(job.rs matches) — 다른 체인에서 연 작업은 이 화면 판매자와 무관하다.
  const jobHere = job != null && job.chain_id === chain.id && job.from.trim() === address.trim() ? job : null;
  const approved = new Set((jobHere?.seller_addrs ?? []).map((a) => a.trim()));

  const rows = useMemo(() => {
    if (!s || !entries) return [];
    return entries
      .filter((e) => {
        let host = "";
        try {
          host = e.resource ? new URL(e.resource).host : "";
        } catch {
          host = "";
        }
        return (host || e.to) === s.key;
      })
      .sort((a, b) => b.ts - a.ts);
  }, [s, entries]);

  const library = (
    <Library onCollapse={() => setLib("closed")}>
      <Eyebrow className="px-3 mb-1">{t("데이터 구매처", "Data sources")}</Eyebrow>
      <ul className="flex flex-col gap-0.5">
        {sellers.map((x) => (
          <LibraryItem
            key={x.key}
            on={s?.key === x.key}
            onPick={() => setPickedKey(x.key)}
            name={x.host || shortenAddress(x.key)}
            meta={t(`${x.count}건 · ${fromUnits(x.paid)} USDC`, `${x.count} · ${fromUnits(x.paid)} USDC`)}
            accent={jobHere != null && x.payTos.some((a) => approved.has(a.trim()))}
          />
        ))}
      </ul>
    </Library>
  );

  if (entries != null && sellers.length === 0) {
    return (
      <div ref={page} className="h-screen min-w-0 flex-1 overflow-y-auto px-6 min-[1100px]:px-10 pt-14 pb-10">
        <PageTitle title={t("데이터 구매처", "Data sources")} />
        <div className="mt-7">
          <Empty title={t("아직 거래한 판매자가 없어요.", "No sellers yet.")} />
        </div>
      </div>
    );
  }

  const okAddrs = s ? s.payTos.filter((a) => approved.has(a.trim())) : [];
  const head = (
    <div>
      {!showLib && (
        <div className="mb-5 flex items-center gap-2">
          {wide && lib === "closed" && (
            <IconButton label={t("목록 펼치기", "Show list")} onClick={() => setLib("open")}>
              <PanelLeft size={15} />
            </IconButton>
          )}
          <select
            value={s?.key ?? ""}
            onChange={(e) => setPickedKey(e.target.value)}
            aria-label={t("구매처", "Source")}
            className={cn("h-8 max-w-full rounded-full px-3 text-md outline-none bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)]", ink)}
          >
            {sellers.map((x) => (
              <option key={x.key} value={x.key}>
                {x.host || shortenAddress(x.key)}
              </option>
            ))}
          </select>
        </div>
      )}
      {entries == null || !s ? (
        <Loading />
      ) : (
        <>
          <Eyebrow className="mb-1">{t("구매처", "Source")}</Eyebrow>
          <PageTitle
            title={s.host || shortenAddress(s.key)}
            sub={t(
              `${s.count}건, 나간 돈 ${fromUnits(s.paid)} USDC. 받는 주소 ${s.payTos.map(shortenAddress).join(", ")}`,
              `${s.count} payments, ${fromUnits(s.paid)} USDC paid. Pays to ${s.payTos.map(shortenAddress).join(", ")}`,
            )}
          />
          {(okAddrs.length > 0 || s.payTos.length > 1) && (
            <div className="mt-3 flex flex-wrap gap-1.5">
              {jobHere && okAddrs.length > 0 && (
                <Tag tone="accent">
                  {s.payTos.length > 1
                    ? t(
                        `«${names.label(jobHere.job)}» 예산 안에서 ${okAddrs.map(shortenAddress).join(", ")} 로는 창 없이`,
                        `No prompt to ${okAddrs.map(shortenAddress).join(", ")} within «${names.label(jobHere.job)}»`,
                      )
                    : t(`«${names.label(jobHere.job)}» 예산 안에서 창 없이`, `No prompt within «${names.label(jobHere.job)}»`)}
                </Tag>
              )}
              {/* 두 곳 이상으로 돈을 받는 호스트 — 같은 이름 아래 다른 주인이 있을 수 있다. 사람이 알아보게만. */}
              {s.payTos.length > 1 && (
                <Tag tone="bad">
                  <AlertTriangle size={10} />
                  {t("여러 주소로 돈을 받았어요", "Paid at more than one address")}
                </Tag>
              )}
            </div>
          )}
          <Eyebrow className="mt-8 mb-1">{t("호출 목록", "Calls")}</Eyebrow>
          <ul>
            {s.paths.map((p) => (
              <li key={p.path} className={cn("grid grid-cols-[minmax(0,1fr)_auto_72px] gap-x-4 items-center py-2.5 border-b last:border-b-0 text-md", hairline)}>
                <span className={cn("min-w-0 truncate font-mono", ink)} title={p.path}>
                  {p.path}
                </span>
                <span className={cn("text-sm num", muted)}>
                  {t(`${p.count}번, 마지막 ${dayLabel(p.last)}`, `${p.count}×, last ${dayLabel(p.last)}`)}
                </span>
                <span className={cn("text-right num text-md", ink)}>{money(p.price)}</span>
              </li>
            ))}
          </ul>
          <Eyebrow className="mt-8">{t("결제 목록", "Payments")}</Eyebrow>
        </>
      )}
    </div>
  );

  return (
    <div ref={page} className="flex h-screen min-w-0 flex-1 bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]">
      {showLib && library}
      <PaymentList key={s?.key ?? "none"} head={head} entries={entries == null ? null : rows} groupDays footer={<PartialNote n={partial} />} />
    </div>
  );
}
