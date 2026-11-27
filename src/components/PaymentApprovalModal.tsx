// 결제 승인 시트 (AI 에이전트 → 사람 승인) — 큰 창의 모든 하위 화면 위에 뜬다(승인은 큰 창만). 모양은 개발 8 시안.

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import { AlertTriangle, FileSignature, Loader2, Lock, ShieldAlert } from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { accountName, fmtAmount, fmtCountdown, fmtExact, secsLeft, shortenAddress } from "@/lib/format";
import type { Account, Balances, JobView, LimitRoom, PaymentRequest } from "@/lib/types";
import { inputBase, modalCard, modalOverlay, primaryBtn, secondaryBtn, PwInput } from "@/components/ui";
import { t } from "@/lib/i18n";
import { money } from "@/lib/ledger";
import { useJobNames } from "@/lib/jobNames";

/// 십진 문자열을 토큰 decimals 기준 base unit(BigInt)으로. 형식이 아니면 null(검사 생략).
/// 부동소수 비교의 정밀도 손실을 피하려고 정수 비교용으로 쓴다.
function toBaseUnits(decimal: string, decimals: number): bigint | null {
  const s = decimal.trim();
  if (!/^\d+(\.\d+)?$/.test(s)) return null;
  const [whole, frac = ""] = s.split(".");
  const fracPadded = (frac + "0".repeat(decimals)).slice(0, decimals);
  try {
    return BigInt(whole + fracPadded);
  } catch {
    return null;
  }
}

export function PaymentApprovalModal({
  request,
  locked,
  balances,
  accounts,
  client,
  onResolved,
}: {
  request: PaymentRequest;
  /** 지금 붙어 있는 AI 클라이언트 이름(예: Claude Code) — 시트 맨 위 «누가» 한 줄. 모르면 «AI». */
  client?: string;
  locked: boolean;
  // 메인 화면이 이미 폴링 중인 잔액 — 사전 검사용(없으면 검사 생략, 백엔드가 최종 방어).
  balances: Balances | null;
  /** 지갑의 계정 목록 — 요청이 각인한 `from` 을 이름으로 바꿔 보여준다. */
  accounts: Account[];
  onResolved: () => void;
}) {
  const chain = useChain();
  // 어느 계정에서 나가는 결제인지. 요청이 각인한 주소가 지갑 계정 중 하나면 그 이름,
  // 아니면(다른 지갑·손상) 주소만 — 백엔드가 승인 시 거부하니 여기선 사실만 보여준다.
  // base58 은 대소문자를 가린다 — 백엔드와 같은 정확 일치.
  const fromAccount = request.from
    ? accounts.find((a) => a.address.trim() === request.from!.trim()) ?? null
    : null;
  const multiAccount = accounts.length > 1;
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [remaining, setRemaining] = useState(() => secsLeft(request.created));
  // 받는 주소가 이전에 비번으로 승인된 적 있는지 (null = 조회 중).
  const [trusted, setTrusted] = useState<boolean | null>(null);

  // 카운트다운(보통 5분, seed 결제는 마감까지). 0이 되면 MCP가 요청을 거둬가고 폴링이 팝업을 닫는다.
  useEffect(() => {
    const h = setInterval(() => setRemaining(secsLeft(request.created)), 1000);
    return () => clearInterval(h);
  }, [request.created]);

  useEffect(() => {
    invoke<boolean>("is_trusted_addr", { to: request.to })
      .then(setTrusted)
      .catch(() => setTrusted(null));
  }, [request.to]);

  // 한도 상황 — «건당 한도 · 오늘 남은 한도 → 결제 뒤». 한도를 넘는 요청은 백엔드가 창 전에 돌려보내므로
  // 여기서 오류가 나는 건 창이 뜬 뒤 다른 결제로 한도가 줄었거나 장부를 못 읽은 경우뿐이다(서명 단계가 어차피 막는다).
  const [room, setRoom] = useState<LimitRoom | null>(null);
  const [roomError, setRoomError] = useState<string | null>(null);
  // 승인 처리 중엔 다시 읽지 않는다 — 방금 예약한 금액이 장부에 들어가 한 번 더 더해져 «한도 초과» 가 잠깐 뜬다.
  const busyRef = useRef(false);
  // 창이 떠 있는 동안 한도를 바꾸거나 다른 결제가 나가면 숫자가 바뀐다 — 5초마다 다시 읽는다.
  useEffect(() => {
    const load = () =>
      busyRef.current ||
      invoke<LimitRoom>("limit_room", { amount: request.amount })
        .then((r) => {
          setRoom(r);
          setRoomError(null);
        })
        .catch((e) => {
          setRoom(null);
          setRoomError(String(e));
        });
    load();
    const h = setInterval(load, 5000);
    return () => clearInterval(h);
  }, [request.amount]);

  // 같은 곳으로 같은 금액이 방금(10분 안) 나갔는가 — 자율 결제가 이 창으로 넘어온 흔한 이유이고,
  // AI 가 응답을 놓치고 다시 요청한 중복 결제를 사람이 알아보는 유일한 자리다. 몇 초 전인지, 없으면 null.
  const [repeatSecs, setRepeatSecs] = useState<number | null>(null);
  useEffect(() => {
    invoke<number | null>("recent_same_payment_secs", {
      token: request.token,
      to: request.to,
      amount: request.amount,
    })
      .then(setRepeatSecs)
      .catch(() => setRepeatSecs(null));
  }, [request.token, request.to, request.amount]);

  // x402 = 서명만. 온체인 제출·가스는 페이실리테이터 몫 → 우리 잔액에서 가스가 안 나간다.
  // 부두가 받는 결제는 이것뿐이고(백엔드가 다른 kind 를 거절한다), 화면도 그 갈래만 그린다.
  // 🔴 백엔드 KIND_X402 와 같은 값. 옛 값과 비교하면 URL 줄·문구가 통째로 사라진다.
  const isX402Sign = request.kind === "x402-svm";
  const isX402 = isX402Sign;
  // 부두가 받는 결제는 x402 USDC 뿐이다(백엔드도 다른 토큰을 거절한다). 여기서 걸러야 "승인할 수 없는 요청"으로 정직하게 뜬다.
  const tokenOk = request.token === "USDC";

  // 사전 잔액 검사 — 거부밖에 못 누르는 막다른 길(정산 실패)을 미리 막는다. 잔액을 아직 못 읽었으면
  // (balances=null)·파싱 실패면 검사를 생략한다. ⚠️ UX 가드일 뿐 보안 경계가 아니다 — 실제 방어선은
  // 백엔드의 한도·긴급잠금·정산 시 온체인 검사. 비교는 base unit BigInt 로(JS Number 정밀도 손실 방지).
  // x402 는 서명만 하고 수수료는 페이실리테이터가 낸다 → SOL 여유분은 따지지 않는다.
  const haveStr = balances?.usdc;
  const decimals = 6; // 솔라나 USDC 민트는 데브넷·메인넷 모두 6
  const needUnits = toBaseUnits(request.amount, decimals);
  const haveUnits = haveStr != null ? toBaseUnits(haveStr, decimals) : null;
  const insufficient = needUnits != null && haveUnits != null && needUnits > haveUnits;
  // 수수료 지불자·온체인 메모 — 서명할 트랜잭션에 그대로 들어가는 값이라 사람이 볼 수 있어야 한다.
  const feePayer = request.svm?.fee_payer ?? "";
  // 판매자가 쓴 문자열. 보이지 않는 문자가 섞였으면 지우고 그리지 않는다 — 그러면 사람이 본 것과 tx 에 새겨질 바이트가
  // 달라진다. MCP 가 이미 거절하지만, 여기 왔다면 경고하고 승인을 막는다.
  const chainMemo = request.svm?.memo ?? "";
  const memoHidden = /[\p{C}]/u.test(chainMemo);
  // 작업 예산 — 작업 이름표가 있는 결제만 «이 작업 전체를 N USDC 까지» 를 고를 수 있다. 예산은 사람만 연다.
  const job = (request.job ?? "").trim();
  const jobNames = useJobNames();
  const [budgetMode, setBudgetMode] = useState<"once" | "job">("once");
  const [budget, setBudget] = useState(() => {
    const need = toBaseUnits(request.amount, 6);
    return need != null && need > 200_000n ? request.amount : "0.20";
  });
  const budgetUnits = toBaseUnits(budget, 6);
  // 같은 작업에 이미 열린 예산 — 이 창이 뜬 이유(처음 보는 판매자·예산 초과)를 사람이 알아보게.
  const [openJob, setOpenJob] = useState<JobView | null>(null);
  // 사람이 예산 칸을 손댔나 — 손대기 전에만 열린 작업에 맞춘 기본값으로 바꾼다.
  const budgetTouched = useRef(false);
  useEffect(() => {
    if (!job) return;
    invoke<JobView | null>("job_status")
      // 이름만이 아니라 체인·계정까지 — 백엔드(job.rs)는 셋이 다 같을 때만 기존 예산을 이어 쓰고, 하나라도 다르면 새 예산을 연다.
      // 이름만 맞추면 다른 체인의 «쓴 0.18 포함» 을 보이고 최소·기본값을 그 기준으로 잡았다.
      .then((v) =>
        setOpenJob(
          v &&
            v.job === job &&
            (request.chain_id == null || v.chain_id === request.chain_id) &&
            (!request.from || v.from.trim() === request.from.trim())
            ? v
            : null,
        ),
      )
      .catch(() => setOpenJob(null));
  }, [job, request.chain_id, request.from]);
  // 🔴 같은 작업에 다시 «이 작업 전체» 를 고르면 예산은 **총액으로 덮어써진다**(job.rs — 이미 쓴 돈은 그대로 남는다).
  // 그래서 이미 쓴 돈 + 이 결제가 최소이고, 기본값도 «지금 예산과 그 최소 중 큰 쪽» — 0.18 을 쓴 뒤 0.20 기본값으로 승인해
  // 남는 예산이 0.02 가 되는 일을 막는다.
  const spentUnits = openJob ? toBaseUnits(openJob.spent, 6) ?? 0n : 0n;
  const minBudget = needUnits != null ? needUnits + spentUnits : null;
  useEffect(() => {
    if (!openJob || budgetTouched.current || minBudget == null) return;
    const cur = toBaseUnits(openJob.budget, 6) ?? 0n;
    const want = cur > minBudget ? cur : minBudget;
    const s = want.toString().padStart(7, "0");
    setBudget(`${s.slice(0, -6)}.${s.slice(-6)}`.replace(/\.?0+$/, "") || "0");
  }, [openJob, minBudget]);
  const budgetBad = budgetMode === "job" && (budgetUnits == null || minBudget == null || budgetUnits < minBudget || budgetUnits === 0n);
  const blocked = !tokenOk || locked || insufficient || roomError != null || memoHidden || budgetBad;

  async function approve() {
    busyRef.current = true;
    setBusy(true);
    setError(null);
    try {
      await invoke("approve_payment", {
        id: request.id,
        password: pw,
        jobBudget: job && budgetMode === "job" ? budget.trim() : null,
      });
      setPw(""); // 성공 후 비번 즉시 비움 (입력칸 수명에만 두는 불변식)
      onResolved(); // 성공 → 팝업 닫고 잔액/내역 갱신
    } catch (e) {
      // 비번 오류·잠금·한도 등 — 요청은 살아있으니 재시도하거나 거부할 수 있다.
      setError(String(e));
      setPw("");
      busyRef.current = false;
      setBusy(false);
    }
  }

  async function reject() {
    setBusy(true);
    try {
      // 이 문구는 내역에 그대로 저장된다 — 나중에 언어를 바꿔도 옛 기록은 그때 언어로 남는다
      // (기록은 "그때 그렇게 보였다"의 사본이라 소급해 고치지 않는다).
      await invoke("reject_payment", {
        id: request.id,
        reason: t("사용자가 거부함", "Rejected by the user"),
      });
    } catch {
      /* 거부 실패는 무시 — 어차피 닫는다 */
    }
    onResolved();
  }

  // 받는 판매자 이름 — 결제 대상 URL 의 호스트. AI 가 정한 값이라 판정이 아니라 표시다(받는 주소는 아래 줄에 그대로).
  let sellerHost = "";
  try {
    if (request.resource) sellerHost = new URL(request.resource).host;
  } catch {
    sellerHost = "";
  }
  const amountText = fmtExact(request.amount);
  const warn = "flex items-start gap-1.5 text-sm leading-snug text-[var(--color-danger)]";
  const caution = "flex items-start gap-1.5 text-sm leading-snug text-[var(--color-danger)]";

  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      transition={{ duration: 0.2 }}
      className={modalOverlay}
    >
      {/* 승인 시트(시안 «공통 · 승인 시트») — 금액이 맨 위에 크게, 판매자·대상·작업·한도가 한 장의 표에, 고르기 둘, 비번 한 칸.
          판단 근거가 아닌 장식(아이콘 줄줄이)은 뺐다. 경고는 표 아래 한곳에 모은다. */}
      <motion.section
        role="dialog"
        aria-modal="true"
        aria-label={t("결제 승인", "Approve payment")}
        initial={{ opacity: 0, y: 12 }}
        animate={{ opacity: 1, y: 0 }}
        exit={{ opacity: 0, y: 8 }}
        transition={{ duration: 0.24, ease: [0.4, 0, 0.2, 1] }}
        className={cn(modalCard, "max-w-[448px] px-7 pt-7 pb-6 shadow-[0_20px_60px_rgba(0,0,0,0.22)] border-0")}
      >
        {/* 누가 · 남은 시간 */}
        <div className="flex items-center justify-between text-sm text-[var(--color-ink-500)]">
          <span className="truncate">
            {client
              ? t(`${client} 가 결제를 요청했어요`, `${client} is asking to pay`)
              : t("AI 가 결제를 요청했어요", "An AI is asking to pay")}
          </span>
          <span className="num shrink-0 ml-3">{fmtCountdown(remaining)}</span>
        </div>

        {/* 금액 — 이 시트의 주인공. 정확한 값(반올림 없음). */}
        <div className="mt-3.5 flex items-baseline gap-1.5 min-w-0">
          <span
            className={cn(
              "balance num text-lg font-bold tracking-[-0.02em] leading-none break-all text-[var(--color-ink-900)] dark:text-[#E8E5DD]",
            )}
          >
            {amountText}
          </span>
          {/* 글자는 세 단계뿐(잔액 숫자만 예외) — 금액은 큰 단계 굵게, 단위는 한 단계 낮춰 금액이 주인공이 되게. */}
          <span className="text-md font-semibold text-[var(--color-ink-500)]">{request.token}</span>
        </div>
        <p className="mt-1.5 text-md text-[var(--color-ink-700)] dark:text-[#B5AFA2] truncate">
          {sellerHost ? t(`${sellerHost} 에게`, `To ${sellerHost}`) : t("받는 곳에게", "To the recipient")}
          <span className="text-[var(--color-ink-500)]"> · </span>
          {/* 메인넷이면 실제 자금 — 빨강(멈춰·확인해). */}
          <span className={cn(chain.testnet ? "text-[var(--color-ink-500)]" : "text-[var(--color-danger)] font-medium")}>
            {chain.name}
            {!chain.testnet && t(" · 실제 자금", " · real funds")}
          </span>
        </p>

        {/* 사실 표 — 사람이 승인 전에 맞춰 볼 것. */}
        <dl className="mt-5 grid grid-cols-[92px_minmax(0,1fr)] gap-x-3 gap-y-2.5 text-md">
          {isX402 && request.resource && (
            <>
              <dt className="text-[var(--color-ink-500)]">{t("결제 대상", "Paying for")}</dt>
              <dd className="font-mono text-sm text-[var(--color-ink-900)] dark:text-[#E8E5DD] break-all">{request.resource}</dd>
            </>
          )}
          <dt className="text-[var(--color-ink-500)]">{t("받는 주소", "Pays to")}</dt>
          <dd className="min-w-0">
            <span className="font-mono text-sm text-[var(--color-ink-900)] dark:text-[#E8E5DD]" title={request.to}>
              {shortenAddress(request.to)}
            </span>
            {/* 신뢰 여부 — 보호자가 승인 판단에 쓰는 핵심 신호 */}
            {trusted === true && (
              <span className="ml-2 text-sm text-[var(--color-ink-500)]">{t("· 승인한 적 있음", "· approved before")}</span>
            )}
            {trusted === false && (
              <span className="ml-2 text-sm text-[var(--color-danger)]">{t("· 처음 보는 주소", "· new address")}</span>
            )}
          </dd>
          {job && (
            <>
              {/* 작업 이름표 — AI 가 붙인 이름이라 판단 근거가 아니라 묶음 표시다. */}
              <dt className="text-[var(--color-ink-500)]">{t("작업", "Job")}</dt>
              {/* 사람이 단 별명이 있으면 그것 + AI 이름표 — 예산은 이름표로 맞춘다. */}
              <dd className="text-[var(--color-ink-900)] dark:text-[#E8E5DD] break-words">
                {jobNames.label(job)}
                {jobNames.renamed(job) && <span className="block text-sm text-[var(--color-ink-500)]">{t(`AI 이름표 ${job}`, `AI label ${job}`)}</span>}
                {!jobNames.renamed(job) && jobNames.collides(job) && (
                  <span className="block text-sm text-[var(--color-danger)]">
                    {t("다른 작업에 붙인 별명과 글자가 같지만 그 작업과는 다른 작업이에요.", "Same text as another job's display name, but it is a different job.")}
                  </span>
                )}
              </dd>
            </>
          )}
          <dt className="text-[var(--color-ink-500)]">{t("메모", "Memo")}</dt>
          <dd className={cn("break-words", request.memo ? "text-[var(--color-ink-900)] dark:text-[#E8E5DD]" : "text-[var(--color-ink-300)]")}>
            {request.memo || t("AI 가 사유를 안 적었어요", "No reason given")}
          </dd>
          {/* 어느 계정에서 나가는지 — 계정이 둘 이상이거나 각인된 주소가 이 지갑 계정이 아닐 때만. */}
          {request.from && (multiAccount || !fromAccount) && (
            <>
              <dt className="text-[var(--color-ink-500)]">{t("계정", "Account")}</dt>
              <dd className={fromAccount ? "text-[var(--color-ink-900)] dark:text-[#E8E5DD]" : "text-[var(--color-danger)]"}>
                {fromAccount
                  ? accountName(fromAccount)
                  : t(`알 수 없는 계정 ${shortenAddress(request.from)}`, `Unknown account ${shortenAddress(request.from)}`)}
              </dd>
            </>
          )}
          {/* 한도 상황 — 이 결제 뒤에 오늘 얼마가 남는지. 한도가 없으면(0 = 무제한) 그 조각을 뺀다. */}
          {room && (room.single || room.daily) && (
            <>
              <dt className="text-[var(--color-ink-500)]">{t("한도", "Limits")}</dt>
              <dd className="num text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
                {[
                  room.daily && t(`결제 뒤 오늘 ${room.left_after} USDC 남음`, `${room.left_after} USDC left today after this`),
                  room.single && t(`건당 ${room.single}`, `${room.single} per payment`),
                ]
                  .filter(Boolean)
                  .join(" · ")}
              </dd>
            </>
          )}
          {/* 🔴 이 결제가 어떻게 나가는가 — 서명 갈래는 우리 가스가 안 나간다. */}
          {isX402Sign && (
            <>
              <dt className="text-[var(--color-ink-500)]">{t("수수료", "Fee")}</dt>
              <dd className="text-[var(--color-ink-500)]">
                {feePayer
                  ? t(`서명만 해요 · 페이실리테이터(${shortenAddress(feePayer)})가 내요`, `Signature only · the facilitator (${shortenAddress(feePayer)}) pays`)
                  : t("서명만 해요 · 페이실리테이터가 내요", "Signature only · the facilitator pays")}
              </dd>
            </>
          )}
          {/* 판매자가 정한 온체인 메모 — tx 에 그대로 새겨진다. 없으면 지갑이 난수를 넣으니 말할 게 없다. */}
          {chainMemo && (
            <>
              <dt className="text-[var(--color-ink-500)]">{t("온체인 메모", "On-chain memo")}</dt>
              <dd className="font-mono text-sm text-[var(--color-ink-500)] truncate" title={chainMemo}>
                {chainMemo}
              </dd>
            </>
          )}
        </dl>

        {/* 알아 둘 것·막는 것 — 한곳에. */}
        {(openJob || repeatSecs != null || locked || !tokenOk || memoHidden || roomError || insufficient) && (
          <div className="mt-4 flex flex-col gap-1.5">
            {openJob && (
              <p className="text-sm leading-snug text-[var(--color-ink-500)] num">
                {t(
                  `이 작업 예산 ${money(openJob.spent)} / ${money(openJob.budget)} USDC 사용 중 — 예산을 넘거나 처음 보는 판매자, 또는 앞의 같은 결제가 내용을 못 받아 다시 물어요`,
                  `Job budget ${money(openJob.spent)} / ${money(openJob.budget)} USDC used — asking again: over budget, a new seller, or the same payment earlier didn't deliver`,
                )}
              </p>
            )}
            {repeatSecs != null && (
              <p className={caution}>
                <AlertTriangle size={12} className="shrink-0 mt-px" />
                {t(
                  `${Math.max(1, Math.round(repeatSecs / 60))}분 전에도 같은 곳에 같은 금액 결제가 있었어요`,
                  `A payment of the same amount to this address was made ${Math.max(1, Math.round(repeatSecs / 60))} min ago`,
                )}
              </p>
            )}
            {locked && (
              <p className={warn}>
                <ShieldAlert size={12} className="shrink-0 mt-px" />
                {t("긴급 잠금 중 — 승인해도 차단돼요. 먼저 잠금을 해제하세요.", "Emergency lock is on — approving won't send. Turn the lock off first.")}
              </p>
            )}
            {!tokenOk && <p className={warn}>{t(`알 수 없는 토큰: ${request.token}`, `Unknown token: ${request.token}`)}</p>}
            {memoHidden && (
              <p className={warn}>
                <AlertTriangle size={12} className="shrink-0 mt-px" />
                {t("온체인 메모에 보이지 않는 문자가 섞여 있어 승인할 수 없어요.", "The on-chain memo contains hidden characters, so it can't be approved.")}
              </p>
            )}
            {roomError && (
              <p className={warn}>
                <AlertTriangle size={12} className="shrink-0 mt-px" />
                {roomError}
              </p>
            )}
            {insufficient && (
              <p className={warn}>
                <AlertTriangle size={12} className="shrink-0 mt-px" />
                <span>
                  {t(
                    `${request.token} 잔액이 부족해요 · 보유 ${fmtAmount(haveStr ?? "0", 6)} / 필요 ${fmtAmount(request.amount, 6)}`,
                    `Not enough ${request.token} · you have ${fmtAmount(haveStr ?? "0", 6)}, this needs ${fmtAmount(request.amount, 6)}`,
                  )}
                  {isX402Sign && t(" (정산 시 실패)", " (settlement would fail)")}
                </span>
              </p>
            )}
          </div>
        )}

        {/* 작업 예산 고르기 — «이 결제만» 이 기본. 고르면 같은 작업·이 판매자·예산 안의 다음 결제는 비번 없이 나간다(60분). */}
        {job && (
          <div className="mt-5">
            <div className="grid grid-cols-2 gap-2" role="radiogroup" aria-label={t("승인 범위", "Approval scope")}>
              {(["once", "job"] as const).map((m) => {
                const on = budgetMode === m;
                return (
                  <button
                    key={m}
                    type="button"
                    role="radio"
                    aria-checked={on}
                    disabled={busy}
                    onClick={() => setBudgetMode(m)}
                    className={cn(
                      "text-left rounded-xl px-3.5 py-3 transition-colors duration-[var(--duration-fast)]",
                      on
                        ? "bg-[var(--color-ok-tint)] shadow-[inset_0_0_0_1.5px_var(--color-ok)] dark:bg-[var(--color-ok)]/15"
                        : "shadow-[inset_0_0_0_1px_var(--color-ivory-400)] dark:shadow-[inset_0_0_0_1px_var(--color-night-600)] hover:bg-[var(--color-ivory-200)] dark:hover:bg-[var(--color-night-700)]",
                    )}
                  >
                    <b className="block text-md font-semibold text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
                      {m === "once" ? t("이 결제만", "Just this one") : t("이 작업 전체", "Whole job")}
                    </b>
                    <span
                      className={cn(
                        "block mt-0.5 text-sm num",
                        m === "job" && on ? "text-[var(--color-ok)]" : "text-[var(--color-ink-500)]",
                      )}
                    >
                      {m === "once"
                        ? t("다음 결제도 다시 물어요", "Asks again next time")
                        : openJob
                          ? t(
                              `총 ${budgetUnits != null ? money(budget.trim()) : "—"} 까지 · 쓴 ${money(openJob.spent)} 포함`,
                              `${budgetUnits != null ? money(budget.trim()) : "—"} total · ${money(openJob.spent)} spent incl.`,
                            )
                          : t(`${budgetUnits != null ? money(budget.trim()) : "—"} USDC 까지 · 60분`, `Up to ${budgetUnits != null ? money(budget.trim()) : "—"} USDC · 60 min`)}
                    </span>
                  </button>
                );
              })}
            </div>
            {budgetMode === "job" && (
              <>
                <div className="mt-2.5 flex items-center gap-2">
                  <input
                    value={budget}
                    onChange={(e) => {
                      budgetTouched.current = true;
                      setBudget(e.target.value);
                    }}
                    inputMode="decimal"
                    disabled={busy}
                    aria-label={t("작업 예산", "Job budget")}
                    className={cn(inputBase, "num text-right")}
                  />
                  <span className="shrink-0 text-sm text-[var(--color-ink-500)]">{t("USDC 까지", "USDC max")}</span>
                </div>
                <p className={cn("mt-1.5 text-sm leading-snug", budgetBad ? "text-[var(--color-danger)]" : "text-[var(--color-ink-500)]")}>
                  {budgetBad
                    ? openJob
                      ? t(
                          `이미 쓴 ${money(openJob.spent)} 와 이 결제를 합친 금액 이상을 넣어 주세요.`,
                          `Enter at least what's already spent (${money(openJob.spent)}) plus this payment.`,
                        )
                      : t("이 결제 금액 이상인 USDC 금액을 넣어 주세요.", "Enter a USDC amount at least this payment.")
                    : t(
                        "이 결제를 포함해 예산 안에서, 이 판매자에게 같은 작업으로 오는 결제는 60분 동안 비번 없이 나가요. 한도·긴급 잠금은 그대로예요.",
                        "Within this budget (this payment included), payments for this job to this seller go through without a password for 60 minutes. Limits and the emergency lock still apply.",
                      )}
                </p>
              </>
            )}
          </div>
        )}

        <div className="mt-4">
          <PwInput
            value={pw}
            onChange={setPw}
            placeholder={isX402Sign ? t("비밀번호로 서명 승인", "Password to approve the signature") : t("비밀번호로 승인", "Password to approve")}
            autoFocus
            onEnter={() => pw && !busy && !blocked && approve()}
          />
        </div>

        {error && <p className="mt-3 text-sm text-[var(--color-danger)] font-mono break-all">{error}</p>}

        <div className="mt-3.5 grid grid-cols-[1fr_2fr] gap-2">
          <button type="button" onClick={reject} disabled={busy} className={cn(secondaryBtn, "w-full h-12 text-md")}>
            {t("거절", "Reject")}
          </button>
          <button type="button" onClick={approve} disabled={!pw || busy || blocked} className={cn(primaryBtn, "h-12 text-md")}>
            {busy ? <Loader2 size={15} className="animate-spin" /> : isX402Sign ? <FileSignature size={15} /> : <Lock size={15} />}
            {isX402Sign ? t("승인하고 서명", "Approve and sign") : t("승인하고 보내기", "Approve and send")}
          </button>
        </div>
      </motion.section>
    </motion.div>
  );
}
