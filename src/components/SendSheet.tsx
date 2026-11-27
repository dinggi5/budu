// 보내기 시트 — USDC 또는 SOL.
//
// x402 결제와 다른 길이다 — AI 는 여기 닿지 못하고, 언제나 사람이 비밀번호로 보낸다(백엔드 `send.rs`).
// 순서: 토큰 · 받는 주소 · 금액 → 확인(수수료까지 다시 보여 줌) + 비밀번호 → 결과.
// 결제 한도는 «AI 가 쓰는 돈» 의 장치라 사람 송금엔 걸리지 않는다 — 시트가 그 사실을 말한다.
// USDC 도 수수료(와 받는 쪽 USDC 계좌가 없으면 그 계좌 렌트)는 SOL 로 낸다 — 모자라면 «다음» 전에 막는다.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { ArrowUpRight, Loader2 } from "lucide-react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { fmtExact, shortenAddress } from "@/lib/format";
import { t } from "@/lib/i18n";
import { when } from "@/lib/ledger";
import { cardBase, CloseButton, inputBase, modalOverlay, primaryBtn, PwInput, secondaryBtn } from "@/components/ui";
import { Tag } from "@/components/win";

type Token = "USDC" | "SOL";
type Send = { ts: number; to: string; token: Token; units: number; signature: string; status: string; detail: string };
type Rules = { fee_lamports: number; rent_exempt_min: number; ata_rent: number };
type Quote = { ata_exists: boolean; sol_need: number; rent_exempt_min: number };

const DEC: Record<Token, number> = { SOL: 9, USDC: 6 };

/** "0.5" → 최소 단위 bigint. 형식이 틀리면 null (백엔드 parse_amount 와 같은 규칙 — 최종 판정은 백엔드). */
function toUnits(s: string, dec: number): bigint | null {
  const m = new RegExp(`^(\\d*)(?:\\.(\\d{0,${dec}}))?$`).exec(s.trim());
  if (!m || (m[1] === "" && !m[2])) return null;
  return BigInt(m[1] || "0") * 10n ** BigInt(dec) + BigInt((m[2] ?? "").padEnd(dec, "0") || "0");
}

function fromUnits(v: bigint, dec: number): string {
  const unit = 10n ** BigInt(dec);
  const int = v / unit;
  const frac = (v % unit).toString().padStart(dec, "0").replace(/0+$/, "");
  return frac ? `${int}.${frac}` : `${int}`;
}

function stateOf(status: string): { tone: "ok" | "bad" | "idle"; text: string } {
  switch (status) {
    case "confirmed":
      return { tone: "ok", text: t("보냄", "sent") };
    case "failed":
      return { tone: "bad", text: t("실패 · 안 나감", "failed · not sent") };
    case "expired":
      return { tone: "idle", text: t("만료 · 안 나감", "expired · not sent") };
    case "unknown":
      // RPC 가 그 무렵 기록을 안 갖고 있어 결말을 못 정했다 — 나갔을 수 있다.
      return { tone: "bad", text: t("결말 모름 · 탐색기에서 확인", "unknown · check the explorer") };
    default:
      return { tone: "idle", text: t("확인 중 · 다시 보내지 마세요", "checking · don't resend") };
  }
}

const ADDR = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/;

export function SendSheet({
  from,
  sol,
  usdc,
  onClose,
  onSent,
}: {
  /** 보내는 계정 주소 — 백엔드가 서명할 계정과 대조한다. */
  from: string;
  sol: string | undefined;
  usdc: string | undefined;
  onClose: () => void;
  onSent: () => void;
}) {
  const chain = useChain();
  const [token, setToken] = useState<Token>("USDC");
  const [step, setStep] = useState<"form" | "confirm" | "done">("form");
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ signature: string; status: string } | null>(null);
  const [rules, setRules] = useState<Rules>({ fee_lamports: 5000, rent_exempt_min: 890880, ata_rent: 2039280 });
  const [recent, setRecent] = useState<Send[]>([]);
  // USDC: 받는 주소를 적으면 그쪽 USDC 계좌가 있는지 묻는다(없으면 렌트가 든다).
  const [quote, setQuote] = useState<{ to: string; chain: number; q: Quote } | { to: string; chain: number; err: string } | null>(null);
  // 확인 단계에 들어갈 때 본 네트워크·계정 — 보낼 때 이 값을 넘겨 그 사이 바뀌었으면 백엔드가 멈춘다.
  const [seen, setSeen] = useState<{ chain: number; from: string } | null>(null);
  const [resetNote, setResetNote] = useState<string | null>(null);

  const loadRecent = () =>
    invoke<Send[]>("get_sends")
      .then((l) => setRecent(l.slice(0, 5)))
      .catch(() => {});
  useEffect(() => {
    invoke<Rules>("send_rules").then(setRules).catch(() => {});
    loadRecent();
    // «확인 중» 으로 남은 것의 결말을 체인에 묻는다.
    invoke<boolean>("check_sends")
      .then((changed) => {
        if (changed) void loadRecent();
      })
      .catch(() => {});
  }, []);

  // Esc = 닫기(보내는 중엔 닫지 않는다 — 결과를 못 보고 다시 보내는 일이 없게).
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !busy) onClose();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [busy, onClose]);

  const addr = to.trim();
  const addrOk = ADDR.test(addr);
  useEffect(() => {
    if (token !== "USDC" || !addrOk) return;
    let live = true;
    const c = chain.id;
    const id = setTimeout(() => {
      invoke<Quote>("usdc_send_quote", { to: addr })
        .then((q) => live && setQuote({ to: addr, chain: c, q }))
        .catch((e) => live && setQuote({ to: addr, chain: c, err: String(e) }));
    }, 300);
    return () => {
      live = false;
      clearTimeout(id);
    };
  }, [token, addr, addrOk, chain.id]);
  // 확인 화면을 띄운 채 네트워크나 계정이 바뀌면 처음 단계로 — 확인한 화면과 다른 곳에서 돈이 나가지 않게.
  // 보내는 중(busy)엔 건드리지 않는다: 백엔드가 대조해 거절하고, 그 결과를 이 화면에서 보여 준다.
  useEffect(() => {
    if (step !== "confirm" || busy || !seen) return;
    if (seen.chain !== chain.id || seen.from !== from) {
      setStep("form");
      setSeen(null);
      setPw("");
      setError(null);
      setResetNote(t("네트워크나 계정이 바뀌어 처음으로 돌아왔어요.", "The network or account changed, so this started over."));
    }
  }, [step, busy, seen, chain.id, from]);

  const dec = DEC[token];
  const solHave = sol != null ? toUnits(sol, 9) : null;
  const usdcHave = usdc != null ? toUnits(usdc, 6) : null;
  const units = toUnits(amount, dec);
  const fee = BigInt(rules.fee_lamports);
  const q = quote && quote.to === addr && quote.chain === chain.id ? quote : null;

  // 금액·SOL 판정(사람 눈에 먼저 — 최종 판정은 백엔드가 다시 한다).
  let amountErr: string | null = null;
  let solErr: string | null = null;
  let max: bigint | null = null;
  if (token === "SOL") {
    max = solHave != null && solHave > fee ? solHave - fee : null;
    if (amount.trim() !== "") {
      if (units == null || units === 0n) amountErr = t("금액을 숫자로 적어 주세요", "Enter an amount");
      else if (solHave != null && units + fee > solHave) amountErr = t("수수료까지 SOL 이 모자라요", "Not enough SOL including the fee");
    }
  } else {
    max = usdcHave != null && usdcHave > 0n ? usdcHave : null;
    if (amount.trim() !== "") {
      if (units == null || units === 0n) amountErr = t("금액을 숫자로 적어 주세요", "Enter an amount");
      else if (usdcHave != null && units > usdcHave) amountErr = t("USDC 가 모자라요", "Not enough USDC");
    }
    if (q && "q" in q && solHave != null) {
      const need = BigInt(q.q.sol_need);
      const rest = solHave - need;
      if (rest < 0n || (rest > 0n && rest < BigInt(q.q.rent_exempt_min))) {
        const min = rest < 0n ? need : need + BigInt(q.q.rent_exempt_min);
        solErr = t(
          `USDC 를 보내려면 SOL 이 ${fromUnits(min, 9)} 필요해요 — 받기로 SOL 을 채우세요`,
          `Sending USDC needs ${fromUnits(min, 9)} SOL — top up SOL with Receive`,
        );
      }
    }
  }
  const quoteErr = token === "USDC" && q && "err" in q ? q.err : null;
  const ready =
    addrOk &&
    units != null &&
    units > 0n &&
    !amountErr &&
    (token === "SOL" || (q != null && "q" in q && !solErr));

  async function send() {
    if (!pw || busy) return;
    setBusy(true);
    setError(null);
    try {
      const r = await invoke<{ signature: string; status: string }>("send_funds", {
        password: pw,
        token,
        to: addr,
        amount: amount.trim(),
        expectedChainId: seen?.chain ?? chain.id,
        expectedFrom: seen?.from ?? from,
      });
      setResult(r);
      setStep("done");
      setSeen(null);
      onSent();
      loadRecent();
    } catch (e) {
      setError(String(e));
      setPw("");
    } finally {
      setBusy(false);
    }
  }

  const label = "text-sm text-[var(--color-ink-500)]";
  const have = token === "SOL" ? sol : usdc;

  return (
    <motion.div
      className={modalOverlay}
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      exit={{ opacity: 0 }}
      onClick={(e) => {
        if (e.target === e.currentTarget && !busy) onClose();
      }}
    >
      <motion.section
        initial={{ opacity: 0, y: 8 }}
        animate={{ opacity: 1, y: 0 }}
        transition={{ duration: 0.18, ease: [0.2, 0, 0, 1] }}
        className={cn(cardBase, "relative w-full max-w-sm my-auto px-7 py-7")}
        role="dialog"
        aria-modal="true"
        aria-label={t("보내기", "Send")}
      >
        {!busy && <CloseButton onClose={onClose} />}
        <h2 className="text-lg font-semibold">{t("보내기", "Send")}</h2>
        <p className={cn("mt-1 num", label)}>
          {t("보유", "Balance")} {have != null ? fmtExact(have) : "—"} {token} · {chain.name}
        </p>

        {step === "form" && (
          <div className="mt-6 flex flex-col gap-4">
            <div role="radiogroup" aria-label={t("토큰", "Token")} className="grid grid-cols-2 gap-1 rounded-full p-1 bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)]">
              {(["USDC", "SOL"] as Token[]).map((k) => (
                <button
                  key={k}
                  type="button"
                  role="radio"
                  aria-checked={token === k}
                  onClick={() => {
                    setToken(k);
                    setAmount("");
                  }}
                  className={cn(
                    "h-8 rounded-full text-md transition-colors",
                    token === k
                      ? "bg-white font-semibold text-[var(--color-ink-900)] shadow-[var(--shadow-soft)] dark:bg-[var(--color-night-600)] dark:text-white"
                      : "text-[var(--color-ink-500)]",
                  )}
                >
                  {k}
                </button>
              ))}
            </div>
            <label className="flex flex-col gap-1.5">
              <span className={label}>{t("받는 주소", "Recipient")}</span>
              <input
                value={to}
                onChange={(e) => setTo(e.target.value)}
                placeholder={t("솔라나 지갑 주소", "Solana wallet address")}
                spellCheck={false}
                autoFocus
                className={cn(inputBase, "font-mono text-sm")}
              />
            </label>
            <label className="flex flex-col gap-1.5">
              <span className="flex justify-between">
                <span className={label}>{t(`금액 (${token})`, `Amount (${token})`)}</span>
                {max != null && (
                  <button type="button" onClick={() => setAmount(fromUnits(max, dec))} className="text-sm text-[var(--color-ok)]">
                    {t("전부", "Max")}
                  </button>
                )}
              </span>
              <input
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                inputMode="decimal"
                placeholder="0.0"
                className={cn(inputBase, "num text-md")}
              />
            </label>
            <p className="text-sm leading-relaxed text-[var(--color-ink-500)]">
              {token === "SOL"
                ? t(
                    `수수료 ${fromUnits(fee, 9)} SOL 이 따로 나가요. 처음 쓰는 주소엔 ${fromUnits(BigInt(rules.rent_exempt_min), 9)} SOL 이상 보내야 해요.`,
                    `A ${fromUnits(fee, 9)} SOL network fee is added. A brand-new address needs at least ${fromUnits(BigInt(rules.rent_exempt_min), 9)} SOL.`,
                  )
                : q && "q" in q && !q.q.ata_exists
                  ? t(
                      `받는 쪽에 아직 USDC 계좌가 없어 만들어 줘요 — 수수료 ${fromUnits(fee, 9)} SOL 에 계좌 값 ${fromUnits(BigInt(q.q.sol_need) - fee, 9)} SOL 이 더 들어요.`,
                      `The recipient has no USDC account yet, so this opens one — ${fromUnits(BigInt(q.q.sol_need) - fee, 9)} SOL on top of the ${fromUnits(fee, 9)} SOL fee.`,
                    )
                  : t(
                      `수수료 ${fromUnits(fee, 9)} SOL 은 SOL 로 내요.`,
                      `The ${fromUnits(fee, 9)} SOL network fee is paid in SOL.`,
                    )}
            </p>
            {addr !== "" && !addrOk && <p className="text-sm text-[var(--color-danger)]">{t("솔라나 주소 모양이 아니에요", "That doesn't look like a Solana address")}</p>}
            {quoteErr && <p className="text-sm text-[var(--color-danger)] break-all">{quoteErr}</p>}
            {amountErr && <p className="text-sm text-[var(--color-danger)]">{amountErr}</p>}
            {solErr && <p className="text-sm text-[var(--color-danger)]">{solErr}</p>}
            {resetNote && <p className="text-sm text-[var(--color-ink-500)]">{resetNote}</p>}
            <button
              type="button"
              disabled={!ready}
              onClick={() => {
                setSeen({ chain: chain.id, from });
                setResetNote(null);
                setStep("confirm");
              }}
              className={cn(primaryBtn, "mt-1")}
            >
              {t("다음", "Next")}
            </button>
          </div>
        )}

        {step === "confirm" && units != null && (
          <div className="mt-6">
            <div className="flex items-baseline gap-1.5">
              <span className="num text-lg font-bold">{fromUnits(units, dec)}</span>
              <span className="text-lg font-semibold text-[var(--color-ink-500)]">{token}</span>
            </div>
            <dl className="mt-4 flex flex-col gap-2 text-md">
              <div>
                <dt className={label}>{t("받는 주소", "Recipient")}</dt>
                <dd className="mt-0.5 font-mono text-sm break-all">{addr}</dd>
              </div>
              <div>
                <dt className={label}>{t("수수료", "Network fee")}</dt>
                <dd className="mt-0.5 num">
                  {token === "USDC" && q && "q" in q
                    ? q.q.ata_exists
                      ? `${fromUnits(fee, 9)} SOL`
                      : t(
                          `${fromUnits(BigInt(q.q.sol_need), 9)} SOL (USDC 계좌 만들기 포함)`,
                          `${fromUnits(BigInt(q.q.sol_need), 9)} SOL (incl. opening a USDC account)`,
                        )
                    : `${fromUnits(fee, 9)} SOL`}
                </dd>
              </div>
            </dl>
            {!chain.testnet && (
              <p className="mt-4 text-sm text-[var(--color-danger)]">
                {t(`메인넷 — 실제 ${token} 가 나가요. 보낸 뒤엔 되돌릴 수 없어요.`, `Mainnet — real ${token}. This can't be undone.`)}
              </p>
            )}
            <div className="mt-5">
              <PwInput value={pw} onChange={setPw} placeholder={t("비밀번호로 보내기", "Password to send")} autoFocus onEnter={send} />
            </div>
            {error && <p className="mt-3 text-sm text-[var(--color-danger)] break-all">{error}</p>}
            <div className="mt-5 grid grid-cols-[1fr_1.8fr] gap-2">
              <button type="button" onClick={() => setStep("form")} disabled={busy} className={cn(secondaryBtn, "w-full")}>
                {t("뒤로", "Back")}
              </button>
              <button type="button" onClick={send} disabled={!pw || busy} className={primaryBtn}>
                {busy && <Loader2 size={15} className="animate-spin" />}
                {busy ? t("보내는 중…", "Sending…") : t("보내기", "Send")}
              </button>
            </div>
          </div>
        )}

        {step === "done" && result && (
          <div className="mt-6">
            <Tag tone={stateOf(result.status).tone}>{stateOf(result.status).text}</Tag>
            <p className="mt-3 text-md leading-relaxed text-[var(--color-ink-700)] dark:text-[#B5AFA2]">
              {result.status === "confirmed"
                ? t("체인에서 확인했어요.", "Confirmed on-chain.")
                : result.status === "failed"
                  ? t(`체인에서 실패했어요. ${token} 는 나가지 않았어요(수수료만).`, `It failed on-chain. The ${token} didn't move (only the fee).`)
                  : t(
                      "보냈지만 아직 체인에서 확인하지 못했어요. 나갔을 수 있으니 다시 보내지 말고, 잠시 뒤 이 시트를 다시 열어 확인하세요.",
                      "Sent, but not confirmed yet. It may have gone through — don't send again; reopen this sheet in a moment to check.",
                    )}
            </p>
            <button
              type="button"
              onClick={() => void openUrl(`${chain.explorerTx}${result.signature}${chain.explorerTxSuffix}`).catch(() => {})}
              className={cn(secondaryBtn, "mt-4 w-full")}
            >
              <ArrowUpRight size={13} />
              {chain.explorerName}
            </button>
          </div>
        )}

        {recent.length > 0 && step !== "confirm" && (
          <div className="mt-7">
            <p className={label}>{t("최근 보낸 것", "Recent sends")}</p>
            <ul className="mt-2 flex flex-col gap-2">
              {recent.map((r) => {
                const s = stateOf(r.status);
                const tk: Token = r.token === "USDC" ? "USDC" : "SOL";
                return (
                  <li key={r.signature} className="flex items-center justify-between gap-3 text-sm">
                    <button
                      type="button"
                      className="min-w-0 truncate text-left font-mono text-[var(--color-ink-500)] hover:underline"
                      title={r.signature}
                      onClick={() => void openUrl(`${chain.explorerTx}${r.signature}${chain.explorerTxSuffix}`).catch(() => {})}
                    >
                      {when(r.ts)} → {shortenAddress(r.to)}
                    </button>
                    <span className="shrink-0 flex items-center gap-2">
                      <span className="num">
                        {fromUnits(BigInt(r.units), DEC[tk])} {tk}
                      </span>
                      <Tag tone={s.tone}>{s.text}</Tag>
                    </span>
                  </li>
                );
              })}
            </ul>
          </div>
        )}
      </motion.section>
    </motion.div>
  );
}
