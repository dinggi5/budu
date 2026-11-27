// 받기 카드 — 주소 QR + 복사 + (테스트넷) 테스트 코인 Faucet / (메인넷) 거래소 입금 안내 + (브릿지가 있는 체인) 공식 브릿지 한 줄.

import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { Check, Copy, Droplets, Fuel } from "lucide-react";
import { QRCodeSVG } from "qrcode.react";
import { cn } from "@/lib/cn";
import { useChain } from "@/lib/chain";
import { useCopy } from "@/lib/useCopy";
import { cardBase, enter, secondaryBtn, CloseButton, FaucetButton } from "@/components/ui";
import { t } from "@/lib/i18n";

export function ReceiveCard({ address, onClose }: { address: string; onClose: () => void }) {
  const chain = useChain();
  const [copied, copy] = useCopy();

  async function openFaucet(url: string) {
    navigator.clipboard?.writeText(address).catch(() => {});
    try {
      await openUrl(url);
    } catch {
      /* opener 실패는 조용히 무시 */
    }
  }

  return (
    <motion.section {...enter} className={cn(cardBase, "relative px-8 py-8")}>
      <CloseButton onClose={onClose} />

      <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">{t("받는 주소", "Receiving address")}</p>

      <div className="mt-6 flex justify-center">
        <div className="p-4 rounded-[10px] bg-white dark:bg-[#E8E5DD] border border-[var(--color-ivory-300)] dark:border-transparent">
          <QRCodeSVG value={address} size={176} bgColor="transparent" fgColor="#1A1A19" level="M" />
        </div>
      </div>

      <p className="mt-6 text-[11px] font-mono text-[var(--color-ink-700)] dark:text-[#B5AFA2] break-all leading-relaxed text-center select-all">
        {address}
      </p>

      <button type="button" onClick={() => copy(address)} className={cn(secondaryBtn, "mt-4 w-full")}>
        {copied ? (
          <>
            <Check size={13} className="text-[var(--color-accent)]" />
            {t("복사됨", "Copied")}
          </>
        ) : (
          <>
            <Copy size={13} />
            {t("주소 복사", "Copy address")}
          </>
        )}
      </button>

      {chain.testnet ? (
        <div className="mt-6 pt-5 border-t border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]">
          <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">{t("테스트 코인 받기", "Get test coins")}</p>
          <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--color-ink-300)]">
            {t(
              // 조사를 체인 이름 뒤에 붙이지 않는다 — 이름이 바뀌면 받침이 갈려 고정 조사가 틀린다.
              // 이름을 문장 끝으로 빼서 조사를 아예 안 타게 했다.
              `주소가 복사돼요. 열리는 페이지에 붙여넣고 네트워크를 고르세요 — ${chain.name}.`,
              `Your address is copied. Paste it on the page that opens and pick ${chain.name}.`,
            )}
          </p>
          {/* 결제엔 USDC 만 필요하다(x402 수수료는 페이실리테이터). SOL 은 데모 판매자 계정 등 따로 쓸 때만. */}
          <div className={cn("mt-3 grid gap-2", chain.solFaucet ? "grid-cols-2" : "grid-cols-1")}>
            {chain.usdcFaucet && (
              <FaucetButton icon={<Droplets size={13} />} label="USDC" sub="Circle" onClick={() => openFaucet(chain.usdcFaucet!)} />
            )}
            {chain.solFaucet && (
              <FaucetButton icon={<Fuel size={13} />} label="SOL" sub="Solana" onClick={() => openFaucet(chain.solFaucet!)} />
            )}
          </div>
        </div>
      ) : (
        <div className="mt-6 pt-5 border-t border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]">
          <p className="text-[11px] tracking-[0.04em] text-[var(--color-ink-500)]">{t("USDC 충전", "Adding USDC")}</p>
          <p className="mt-1.5 text-[11px] leading-relaxed text-[var(--color-ink-300)]">
            {t(
              <>
                거래소·다른 지갑에서 이 주소로 보낼 때 반드시 <b>{chain.depositNetwork ?? chain.name}</b>{" "}
                네트워크를 고르세요. 다른 네트워크로 보내면 자금을 잃을 수 있어요. USDC 만 받으면 돼요 — 결제 수수료는
                판매자 쪽이 내서 SOL 은 없어도 돼요. 에이전트 운영 예산만 소액 충전하세요.
              </>,
              <>
                When sending from an exchange or another wallet, you must pick the{" "}
                <b>{chain.depositNetwork ?? chain.name}</b> network. Sending over a different network can lose the funds. USDC is all you
                need — the seller's side pays the network fee, so no SOL is required. Top up only what the agent needs to spend.
              </>,
            )}
          </p>
        </div>
      )}
    </motion.section>
  );
}
