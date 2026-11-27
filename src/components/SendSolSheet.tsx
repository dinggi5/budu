// SOL 보내기 시트.
//
// x402 결제와 다른 길이다 — AI 는 여기 닿지 못하고, 언제나 사람이 비밀번호로 보낸다(백엔드 `sol_send.rs`).
// 순서: 받는 주소·금액 → 확인(수수료까지 다시 보여 줌) + 비밀번호 → 결과. USDC 한도는 USDC 에만 걸린다 — 시트가 그 사실을 말한다.

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

type SolSend = { ts: number; to: string; lamports: number; signature: string; status: string; detail: string };
type Rules = { fee_lamports: number; rent_exempt_min: number };

const DEC = 9n;
const UNIT = 10n ** DEC;

/** "0.5" → 500000000n. 형식이 틀리면 null (백엔드 parse_sol 과 같은 규칙 — 최종 판정은 백엔드). */
function toLamports(s: string): bigint | null {
  const m = /^(\d*)(?:\.(\d{0,9}))?$/.exec(s.trim());
  if (!m || (m[1] === "" && !m[2])) return null;
  return BigInt(m[1] || "0") * UNIT + BigInt((m[2] ?? "").padEnd(9, "0") || "0");
}

function fromLamports(v: bigint): string {
  const int = v / UNIT;
  const frac = (v % UNIT).toString().padStart(9, "0").replace(/0+$/, "");
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
    default:
      return { tone: "idle", text: t("확인 중 · 다시 보내지 마세요", "checking · don't resend") };
  }
}

export function SendSolSheet({ balance, onClose, onSent }: { balance: string | undefined; onClose: () => void; onSent: () => void }) {
  const chain = useChain();
  const [step, setStep] = useState<"form" | "confirm" | "done">("form");
  const [to, setTo] = useState("");
  const [amount, setAmount] = useState("");
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<{ signature: string; status: string } | null>(null);
  const [rules, setRules] = useState<Rules>({ fee_lamports: 5000, rent_exempt_min: 890880 });
  const [recent, setRecent] = useState<SolSend[]>([]);

  const loadRecent = () =>
    invoke<SolSend[]>("get_sol_sends")
      .then((l) => setRecent(l.slice(0, 5)))
      .catch(() => {});
  useEffect(() => {
    invoke<Rules>("sol_rules").then(setRules).catch(() => {});
    loadRecent();
    // «확인 중» 으로 남은 것의 결말을 체인에 묻는다.
    invoke<boolean>("check_sol_sends")
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

  const have = balance != null ? toLamports(balance) : null;
  const lamports = toLamports(amount);
  const fee = BigInt(rules.fee_lamports);
  const addrOk = /^[1-9A-HJ-NP-Za-km-z]{32,44}$/.test(to.trim());
  const amountOk = lamports != null && lamports > 0n && (have == null || lamports + fee <= have);
  const max = have != null && have > fee ? have - fee : null;

  async function send() {
    if (!pw || busy) return;
    setBusy(true);
    setError(null);
    try {
      const r = await invoke<{ signature: string; status: string }>("send_sol", { password: pw, to: to.trim(), amount: amount.trim(), expectedChainId: chain.id });
      setResult(r);
      setStep("done");
      onSent();
      loadRecent();
    } catch (e) {
      setError(String(e));
      setPw("");
    } finally {
      setBusy(false);
    }
  }

  const label = "text-[12px] text-[var(--color-ink-500)]";

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
        aria-label={t("SOL 보내기", "Send SOL")}
      >
        {!busy && <CloseButton onClose={onClose} />}
        <h2 className="text-[17px] font-semibold">{t("SOL 보내기", "Send SOL")}</h2>
        <p className={cn("mt-1 num", label)}>
          {t("보유", "Balance")} {balance != null ? fmtExact(balance) : "—"} SOL · {chain.name}
        </p>

        {step === "form" && (
          <div className="mt-6 flex flex-col gap-4">
            <label className="flex flex-col gap-1.5">
              <span className={label}>{t("받는 주소", "Recipient")}</span>
              <input
                value={to}
                onChange={(e) => setTo(e.target.value)}
                placeholder={t("솔라나 주소", "Solana address")}
                spellCheck={false}
                autoFocus
                className={cn(inputBase, "font-mono text-[12px]")}
              />
            </label>
            <label className="flex flex-col gap-1.5">
              <span className="flex justify-between">
                <span className={label}>{t("금액 (SOL)", "Amount (SOL)")}</span>
                {max != null && (
                  <button type="button" onClick={() => setAmount(fromLamports(max))} className="text-[12px] text-[var(--color-accent)]">
                    {t("전부", "Max")}
                  </button>
                )}
              </span>
              <input
                value={amount}
                onChange={(e) => setAmount(e.target.value)}
                inputMode="decimal"
                placeholder="0.0"
                className={cn(inputBase, "num text-[15px]")}
              />
            </label>
            <p className="text-[12px] leading-relaxed text-[var(--color-ink-300)]">
              {t(
                `수수료 ${fromLamports(fee)} SOL 이 따로 나가요. 처음 쓰는 주소엔 ${fromLamports(BigInt(rules.rent_exempt_min))} SOL 이상 보내야 해요. 결제 한도는 USDC 에만 걸려요.`,
                `A ${fromLamports(fee)} SOL network fee is added. A brand-new address needs at least ${fromLamports(BigInt(rules.rent_exempt_min))} SOL. Payment limits apply to USDC only.`,
              )}
            </p>
            {to.trim() !== "" && !addrOk && <p className="text-[12px] text-red-500">{t("솔라나 주소 모양이 아니에요", "That doesn't look like a Solana address")}</p>}
            {amount.trim() !== "" && !amountOk && (
              <p className="text-[12px] text-red-500">
                {lamports == null || lamports === 0n ? t("금액을 숫자로 적어 주세요", "Enter an amount") : t("수수료까지 SOL 이 모자라요", "Not enough SOL including the fee")}
              </p>
            )}
            <button type="button" disabled={!addrOk || !amountOk} onClick={() => setStep("confirm")} className={cn(primaryBtn, "mt-1")}>
              {t("다음", "Next")}
            </button>
          </div>
        )}

        {step === "confirm" && lamports != null && (
          <div className="mt-6">
            <div className="flex items-baseline gap-1.5">
              <span className="num text-[32px] font-bold leading-none tracking-[-0.02em]">{fromLamports(lamports)}</span>
              <span className="text-[15px] font-semibold text-[var(--color-ink-500)]">SOL</span>
            </div>
            <dl className="mt-4 flex flex-col gap-2 text-[13px]">
              <div>
                <dt className={label}>{t("받는 주소", "Recipient")}</dt>
                <dd className="mt-0.5 font-mono text-[12px] break-all">{to.trim()}</dd>
              </div>
              <div>
                <dt className={label}>{t("수수료", "Network fee")}</dt>
                <dd className="mt-0.5 num">{fromLamports(fee)} SOL</dd>
              </div>
            </dl>
            {!chain.testnet && (
              <p className="mt-4 text-[12px] text-[var(--color-accent)]">{t("메인넷 — 실제 SOL 이 나가요. 보낸 뒤엔 되돌릴 수 없어요.", "Mainnet — real SOL. This can't be undone.")}</p>
            )}
            <div className="mt-5">
              <PwInput value={pw} onChange={setPw} placeholder={t("비밀번호로 보내기", "Password to send")} autoFocus onEnter={send} />
            </div>
            {error && <p className="mt-3 text-[12px] text-red-500 break-all">{error}</p>}
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
            <p className="mt-3 text-[13px] leading-relaxed text-[var(--color-ink-700)] dark:text-[#B5AFA2]">
              {result.status === "confirmed"
                ? t("체인에서 확인했어요.", "Confirmed on-chain.")
                : result.status === "failed"
                  ? t("체인에서 실패했어요. 보낸 SOL 은 나가지 않았어요(수수료만).", "It failed on-chain. The SOL didn't move (only the fee).")
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
            <p className={label}>{t("최근 보낸 SOL", "Recent SOL sends")}</p>
            <ul className="mt-2 flex flex-col gap-2">
              {recent.map((r) => {
                const s = stateOf(r.status);
                return (
                  <li key={r.signature} className="flex items-center justify-between gap-3 text-[12px]">
                    <button
                      type="button"
                      className="min-w-0 truncate text-left font-mono text-[var(--color-ink-500)] hover:underline"
                      title={r.signature}
                      onClick={() => void openUrl(`${chain.explorerTx}${r.signature}${chain.explorerTxSuffix}`).catch(() => {})}
                    >
                      {when(r.ts)} → {shortenAddress(r.to)}
                    </button>
                    <span className="shrink-0 flex items-center gap-2">
                      <span className="num">{fromLamports(BigInt(r.lamports))}</span>
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
