// 잔액 카드 — 메뉴바 팝오버(미니)의 주인공. 큰 USDC 잔액 + SOL 한 줄(수수료 토큰, 결제엔 불필요) + 주소 복사.

import { motion } from "framer-motion";
import { Check, Copy, Fuel, RefreshCw } from "lucide-react";
import { cn } from "@/lib/cn";
import { fmtAmount, shortenAddress } from "@/lib/format";
import { balanceText } from "@/lib/ledger";
import type { Balances } from "@/lib/types";
import { cardBase, enter } from "@/components/ui";
import { t } from "@/lib/i18n";

export function BalanceCard({
  address,
  copied,
  onCopy,
  balances,
  balanceError,
  refreshing,
  onRefresh,
}: {
  address: string;
  copied: boolean;
  onCopy: () => void;
  balances: Balances | null;
  balanceError: string | null;
  refreshing: boolean;
  onRefresh: () => void;
}) {
  return (
    <motion.section {...enter} className={cn(cardBase, "px-8 py-10")}>
      <div className="flex items-center justify-between">
        {/* 개발 54 는 여기(「총 잔액」 라벨 자리)에 계정 칩을 뒀지만, 11px 회색이라 «라벨»로
            읽혀 아무도 누를 생각을 안 했다. 계정은 화면 맨 위 헤더로 옮겼고
            여기는 비운다 — 라벨을 다시 채우는 건 개발 54 가 「숫자가 잔액인 건 자명하다」고
            뺀 것을 되돌리는 일이라 하지 않는다. */}
        <span aria-hidden />
        <button
          type="button"
          onClick={onRefresh}
          disabled={refreshing}
          aria-label={t("잔액 새로고침", "Refresh balance")}
          className={cn(
            "inline-flex items-center justify-center w-7 h-7 rounded-full",
            "text-[var(--color-ink-500)] dark:text-[#B5AFA2]",
            "bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)]",
            "border border-[var(--color-ivory-300)] dark:border-[var(--color-night-600)]",
            "hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
            "hover:bg-[var(--color-ivory-300)] dark:hover:bg-[var(--color-night-600)]",
            "transition-colors duration-[var(--duration-base)]",
            "disabled:opacity-40 disabled:cursor-not-allowed",
          )}
        >
          <RefreshCw size={14} className={refreshing ? "animate-spin" : ""} />
        </button>
      </div>

      <div className="mt-6 flex items-baseline gap-2">
        <span className="balance text-hero leading-none num">
          {/* 큰 창과 같은 표기 — 자르고, 1 미만 소액은 정확히(lib/ledger). 반올림하면 4.999999 가 5.00 으로 보였다. */}
          {balanceError ? "—" : balanceText(balances)}
        </span>
        <span className="text-md text-[var(--color-ink-500)] tracking-tight">USDC</span>
      </div>

      <p className="mt-2 flex items-center gap-1.5 text-md text-[var(--color-ink-500)] num">
        <Fuel size={12} className="text-[var(--color-ink-300)]" />
        {balanceError ? (
          <span className="text-[var(--color-danger)] text-sm">{t("잔액 조회 실패", "Couldn't load balance")}</span>
        ) : (
          // SOL 은 수수료 토큰 — x402 결제엔 필요 없다(페이실리테이터가 낸다). 0 이어도 결제는 된다.
          <>{t("SOL ", "SOL ")}{fmtAmount(balances?.sol, 4)}</>
        )}
      </p>

      <div className="mt-8">
        <button
          type="button"
          onClick={onCopy}
          className={cn(
            "group inline-flex items-center gap-2",
            "text-md text-[var(--color-ink-700)] dark:text-[#B5AFA2]",
            "font-mono tracking-tight",
            "transition-colors duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
            "hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD]",
          )}
          title={address}
        >
          <span>{shortenAddress(address)}</span>
          {copied ? (
            <Check size={13} className="text-[var(--color-ok)]" />
          ) : (
            <Copy size={13} className="text-[var(--color-ink-300)] group-hover:text-[var(--color-ink-700)] transition-colors" />
          )}
        </button>
      </div>
    </motion.section>
  );
}
