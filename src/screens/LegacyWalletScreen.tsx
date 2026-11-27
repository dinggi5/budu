// 옛 형식 지갑 — 개발 1(EVM 틀)이 만든 wallet.enc 는 평문 주소가 `0x…` 라 잔액·받기·결제가 전부 막힌다.
// 비밀번호 한 번이면 같은 12단어에서 솔라나 주소를 다시 계산해 고쳐 쓴다(키·암호문은 그대로).
// 솔라나 꼴인데 키와 안 맞는 주소는 백엔드가 고치지 않는다(변조일 수 있다) — 그땐 이 화면이 아니라 오류로 막힌다.

import { useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import { Loader2, RefreshCw, Wrench } from "lucide-react";
import { cn } from "@/lib/cn";
import type { WalletStatus } from "@/lib/types";
import { cardBase, enter, primaryBtn, shell, FlowIcon, PwInput } from "@/components/ui";
import { t } from "@/lib/i18n";

export function LegacyWalletScreen({ onDone }: { onDone: (s: WalletStatus) => void }) {
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  // 지갑 화면처럼 1초마다 대기 요청을 묻는다 — 백엔드는 이 물음을 「화면이 살아 있다」 로 치고, 옛 형식인 동안 들어온 AI 결제는
  // 창 전에 진짜 이유와 함께 돌려보낸다. 안 물으면 감시 스레드가 「창이 응답 없음」 으로 오판했다.
  useEffect(() => {
    const id = window.setInterval(() => {
      invoke("get_pending_request").catch(() => {});
      // 다른 창(큰 창·팝오버)에서 이미 고쳤으면 이 화면도 지갑으로 넘어간다.
      invoke<WalletStatus>("get_wallet_status")
        .then((s) => {
          if (s.state === "encrypted" && !s.legacy_address) onDone(s);
        })
        .catch(() => {});
    }, 1000);
    return () => window.clearInterval(id);
  }, [onDone]);

  async function submit() {
    if (!pw || busy) return;
    setBusy(true);
    setError(null);
    try {
      const s = await invoke<WalletStatus>("repair_wallet_address", { password: pw });
      setPw("");
      if (s.legacy_address) {
        setError(t("주소를 고치지 못했어요. 지갑 파일을 확인해주세요.", "Couldn't fix the address. Check the wallet file."));
        setBusy(false);
        return;
      }
      onDone(s);
    } catch (e) {
      setError(String(e));
      setBusy(false);
    }
  }

  return (
    <main className={shell}>
      <header className="w-full max-w-md flex items-center gap-2 text-[12px] text-[var(--color-ink-500)]">
        <span className="inline-block w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" aria-hidden />
        Budu
      </header>

      <motion.section {...enter} className={cn(cardBase, "max-w-md px-8 py-10")}>
        <FlowIcon>
          <Wrench size={22} />
        </FlowIcon>
        <h1 className="mt-5 text-center text-[19px] tracking-tight">
          {t("옛 형식 지갑이에요", "This wallet uses an old format")}
        </h1>
        <p className="mt-2 text-center text-[13px] leading-relaxed text-[var(--color-ink-500)]">
          {t(
            <>
              이 지갑 파일엔 이더리움 주소가 적혀 있어요.
              <br />
              비밀번호를 넣으면 같은 12단어로 솔라나 주소를 다시 적어요.
            </>,
            <>
              This wallet file still lists an Ethereum address.
              <br />
              Enter your password and the Solana address is rewritten from the same 12 words.
            </>,
          )}
        </p>

        <div className="mt-6">
          <PwInput value={pw} onChange={setPw} placeholder={t("비밀번호", "Password")} autoFocus onEnter={submit} />
        </div>

        {error && <p className="mt-4 text-[12px] text-red-500 font-mono break-all">{error}</p>}

        <button type="button" onClick={submit} disabled={!pw || busy} className={cn(primaryBtn, "mt-6")}>
          {busy ? <Loader2 size={15} className="animate-spin" /> : <RefreshCw size={15} />}
          {t("주소 고치기", "Fix address")}
        </button>

        <p className="mt-3 text-center text-[11px] leading-relaxed text-[var(--color-ink-300)]">
          {t(
            "12단어와 솔라나 쪽 돈은 그대로예요. 옛 이더리움 주소로 받은 것이 있다면 같은 12단어로 이더리움 지갑에서 계속 쓸 수 있어요.",
            "Your 12 words and Solana funds stay as they are. Anything sent to the old Ethereum address is still reachable with the same 12 words in an Ethereum wallet.",
          )}
        </p>
      </motion.section>
      <div />
    </main>
  );
}
