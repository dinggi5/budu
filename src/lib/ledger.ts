// 장부 표기 — 큰 창(시안 B «장부»)의 홈·작업·내역·판매자와 팝오버가 같이 쓰는 금액·상태 규칙.
//
// 개발 7 에선 이 함수들이 WindowHome.tsx 안에 있었고 팝오버의 BalanceCard 는 따로 반올림했다(4.999999 가 팝오버에선 5.00,
// 큰 창에선 4.99). 같은 잔액이 창마다 다르게 보이면 안 되니 한 곳에 둔다.

import { fmtExact, shortenAddress } from "./format";
import { locale, t } from "./i18n";
import type { Balances, HistoryEntry } from "./types";

/** 금액 표기 — 정확한 값(반올림 없음)에 소수 둘째 자리까지는 채운다(20 → 20.00, 0.005 → 0.005). 장부는 자리가 맞아야 읽힌다. */
export function money(s: string | undefined): string {
  if (s == null) return "—";
  const x = fmtExact(s);
  if (!/^[\d,]+(\.\d*)?$/.test(x)) return x;
  const [i, f = ""] = x.split(".");
  return `${i}.${f.padEnd(2, "0")}`;
}

/** USDC 십진 문자열 → 1e-6 정수. 못 읽으면 null. */
export function units(s: string): bigint | null {
  const m = /^(\d*)(?:\.(\d*))?$/.exec(s.trim());
  if (!m || (m[1] === "" && !m[2])) return null;
  return BigInt((m[1] || "0") + (m[2] ?? "").padEnd(6, "0").slice(0, 6));
}

/** 1e-6 정수 → 장부 금액. */
export function fromUnits(v: bigint): string {
  const s = v.toString().padStart(7, "0");
  return money(`${s.slice(0, -6)}.${s.slice(-6)}`);
}

/** 정확한 십진 문자열의 뺄셈(작은 쪽이 0) — 한도 남은 값. Number 로 하면 0.1+0.2 꼴 오차가 화면에 샌다. */
export function minusDec(a: string, b: string): string {
  const x = units(a);
  const y = units(b);
  if (x == null || y == null) return "—";
  return fromUnits(x > y ? x - y : 0n);
}

/** 채움 비율 0~1. 못 읽으면 0. */
export function ratio(used: string, total: string): number {
  const u = Number(used);
  const tt = Number(total);
  if (!Number.isFinite(u) || !Number.isFinite(tt) || tt <= 0) return 0;
  return Math.min(1, Math.max(0, u / tt));
}

/** 잔액 표기 — 소수 둘째 자리에서 **자른다**(반올림하면 4.999999 가 5.00 으로, 가진 것보다 많아 보인다). 다만 1 미만인데
 *  자른 값이 0.00 이 되면(0.004 처럼 x402 소액 결제 단위가 남았을 때) 정확한 값을 그대로 — «빈 지갑» 으로 읽히지 않게.
 *  팝오버와 큰 창이 같이 쓴다. */
export function balanceText(b: Balances | null): string {
  if (!b) return "—";
  const m = /^(\d*)(?:\.(\d*))?$/.exec(b.usdc.trim());
  if (!m) return b.usdc;
  const int = (m[1] || "0").replace(/^0+(?=\d)/, "").replace(/\B(?=(\d{3})+(?!\d))/g, ",");
  const frac = m[2] ?? "";
  const cut = frac.padEnd(2, "0").slice(0, 2);
  if (int === "0" && cut === "00" && /[1-9]/.test(frac)) return fmtExact(b.usdc);
  return `${int}.${cut}`;
}

/** 한도 칸 하나의 뜻 — 빈 값은 «무제한» 이 아니라 «설정을 못 읽었다»(`limits::LIMIT_UNKNOWN`)이고
 *  백엔드는 그때 **모든 결제를 막는다**. 무제한은 0 하나뿐이다(설정 화면 「0으로 두면 무제한」). */
export function limitKind(v: string | undefined): "loading" | "unknown" | "unlimited" | "set" {
  if (v === undefined) return "loading";
  const x = v.trim();
  if (x === "") return "unknown";
  // 숫자가 하나는 있어야 한다 — 「.」 하나는 백엔드가 거부하는 값(결제 막힘)이다.
  if (/^0*(\.0*)?$/.test(x) && /\d/.test(x)) return "unlimited";
  return "set";
}

/** 결제 대상 URL → (호스트, 경로). 경로가 없으면 호스트를 «무엇» 으로. 못 읽으면 원문. */
export function resourceParts(e: HistoryEntry): { host: string; path: string } {
  if (!e.resource) return { host: "", path: "" };
  try {
    const u = new URL(e.resource);
    return { host: u.host, path: u.pathname && u.pathname !== "/" ? u.pathname : "" };
  } catch {
    return { host: "", path: e.resource };
  }
}

export type Tone = "ok" | "bad" | "idle";

/** 판매자 경로 → 사람이 읽는 이름(일지는 명령어 대신 명사로). 경로·호스트·포트는 «세부 정보» 에서만.
 *  모르는 경로는 «데이터» — 판매자마다 이름을 지어 붙이지 않는다(거짓 이름보다 낫다). */
const NOUNS: Record<string, [string, string]> = {
  "/wave": ["파고·바람 정보", "wave and wind data"],
  "/tide": ["조석 정보", "tide data"],
  "/congestion": ["터미널 대기 정보", "terminal wait data"],
  "/carbon": ["탄소 배출 계산", "carbon estimate"],
};

export function nounOf(e: HistoryEntry): string {
  const { path } = resourceParts(e);
  const hit = NOUNS[path.replace(/\/+$/, "")];
  if (hit) return t(hit[0], hit[1]);
  return e.resource ? t("데이터", "data") : t("결제", "payment");
}

function cap(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}

/** 일지 한 줄 — 명사 + 결말 한 마디(«조석 정보 결제 완료»). */
export function journalLine(e: HistoryEntry): string {
  const n = nounOf(e);
  const undelivered = !!e.undelivered && ["settled", "signed", "settle_failed"].includes(e.status);
  if (undelivered) return t(`${n} — 내용을 못 받았어요`, `${cap(n)} — content never arrived`);
  switch (e.status) {
    case "settled":
      // «받았어요» 라고 쓰지 않는다 — 체인 확인(confirm.rs)으로만 정산된 기록도 settle_tx 가 채워져, 지금 기록으론
      // 내용을 받았는지 가를 신호가 없다(못 받은 게 확실한 건 위 undelivered 갈래). 신호를 내역에 싣는 건 개발 12.
      return t(`${n} 결제 완료`, `Paid for ${n}`);
    case "signed":
      return t(`${n} — 정산 기다리는 중`, `${cap(n)} — awaiting settlement`);
    case "settle_failed":
      return t(`${n} — 정산 실패`, `${cap(n)} — settlement failed`);
    case "blocked":
      return t(`${n} 결제를 막았어요`, `Blocked a payment for ${n}`);
    case "failed":
      return t(`${n} 결제가 안 됐어요`, `Payment for ${n} failed`);
    case "expired":
    case "reverted":
      return t(`${n} 결제가 안 나갔어요`, `Payment for ${n} didn't go through`);
    default:
      // 모르는 상태에 «안 나감» 을 쓰지 않는다 — 나갔을 수 있다.
      return t(`${n} — 결과를 몰라요, 다시 내지 마세요`, `${cap(n)} — outcome unknown, don't pay again`);
  }
}

/** 사유의 앞머리(«승인 창 전 한도 검사 — »)를 뗀 짧은 꼴 — 줄 미리보기용. 전체는 상세에. */
export function shortReason(s: string): string {
  const i = s.indexOf(" — ");
  return i > 0 && i < 40 ? s.slice(i + 3) : s;
}

/** 기록 한 줄을 장부 말로 — 홈의 일지·내역·작업·판매자가 같은 판정을 쓴다(둘이 갈리면 한 화면에서만 경고가 사라진다).
 *  tone = 점 색, state = 상태 글자, what = 무엇을 샀나, why = 보조 한 줄, struck = 돈이 안 나간 금액(회색 취소선),
 *  paid = 합계에 넣는 «나간 돈»(정산된 것만), link = 탐색기 링크를 걸 tx. */
export function entryFacts(e: HistoryEntry): {
  tone: Tone;
  state: string;
  what: string;
  place: string;
  why: string;
  struck: boolean;
  paid: boolean;
  link: string;
  undelivered: boolean;
} {
  const { host } = resourceParts(e);
  // 무엇 = 사람이 읽는 이름(«조석 정보»). 경로·호스트는 상세의 «결제 대상» 에.
  const what = cap(nounOf(e));
  const place = host || shortenAddress(e.to);
  const undelivered = !!e.undelivered && ["settled", "signed", "settle_failed"].includes(e.status);
  const tx = e.settle_tx ? shortenAddress(e.settle_tx) : "";
  // 익스플로러 링크는 정산된 결제의 tx(판매자가 전한 페이실리테이터 서명)만. 「정산 대기」의 detail 은 **우리** 서명인데
  // 솔라나 tx id 는 첫 서명(수수료 지불자)이라 우리 서명으로는 탐색기에서 못 찾는다.
  const link = e.status === "settled" || e.status === "reverted" ? e.settle_tx ?? "" : "";
  // 돈이 나갔거나 나갔을 수 있는데 내용을 못 받았다 — 상태와 무관하게 이 경고가 먼저다.
  const lost =
    e.status === "signed"
      ? t("결과를 몰라요 — 다시 내지 마세요", "Outcome unknown — don't pay again")
      : t("돈은 나갔을 수 있는데 내용을 못 받았어요", "May have paid, but the content never arrived");
  const base = { what, place, link, undelivered };
  switch (e.status) {
    case "settled":
      return {
        ...base,
        tone: undelivered ? "bad" : "ok",
        // 체인 확인 전엔 판매자 말뿐이다.
        state: e.checked ? t("체인 확인", "on-chain") : t("판매자 보고", "per seller"),
        // 체인에서 확인된 정산이면 돈은 확실히 나갔다 — «나갔을 수 있는데» 가 아니다.
        why: undelivered
          ? e.checked
            ? t("돈은 나갔는데 내용을 못 받았어요", "Paid, but the content never arrived")
            : lost
          : [place, tx].filter(Boolean).join(" · "),
        struck: false,
        paid: true,
      };
    case "signed":
      return { ...base, tone: undelivered ? "bad" : "idle", state: t("정산 대기", "awaiting settlement"), why: undelivered ? lost : place, struck: false, paid: false };
    case "settle_failed":
      return { ...base, tone: "bad", state: t("정산 실패", "settlement failed"), why: undelivered ? lost : place, struck: false, paid: false };
    // 차단·실패는 돈이 안 나갔다 — 점과 사유는 붉게, 금액은 회색 취소선(붉은 금액은 «나간 돈» 으로 읽힌다).
    case "blocked":
      return { ...base, tone: "bad", state: t("차단", "blocked"), why: e.detail || place, struck: true, paid: false };
    case "failed":
      return { ...base, tone: "bad", state: t("실패", "failed"), why: e.detail || place, struck: true, paid: false };
    case "expired":
      return { ...base, tone: "idle", state: t("만료 · 안 나감", "expired · not paid"), why: place, struck: true, paid: false };
    case "reverted":
      return { ...base, tone: "idle", state: t("체인에서 실패 · 안 나감", "failed on-chain · not paid"), why: place, struck: true, paid: false };
    default:
      return { ...base, tone: "idle", state: e.status, why: place, struck: false, paid: false };
  }
}

export function hhmm(ts: number): string {
  return new Date(ts * 1000).toLocaleTimeString(locale(), { hour: "2-digit", minute: "2-digit", hour12: false });
}

export function dayLabel(ts: number): string {
  const d = new Date(ts * 1000);
  const thisYear = d.getFullYear() === new Date().getFullYear();
  return d.toLocaleDateString(locale(), { ...(thisYear ? {} : { year: "numeric" }), month: "short", day: "numeric" });
}

/** 오늘 것이 아니면 날짜를 붙인다 — 일지는 «오늘» 을 전제로 읽히니 어제 줄이 오늘처럼 보이면 안 된다. */
export function when(ts: number): string {
  const d = new Date(ts * 1000);
  if (d.toDateString() === new Date().toDateString()) return hhmm(ts);
  return `${dayLabel(ts)} ${hhmm(ts)}`;
}

/** 기록 묶음 — 작업 이름표가 있으면 작업별, 없으면 날짜별(«작업 없음 · 11월 21일»). 최근 것이 먼저. */
export type Group = {
  key: string;
  job: string;
  rows: HistoryEntry[];
  /** 정산된(나간) 합계 — 1e-6 정수. */
  paid: bigint;
  pending: number;
  last: number;
  first: number;
};

export function groupEntries(entries: HistoryEntry[]): Group[] {
  const map = new Map<string, Group>();
  for (const e of entries) {
    const job = (e.job ?? "").trim();
    const key = job ? `job:${job}` : `day:${new Date(e.ts * 1000).toDateString()}`;
    const g = map.get(key) ?? { key, job, rows: [], paid: 0n, pending: 0, last: 0, first: Number.MAX_SAFE_INTEGER };
    g.rows.push(e);
    const f = entryFacts(e);
    if (f.paid) g.paid += units(e.amount) ?? 0n;
    if (e.status === "signed" || e.status === "settle_failed") g.pending += 1;
    g.last = Math.max(g.last, e.ts);
    g.first = Math.min(g.first, e.ts);
    map.set(key, g);
  }
  const groups = [...map.values()];
  for (const g of groups) g.rows.sort((a, b) => b.ts - a.ts);
  return groups.sort((a, b) => b.last - a.last);
}

/** 판매자 묶음 — 결제 대상 호스트(없으면 받는 주소)별로 경로·건수·나간 돈. */
export type Seller = {
  key: string;
  host: string;
  payTos: string[];
  paid: bigint;
  count: number;
  last: number;
  paths: { path: string; price: string; count: number; last: number }[];
};

export function sellersOf(entries: HistoryEntry[]): Seller[] {
  const map = new Map<string, Seller>();
  for (const e of entries) {
    // 돈이 나갔거나 나갔을 수 있는 기록만 — 차단·실패·만료·체인 실패는 판매자와 거래한 게 아니다.
    // 백엔드 `history::counts_as_paid` 와 같은 목록.
    if (!["sent", "signed", "settled", "unknown", "settle_failed"].includes(e.status)) continue;
    const { host, path } = resourceParts(e);
    const key = host || e.to;
    const s = map.get(key) ?? { key, host, payTos: [], paid: 0n, count: 0, last: 0, paths: [] };
    if (!s.payTos.includes(e.to)) s.payTos.push(e.to);
    s.count += 1;
    if (entryFacts(e).paid) s.paid += units(e.amount) ?? 0n;
    s.last = Math.max(s.last, e.ts);
    const p = path || "/";
    const row = s.paths.find((x) => x.path === p);
    if (row) {
      row.count += 1;
      if (e.ts > row.last) {
        row.last = e.ts;
        row.price = e.amount;
      }
    } else {
      s.paths.push({ path: p, price: e.amount, count: 1, last: e.ts });
    }
    map.set(key, s);
  }
  const list = [...map.values()];
  for (const s of list) s.paths.sort((a, b) => b.count - a.count || a.path.localeCompare(b.path));
  return list.sort((a, b) => b.last - a.last);
}
