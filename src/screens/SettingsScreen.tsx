// 설정 화면 — 한도·자율 결제·화이트리스트·네트워크·RPC·앱(상시 구동/자동 시작).
// 도메인별 그룹 카드 스택(토스/Apple 설정 관용구) — 길게 쌓이던 한 덩어리를 끊어 스캔 가능하게.

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { Check, Gauge, Globe, Loader2, Network, Power, Settings as SettingsIcon, Zap } from "lucide-react";
import { cn } from "@/lib/cn";
import { CHAINS, chainFromId, type ChainConfig } from "@/lib/chain";
import type { Settings, SpendView } from "@/lib/types";
import { cardBase, inputBase, pageColumn, pageShell, primaryBtn, ScreenHeader , enter } from "@/components/ui";
import { isWindow } from "@/lib/win";
import { Selectable, ink, inkSoft } from "@/components/win";
import { holdLangReload, t } from "@/lib/i18n";
import {
  NavCtx,
  Section,
  AboutSection,
  TrustedAddrsModal,
  RowGroup,
  ToggleRow,
  LanguageRow,
  UnitInput,
  LimitGroup,
} from "@/screens/settings/parts";

const RPC_CUSTOM = "__custom__";

// Settings 의 boolean 필드만 — toggle() 이 실수로 문자열/숫자 필드를 뒤집지 못하게 좁힌다.
type BoolSettingKey =
  | "lock_on_blur"
  | "notify_auto"
  | "notify_hide_amount"
  | "auto_trusted_only";

// RPC 프리셋은 활성 체인에 따라 달라진다(공식·PublicNode URL이 체인별로 다름).
// ①공식 ②로그 안 남긴다 표방하는 대체 공개 RPC ③직접 입력(본인 키/노드 = 진짜 프라이버시).
// "공식"의 url 은 빈 값 = "활성 체인의 공식 RPC를 따라간다"(백엔드 effective_rpc 가 해석). 구체 URL 을
// 저장하면 체인을 바꿔도 옛 RPC 에 고정되는 함정이 생긴다.
function rpcPresets(chain: ChainConfig): { label: string; url: string }[] {
  const presets = [{ label: t(`${chain.name} 공식`, `${chain.name} official`), url: "" }];
  // PublicNode 가 없는 클러스터(지금은 솔라나 둘 다)는 이 줄 자체를 안 만든다 — 없는 대체 RPC 를 있는 척 채우면
  // "로그 안 남김"이라는 프라이버시 약속이 거짓이 된다. 그 체인은 공식 / 직접 입력 둘뿐.
  if (chain.publicNode) {
    presets.push({ label: t("PublicNode (로그 없음 표방)", "PublicNode (claims no logs)"), url: chain.publicNode });
  }
  return presets;
}

// 저장된 rpc_url 을 프리셋 표현으로 정규화. 옛 설정이 구체 공식 URL 을 박아 뒀어도 "공식"(빈 값)으로
// 인식되게 해, 드롭다운이 "직접 입력"으로 잘못 보이지 않게 한다.
function presetUrl(rpc: string | undefined, chain: ChainConfig): string {
  return !rpc || rpc === chain.defaultRpc ? "" : rpc;
}

export function SettingsScreen({
  current,
  spend,
  onClose,
  onGuard,
  initialGroup,
}: {
  current: Settings | null;
  spend: SpendView | null;
  onClose: () => void;
  /** 큰 창의 레일이 떠나도 되는지 묻는 데 쓴다 — 저장 중(busy)·저장 안 한 변경(dirty). */
  onGuard?: (g: { busy: boolean; dirty: boolean }) => void;
  /** 처음 열 묶음 — "app" 이면 «앱»(언어). */
  initialGroup?: "app";
}) {
  const [s, setS] = useState<Settings | null>(current);
  // 폼을 채운 원본 — 저장할 때 「이 창에서 바꾼 칸」만 골라내는 기준.
  // 창이 둘이면 다른 창이 그 사이 저장했을 수 있다. 예전처럼 폼 전체를 보내면, 손대지도 않은 자율 결제·한도·
  // 네트워크가 이 창이 열릴 때의 옛 값으로 **조용히 되돌아갔다**.
  const base = useRef<Settings | null>(current);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);
  // 저장 안 한 폼 변경이 있는가 — 언어 전환이 창을 다시 읽어서 이 값들을 버리기 때문에 필요하다
  // (한도를 낮추거나 테스트넷으로 바꿔 두고 언어를 누르면 그 변경이 사라졌다).
  const [dirty, setDirty] = useState(false);
  // RPC 직접 입력 모드(프리셋에 없는 URL이면 처음부터 직접 입력으로). 프리셋은 체인별로 다르다.
  const [rpcCustom, setRpcCustom] = useState<boolean>(() => {
    const c = chainFromId(current?.chain_id);
    return !rpcPresets(c).some((p) => p.url === presetUrl(current?.rpc_url, c));
  });
  // 화이트리스트 주소 목록 — null=로딩 중. 철회는 즉시 반영(저장 버튼과 무관).
  const [trusted, setTrusted] = useState<string[] | null>(null);
  const [showTrusted, setShowTrusted] = useState(false);
  // 로그인 시 자동 시작 — OS 로그인 아이템 상태가 진실 원천이라 즉시 적용(저장 버튼과 무관).
  const [autostart, setAutostart] = useState<boolean | null>(null);
  // 메뉴바 아이콘 — 앱 관리 필드라 저장 버튼과 무관하게 즉시 적용.
  const [menubar, setMenubar] = useState<boolean | null>(null);
  // 저장 성공 후 자동 닫기 타이머 — 언마운트/수동 닫기 때 정리해 옛 인스턴스 타이머가 부모를 다시 닫지 않게.
  const closeTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 이미 닫혔는지 — 저장 invoke 가 늦게 resolve 돼도(닫기를 그 사이 눌렀을 때) 상태 변경·타이머 생성을 막는다.
  const closed = useRef(false);

  useEffect(() => {
    invoke<string[]>("get_trusted_addrs")
      .then(setTrusted)
      .catch(() => setTrusted([]));
    invoke<boolean>("get_autostart")
      .then(setAutostart)
      .catch(() => setAutostart(false));
    invoke<boolean>("get_menubar")
      .then(setMenubar)
      .catch(() => setMenubar(true));
  }, []);

  // 언마운트 시 닫힘 표시 + 자동 닫기 타이머 정리.
  useEffect(
    () => () => {
      closed.current = true;
      if (closeTimer.current) clearTimeout(closeTimer.current);
    },
    [],
  );

  async function toggleAutostart() {
    if (autostart === null) return;
    const next = !autostart;
    setAutostart(next); // 낙관적 반영, 실패하면 원복
    try {
      await invoke("set_autostart", { enabled: next });
    } catch {
      setAutostart(!next);
    }
  }

  async function toggleMenubar() {
    if (menubar === null) return;
    const next = !menubar;
    setMenubar(next); // 낙관적 반영, 실패하면 원복
    try {
      await invoke("set_menubar", { on: next });
    } catch {
      setMenubar(!next);
    }
  }

  async function revokeTrusted(addr: string) {
    try {
      await invoke("remove_trusted_addr", { to: addr });
      setTrusted((prev) => (prev ? prev.filter((a) => a !== addr) : prev));
    } catch {
      /* 철회 실패는 목록 유지 — 다음 시도에서 다시 */
    }
  }

  // 설정이 늦게 로드되면 동기화. rpcCustom 은 마운트 시 한 번만 초기화되므로(useState),
  // current 가 null 로 시작해 늦게 로드되는 경우 여기서 RPC 모드도 함께 재계산해야
  // 커스텀 RPC 가 "직접 입력"으로 안 뜨는 문제를 막는다.
  // 저장 안 한 변경이 있는 동안엔 다른 창의 언어 변경이 이 창을 다시 읽지 않게.
  useEffect(() => (dirty ? holdLangReload() : undefined), [dirty]);
  useEffect(() => {
    onGuard?.({ busy, dirty });
  }, [onGuard, busy, dirty]);
  useEffect(() => () => onGuard?.({ busy: false, dirty: false }), [onGuard]);

  // 다른 창이 저장해 current 가 바뀌었고 이 폼은 손대지 않았다면 새 값으로 갈아 끼운다.
  useEffect(() => {
    if (current && s && !dirty && !busy && current !== base.current) {
      base.current = current;
      setS(current);
      const c = chainFromId(current.chain_id);
      setRpcCustom(!rpcPresets(c).some((p) => p.url === presetUrl(current.rpc_url, c)));
    }
  }, [current, s, dirty, busy]);

  useEffect(() => {
    if (current && !s) {
      base.current = current;
      setS(current);
      const c = chainFromId(current.chain_id);
      setRpcCustom(!rpcPresets(c).some((p) => p.url === presetUrl(current.rpc_url, c)));
    }
  }, [current, s]);

  // 화면 안에서 쓰는 활성 체인 = 편집 중인 s 기준(토글하면 즉시 프리셋·경고가 따라온다).
  const chain = chainFromId(s?.chain_id);
  const presets = rpcPresets(chain);

  function field(key: keyof Settings, value: string) {
    setS((prev) => (prev ? { ...prev, [key]: value } : prev));
    setSaved(false);
    setDirty(true);
  }

  function toggle(key: BoolSettingKey) {
    setS((prev) => (prev ? { ...prev, [key]: !prev[key] } : prev));
    setSaved(false);
    setDirty(true);
  }

  // 체인 전환 — RPC "선택 종류"는 유지한다. 공식/PublicNode 는 체인별로 정의돼 있으니 새 체인 값으로
  // 매핑하고, 직접 입력한 커스텀 URL 만 새 체인 공식으로 되돌린다(커스텀은 보통 체인 전용 — 예: Alchemy
  // 키에 base-sepolia/base-mainnet 이 박힘). 옛 체인 전용 RPC 가 새 체인에 남지 않게 한다는 개발 18
  // 커스텀만 리셋하고 프리셋은 안전하게 재매핑한다. 저장해야 적용.
  function selectChain(id: number) {
    const next = chainFromId(id);
    setS((prev) => {
      if (!prev) return prev;
      const rpc_url = rpcCustom
        ? "" // 커스텀 → 새 체인 공식 (옛 체인 전용 URL 은 못 옮긴다)
        : chain.publicNode && presetUrl(prev.rpc_url, chain) === chain.publicNode
          ? (next.publicNode ?? "") // PublicNode 선택 유지 (없는 체인이면 그 체인 공식으로)
          : ""; // 공식 유지
      return { ...prev, chain_id: id, rpc_url };
    });
    setRpcCustom(false);
    setSaved(false);
    setDirty(true);
  }

  async function save() {
    if (!s) return;
    setBusy(true);
    setError(null);
    try {
      // 바꾼 칸만 보낸다 — 합치기는 백엔드가 한 줄로 서서 한다(settings.rs `save_settings_edit`).
      // 폼을 채운 원본이 없으면(있을 수 없지만) 전부 바꾼 것으로 — 예전 동작.
      const from = base.current;
      const changed = (Object.keys(s) as (keyof Settings)[]).filter((k) => !from || s[k] !== from[k]);
      await invoke("save_settings_edit", {
        form: s,
        changed,
        baseChainId: from?.chain_id ?? s.chain_id,
      });
      if (closed.current) return; // 저장 도중 닫혔으면 늦은 setState·자동 닫기 타이머 생성을 막는다
      setSaved(true);
      setDirty(false);
      // 저장 성공 → "저장됨"을 잠깐 보여준 뒤 메인 화면으로 자동 복귀(매번 닫기를 또 누르는 번거로움 제거).
      // busy 를 유지해 닫히는 동안 폼 전체(아래 fieldset)가 잠긴다 → 이 0.5초 사이 입력이 조용히
      // 버려지는 일을 막는다. onClose 는 부모가 설정을 다시 불러오게 한다.
      closeTimer.current = setTimeout(onClose, 500);
    } catch (e) {
      if (closed.current) return;
      const msg = String(e);
      setError(msg);
      // 큰 창은 한 묶음만 보인다 — 오류가 가리키는 칸이 숨은 묶음에 있으면 그 묶음으로 옮긴다.
      if (isWindow) {
        const g = /RPC/.test(msg) ? 3 : /네트워크|network|체인|chain/i.test(msg) ? 2 : /자율 결제|autopay|자동 잠금|auto-lock/i.test(msg) ? 1 : /한도|limit/i.test(msg) ? 0 : -1;
        if (g >= 0) setNavSel(g);
      }
      setBusy(false);
    }
  }

  // 수동 닫기 — 닫힘 표시 후 자동 닫기 타이머를 정리하고 닫는다(이중 onClose·늦은 setState 방지).
  function close() {
    closed.current = true;
    if (closeTimer.current) clearTimeout(closeTimer.current);
    onClose();
  }

  // 큰 창 — 왼쪽 묶음 목록에서 고른 묶음 하나만 오른쪽에. 값은 폼 하나라 묶음을 옮겨도 그대로다.
  const groups = [t("한도", "Limits"), t("자율 결제", "Autopay"), t("네트워크", "Network"), t("RPC 서버", "RPC server"), t("앱", "App"), t("정보", "About")];
  // «앱» = 언어를 바꿔 다시 읽은 뒤 그 묶음으로 돌아올 때(WalletScreen).
  const [navSel, setNavSel] = useState(() => (initialGroup === "app" ? Math.max(0, groups.indexOf(t("앱", "App"))) : 0));
  return (
    <NavCtx.Provider value={isWindow ? groups[navSel] : null}>
    <main className={pageShell}>
      <div className={cn(pageColumn, "flex flex-col gap-3 win:gap-8")}>
        <ScreenHeader
          icon={<SettingsIcon size={12} />}
          title={t("설정", "Settings")}
          sub={t("바꾼 뒤 맨 아래 저장을 눌러야 적용돼요.", "Changes apply when you press Save at the bottom.")}
          onClose={close}
          closeDisabled={busy}
        />

        <div className="win:flex win:gap-10 min-[1100px]:win:gap-14 win:items-start">
        {isWindow && (
          <nav className="w-[200px] shrink-0 sticky top-14" aria-label={t("설정 묶음", "Settings groups")}>
            <ul className="flex flex-col gap-0.5">
              {groups.map((g, i) => (
                <li key={g}>
                  <Selectable on={navSel === i} onClick={() => setNavSel(i)} className="px-3 py-2">
                    <span className={cn("text-[14px]", navSel === i ? cn(ink, "font-semibold") : inkSoft)}>{g}</span>
                  </Selectable>
                </li>
              ))}
            </ul>
          </nav>
        )}
        <div className="flex flex-col gap-3 win:flex-1 win:min-w-0 win:max-w-[680px]">
        {!s ? (
          <motion.section {...enter} className={cn(cardBase, "px-6 py-10")}>
            <div className="flex flex-col items-center">
              <Loader2 size={24} className="animate-spin text-[var(--color-accent)]" />
              <p className="mt-3 text-[13px] text-[var(--color-ink-500)]">
                {t("설정 불러오는 중…", "Loading settings…")}
              </p>
            </div>
          </motion.section>
        ) : (
          // 저장 직후 자동 닫기 대기(busy) 동안 폼 전체를 잠가, 그 사이 입력이 조용히 버려지지 않게 한다.
          <fieldset disabled={busy} className="flex flex-col gap-3 win:gap-0 border-0 p-0 m-0 min-w-0">
            {/* 한도 — 단일/하루 누적 한도(USDC) */}
            <Section
              icon={<Gauge size={13} className="text-[var(--color-accent)]" />}
              title={t("한도", "Limits")}
              delay={0}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "단일 거래·하루 누적 한도를 넘으면 결제를 막아줘요. AI의 큰 지출을 막기 위한 안전 장치예요.",
                  "Anything over the per-payment or daily limit is blocked. This is the guardrail against a big spend by the AI.",
                )}
              </p>
              <p className="mt-1.5 text-[11px] text-[var(--color-ink-300)]">
                {t("0으로 두면 무제한이에요.", "Leave a limit at 0 for no limit.")}
              </p>

              <div className="mt-5 space-y-3">
                <LimitGroup
                  token="USDC"
                  single={s.single_usdc}
                  daily={s.daily_usdc}
                  onSingle={(v) => field("single_usdc", v)}
                  onDaily={(v) => field("daily_usdc", v)}
                  usedToday={spend?.usdc}
                />
              </div>
            </Section>

            {/* 자율 결제 — 한도 이하 AI 결제를 비번 없이 자동 승인 */}
            <Section
              icon={<Zap size={13} className="text-[var(--color-accent)]" />}
              title={t("자율 결제", "Autopay")}
              delay={0.04}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "이 금액 이하의 AI 결제 요청은, 세션을 잠금 해제해 두면 비번 없이 자동 승인돼요. 넘는 금액은 항상 비번을 받아요.",
                  "With the session unlocked, AI payments at or under this amount go through without your password. Anything larger always asks.",
                )}
              </p>
              <p className="mt-1.5 text-[11px] text-[var(--color-ink-300)]">
                {t(
                  "한도를 0으로 두면 자율 결제가 꺼져요(항상 비번). 단일·하루 한도는 그대로 함께 적용돼요.",
                  "Set the limit to 0 to turn autopay off (always ask). The per-payment and daily limits still apply on top.",
                )}
              </p>

              <div className="mt-5 rounded-[var(--radius-card)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] px-4 py-4 win:border-0 win:p-0">
                <div className="grid grid-cols-2 gap-3">
                  <UnitInput
                    label={t("자율 한도", "Autopay limit")}
                    unit="USDC"
                    value={s.auto_approve_usdc}
                    onChange={(v) => field("auto_approve_usdc", v)}
                  />
                  <UnitInput
                    label={t("자동 잠금", "Auto-lock")}
                    unit={t("분", "min")}
                    intOnly
                    value={s.auto_lock_mins}
                    onChange={(v) => field("auto_lock_mins", v)}
                  />
                </div>
                <p className="mt-2.5 text-[11px] text-[var(--color-ink-300)]">
                  {t(
                    "유휴 시간이 지나면 자동으로 다시 잠겨요. 0이면 유휴 잠금 안 함(앱 종료·긴급 잠금 시엔 항상 잠김).",
                    "The session locks itself after this much idle time. 0 means no idle lock (quitting the app or the emergency lock still locks it).",
                  )}
                </p>
              </div>

              {/* 자율 결제 부속 토글·목록 — 낱개 카드 대신 구분선 리스트 하나로.
                  상자 안 상자가 다섯 겹 쌓이던 걸 한 컨테이너로 접어 시각 소음을 줄인다. */}
              <RowGroup>
                {/* 자리비움 자동 잠금 */}
                <ToggleRow
                  title={t("자리 비우면 자동 잠금", "Lock when you step away")}
                  desc={t(
                    "다른 앱으로 전환하거나 화면이 잠기면 세션을 즉시 잠가요.",
                    "Switching to another app or locking your screen locks the session at once.",
                  )}
                  checked={s.lock_on_blur}
                  onToggle={() => toggle("lock_on_blur")}
                />
                {/* 자율 결제 알림 — 비번 없이 나간 돈을 보호자가 사후 인지 */}
                <ToggleRow
                  title={t("자율 결제 알림", "Autopay notifications")}
                  desc={t(
                    "비번 없이 자동 승인된 결제를 macOS 알림으로 알려줘요.",
                    "A macOS notification tells you about payments approved without your password.",
                  )}
                  checked={s.notify_auto}
                  onToggle={() => toggle("notify_auto")}
                />
                {/* 알림 금액 숨기기 — macOS 알림은 잠금 화면·화면 공유에도 뜬다.
                    알림이 꺼져 있으면 의미 없는 토글이라 행 자체를 접는다. */}
                {s.notify_auto && (
                  <ToggleRow
                    title={t("알림에 금액 숨기기", "Hide amounts in notifications")}
                    desc={t(
                      "잠금 화면·화면 공유에 금액이 보이지 않게 「자율 결제」로만 알려줘요.",
                      "Notifications say just “Autopay,” so amounts don't show on the lock screen or a shared screen.",
                    )}
                    checked={s.notify_hide_amount}
                    onToggle={() => toggle("notify_hide_amount")}
                  />
                )}
                {/* 자율 결제 화이트리스트 — 처음 보는 주소는 자율 대상에서 제외 */}
                <ToggleRow
                  title={t("화이트리스트에만 자율 결제", "Autopay to the whitelist only")}
                  desc={t(
                    "비번으로 승인한 적 있는 주소에만 자율 결제를 허용해요. 새 주소는 첫 결제만 비번이 필요해요.",
                    "Autopay only goes to addresses you've approved with your password before. A new address needs your password once.",
                  )}
                  checked={s.auto_trusted_only}
                  onToggle={() => toggle("auto_trusted_only")}
                />
                {/* 화이트리스트 — 목록은 모달로 (많아지면 설정이 길어지니) */}
                <div className="flex items-center justify-between px-4 py-3.5">
                  <div className="min-w-0 pr-3">
                    <p className="text-[13px] tracking-tight">{t("화이트리스트", "Whitelist")}</p>
                    <p className="mt-0.5 text-[11px] leading-snug text-[var(--color-ink-300)]">
                      {trusted === null
                        ? t("불러오는 중…", "Loading…")
                        : trusted.length === 0
                          ? t(
                              "아직 없어요 — 비번으로 승인하면 자동으로 학습돼요.",
                              "None yet — approving with your password adds one.",
                            )
                          : t(`${trusted.length}개 학습됨`, `${trusted.length} learned`)}
                    </p>
                  </div>
                  <button
                    type="button"
                    onClick={() => setShowTrusted(true)}
                    disabled={!trusted || trusted.length === 0}
                    className="shrink-0 text-[12px] text-[var(--color-accent)] disabled:text-[var(--color-ink-300)] hover:underline transition-colors"
                  >
                    {t("주소 보기", "View")}
                  </button>
                </div>
              </RowGroup>
            </Section>

            {/* 네트워크 — 데브넷 ↔ 메인넷 런타임 전환.
                메인넷 선택 시엔 실돈 경고를 콜아웃으로 올려 위계를 준다. */}
            <Section
              icon={<Network size={13} className="text-[var(--color-accent)]" />}
              title={t("네트워크", "Network")}
              delay={0.08}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "잔액·결제가 이뤄지는 블록체인이에요. 메인넷은 실제 자금이 오가고, 데브넷은 테스트 코인으로 연습해요. 사용액·내역·화이트리스트는 네트워크별로 따로, 한도는 모든 네트워크에 같이 적용돼요.",
                  "The blockchain your balance and payments live on. Mainnet moves real funds; devnet is practice with test coins. Spending, history, and the whitelist are kept per network; limits apply to every network.",
                )}
              </p>
              <div className="mt-4 grid grid-cols-2 gap-2">
                {CHAINS.map((c) => {
                  const active = c.id === s.chain_id;
                  return (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => selectChain(c.id)}
                      className={cn(
                        "rounded-[var(--radius-card)] border px-2 py-3 text-[12px] tracking-tight transition-colors",
                        active
                          ? "border-[var(--color-accent)] bg-[var(--color-accent)]/10 text-[var(--color-accent)] font-medium"
                          : "border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] text-[var(--color-ink-500)] hover:border-[var(--color-ink-300)]",
                      )}
                    >
                      {c.name}
                      <span className="block mt-0.5 text-[10px] text-[var(--color-ink-300)]">
                        {c.testnet ? t("연습용", "Practice") : t("실제 자금", "Real funds")}
                      </span>
                    </button>
                  );
                })}
              </div>
              {chain.testnet ? (
                <p className="mt-2.5 text-[11px] text-[var(--color-ink-300)]">
                  {t("바꾼 뒤 저장을 누르면 적용돼요.", "Press save to apply the change.")}
                </p>
              ) : (
                // 실돈 경고는 문장 한 줄이 아니라 콜아웃 — 일상 설정과 위험 설정의 시각 위계 분리.
                <div className="mt-2.5 rounded-[var(--radius-card)] border border-amber-500/30 bg-amber-500/5 px-3.5 py-2.5">
                  <p className="text-[11px] leading-relaxed text-amber-600 dark:text-amber-500">
                    {t(
                      "메인넷은 실제 USDC가 오가요. 운영 예산만 소액 충전하고, 위의 한도를 확인해 두세요. 바꾼 뒤 저장을 누르면 적용돼요.",
                      "Mainnet moves real USDC. Top up only what the agent needs, and check your limits above. Press save to apply the change.",
                    )}
                  </p>
                </div>
              )}
            </Section>

            {/* RPC 서버 — 프라이버시: 공개 RPC는 내 IP↔지갑을 볼 수 있음 */}
            <Section
              icon={<Globe size={13} className="text-[var(--color-accent)]" />}
              title={t("RPC 서버", "RPC server")}
              delay={0.12}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "잔액 조회·결제 준비(블록해시)에 쓰는 솔라나 서버예요. 공개 RPC는 내 IP와 지갑 주소를 볼 수 있고 요청 제한도 빡빡해요. 메인넷에서 쓰거나 프라이버시가 중요하면 본인 키 엔드포인트(Helius·QuickNode·Triton 등)를 직접 넣으세요.",
                  "The Solana server used to read balances and prepare payments (recent blockhash). A public RPC can see your IP and wallet address and is tightly rate-limited. On mainnet, or if privacy matters, paste your own endpoint (Helius, QuickNode, Triton and so on).",
                )}
              </p>
              <select
                value={rpcCustom ? RPC_CUSTOM : presetUrl(s.rpc_url, chain)}
                onChange={(e) => {
                  const v = e.target.value;
                  if (v === RPC_CUSTOM) {
                    setRpcCustom(true);
                  } else {
                    setRpcCustom(false);
                    field("rpc_url", v);
                  }
                }}
                className={cn(inputBase, "mt-4 text-[13px]")}
              >
                {presets.map((p) => (
                  <option key={p.label} value={p.url}>
                    {p.label}
                  </option>
                ))}
                <option value={RPC_CUSTOM}>{t("직접 입력…", "Enter your own…")}</option>
              </select>
              {rpcCustom && (
                <input
                  value={s.rpc_url}
                  onChange={(e) => field("rpc_url", e.target.value)}
                  placeholder="https://…"
                  spellCheck={false}
                  autoCapitalize="off"
                  className={cn(inputBase, "mt-2 font-mono text-[12px]")}
                />
              )}
            </Section>

            {/* 앱 — 상시 구동: 창을 닫아도 백그라운드에서 결제 요청을 받는다 */}
            <Section
              icon={<Power size={13} className="text-[var(--color-accent)]" />}
              title={t("앱", "App")}
              delay={0.16}
            >
              <p className="text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "창을 닫아도 앱은 종료되지 않고 백그라운드에서 결제 요청을 받아요 — 요청이 오면 창이 다시 떠요. 완전히 끄려면 ⌘Q를 누르세요.",
                  "Closing the window doesn't quit Budu — it keeps taking payment requests in the background and brings the window back when one arrives. Press ⌘Q to quit for real.",
                )}
              </p>
              <RowGroup>
                <ToggleRow
                  title={t("로그인 시 자동 시작", "Start at login")}
                  desc={t(
                    "Mac에 로그인하면 지갑이 자동으로 켜져요. 끄고 켜는 즉시 적용돼요.",
                    "The wallet opens when you log in to your Mac. This applies the moment you flip it.",
                  )}
                  checked={autostart ?? false}
                  onToggle={toggleAutostart}
                />
                <ToggleRow
                  title={t("메뉴바에 두기", "Show in menu bar")}
                  desc={t(
                    "메뉴바 아이콘에서 잔액과 진행 중인 작업을 바로 봐요. 결제 승인은 늘 이 창에서 해요.",
                    "See your balance and the running job from the menu bar. Payments are always approved in this window.",
                  )}
                  checked={menubar ?? true}
                  onToggle={toggleMenubar}
                />
                <LanguageRow dirty={dirty} />
              </RowGroup>
            </Section>

            {error && <p className="text-[12px] text-red-500 font-mono break-all win:mb-3">{error}</p>}

            <button type="button" onClick={save} disabled={busy} className={cn(primaryBtn, "mt-2 win:w-auto win:px-8 win:self-start", isWindow && navSel === groups.length - 1 && "hidden")}>
              {saved ? (
                <Check size={15} />
              ) : busy ? (
                <Loader2 size={15} className="animate-spin" />
              ) : null}
              {saved ? t("저장됨", "Saved") : t("저장하고 닫기", "Save and close")}
            </button>
          </fieldset>
        )}

        {/* 정보 — 읽기 전용이라 fieldset(저장 중 잠금) 밖에 두고, 설정 로드가 실패해도 보이게
            조건 분기 바깥에 둔다. 버전·소스 링크는 "이 앱이 뭘 하는지 직접 확인할 통로"라
            설정이 안 열리는 상황일수록 오히려 더 필요하다. */}
        {/* 폴백 false = 신규 기본. 실제 값은 백엔드 설정이 로드되면 그걸 따른다. */}
        <AboutSection />
        </div>
        </div>
      </div>

      <AnimatePresence>
        {showTrusted && trusted && (
          <TrustedAddrsModal
            addrs={trusted}
            onRevoke={revokeTrusted}
            onClose={() => setShowTrusted(false)}
          />
        )}
      </AnimatePresence>
    </main>
    </NavCtx.Provider>
  );
}
