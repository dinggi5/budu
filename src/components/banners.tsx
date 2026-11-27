// 메인 화면 상단 배너 — 시드 백업 경고 + 긴급 잠금 안내 + 업데이트 알림 + 큰 창의 승인 안내.

import { motion } from "framer-motion";
import { AlertTriangle, ShieldAlert, Wallet } from "lucide-react";
import { invoke } from "@tauri-apps/api/core";
import { cn } from "@/lib/cn";
import { enter } from "@/components/ui";
import { t } from "@/lib/i18n";

export function BackupNag({ onBackup }: { onBackup: () => void }) {
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)]",
        "bg-[var(--color-danger-tint)]",
      )}
    >
      <AlertTriangle size={16} className="shrink-0 text-[var(--color-danger)]" />
      <p className="flex-1 min-w-0 text-sm leading-snug text-[var(--color-danger)]">
        {t(
          "시드 12단어를 백업하세요.",
          "Back up your 12-word seed.",
        )}
      </p>
      <button
        type="button"
        onClick={onBackup}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-sm tracking-tight",
          "bg-[var(--color-danger)] text-white hover:bg-[var(--color-danger-active)]",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        {t("백업하기", "Back up")}
      </button>
    </motion.div>
  );
}

/** 설정 파일을 못 읽어 기본값으로 도는 중.
 *
 *  백엔드는 깨진 settings.json 의 한도·옵션을 보수적 기본값으로 조용히 접는다 — 앱이 안 뜨는
 *  것보단 낫지만 사용자가 고른 값이 아니다. 체인과 RPC 는 다르다: 파일의 그 두 필드만
 *  따로 읽어 화면·송금·MCP 가 같은 값을 쓴다 — 그래서 문구는 「화면에 보이는 대로」 라고만
 *  말한다(chain_id 까지 깨졌으면 연습용 체인·공식 RPC 로 접히고 화면도 그렇게 보인다).
 *  동작은 그대로 두고 그 사실만 알린다. 버튼은 없다 — 설정
 *  화면의 저장도 같은 이유로 막혀 있어서(read_settings_for_update) 안에서 할 수 있는 게 없다. */
export function SettingsBrokenBanner() {
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)]",
        "bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)]",
      )}
    >
      <AlertTriangle size={16} className="shrink-0 text-[var(--color-ink-500)]" />
      <p className="flex-1 min-w-0 text-sm leading-snug text-[var(--color-ink-700)] dark:text-[#B5AFA2]">
        {t(
          "설정 파일(~/.budu/settings.json)을 읽지 못했어요. 한도·옵션은 기본값이고, 체인과 RPC는 지금 화면에 보이는 대로예요. 파일을 고치거나 지우면 돌아와요.",
          "Couldn't read the settings file (~/.budu/settings.json). Limits and options are at their defaults; the chain and RPC are what the screens show. Fix or delete the file to get the rest back.",
        )}
      </p>
    </motion.div>
  );
}

export function LockBanner({ onUnlock }: { onUnlock: () => void }) {
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)]",
        "bg-[var(--color-danger-tint)]",
        "border border-[var(--color-danger-tint)]",
      )}
    >
      <ShieldAlert size={16} className="shrink-0 text-[var(--color-danger)]" />
      <p className="flex-1 min-w-0 text-sm leading-snug text-[var(--color-danger)]">
        {t(
          "긴급 잠금이 켜져 있어요. 모든 결제가 차단돼요.",
          "Emergency lock is on. Every payment is blocked.",
        )}
      </p>
      <button
        type="button"
        onClick={onUnlock}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-sm tracking-tight",
          "bg-[var(--color-danger)] text-white hover:bg-[var(--color-danger-active)]",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        {t("해제", "Turn off")}
      </button>
    </motion.div>
  );
}

/** 메뉴바 팝오버에서 결제 승인 요청이 기다리는 중.
 *
 *  승인은 큰 창만 한다(src-tauri/src/appwin.rs 머리) — 보통은 요청이 오는 순간 큰 창이 맨 위로
 *  뜨니 이 띠가 할 일이 없다. 있는 이유는 사람이 그 창을 일부러 닫아 둔 경우: 그러면 만료 1분
 *  전까지 다시 안 뜨는데, 사람은 지금 이 팝오버를 보고 있다. 버튼이 큰 창을 다시 띄운다. */
export function ApprovalElsewhereBanner() {
  return (
    <motion.div
      {...enter}
      className={cn(
        "w-full max-w-md flex items-center gap-3 px-4 py-3",
        "rounded-[var(--radius-card)]",
        "bg-[var(--color-ivory-50)] dark:bg-[var(--color-night-800)]",
        "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
      )}
    >
      <Wallet size={16} className="shrink-0 text-[var(--color-ok)]" />
      <p className="flex-1 min-w-0 text-sm leading-snug text-[var(--color-ink-500)]">
        {t("결제 승인을 기다리는 요청이 있어요.", "A payment is waiting for your approval.")}
      </p>
      <button
        type="button"
        onClick={() => invoke("show_approval").catch(() => {})}
        className={cn(
          "shrink-0 h-8 px-3.5 rounded-[var(--radius-pill)] text-sm tracking-tight",
          "bg-[var(--color-ok)] text-white hover:bg-[var(--color-ok-hover)]",
          "transition-colors duration-[var(--duration-base)]",
        )}
      >
        {t("승인하러 가기", "Review")}
      </button>
    </motion.div>
  );
}
