// 설정 화면 — 한도·자율 결제·화이트리스트·네트워크·RPC·앱(상시 구동/자동 시작).
// 도메인별 그룹 카드 스택(토스/Apple 설정 관용구) — 길게 쌓이던 한 덩어리를 끊어 스캔 가능하게.

import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { Check, Gauge, Globe, Loader2, Network, Power, Settings as SettingsIcon, Zap } from "lucide-react";
import { cn } from "@/lib/cn";
import { CHAINS, chainFromId, type ChainConfig } from "@/lib/chain";
import type { NetLimits, Settings } from "@/lib/types";
import { cardBase, inputBase, pageColumn, pageShell, primaryBtn, ScreenHeader , enter } from "@/components/ui";
import { isWindow } from "@/lib/win";
import { Selectable, ink, inkSoft } from "@/components/win";
import { holdLangReload, t } from "@/lib/i18n";
import {
  NavCtx,
  Section,
  AboutSection,
  TrustedList,
  RowGroup,
  ToggleRow,
  LockRow,
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
  | "auto_trusted_only"
  | "auto_learn_trusted";

// 한도 셋 — 체인별(`limits`)이 정본이고 평평한 세 칸은 백엔드가 활성 체인 값으로 채우는 보기 칸이라 보내지 않는다.
const LIMIT_KEYS = ["single_usdc", "daily_usdc", "auto_approve_usdc"] as const;
type LimitKey = (typeof LIMIT_KEYS)[number];

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
  onClose,
  onSaved,
  onGuard,
  initialGroup,
}: {
  current: Settings | null;
  onClose: () => void;
  /** 저장이 끝났다 — 부모가 설정·한도를 다시 읽는다(화면은 설정에 머문다). */
  onSaved: () => void;
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
  // 화이트리스트 주소 목록 — null=로딩 중. 추가·철회는 즉시 반영(저장 버튼과 무관).
  const [trusted, setTrusted] = useState<string[] | null>(null);
  // 로그인 시 자동 시작 — OS 로그인 아이템 상태가 진실 원천이라 즉시 적용(저장 버튼과 무관).
  const [autostart, setAutostart] = useState<boolean | null>(null);
  // 메뉴바 아이콘 — 앱 관리 필드라 저장 버튼과 무관하게 즉시 적용.
  const [menubar, setMenubar] = useState<boolean | null>(null);
  // «저장됨» 을 거두는 타이머 — 언마운트 때 정리.
  const savedTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  // 이미 닫혔는지 — 저장 invoke 가 늦게 resolve 돼도(닫기를 그 사이 눌렀을 때) 늦은 상태 변경을 막는다.
  const closed = useRef(false);

  // 화이트리스트는 체인별 — 저장된 네트워크가 바뀌면(이 창의 저장이든 다른 창이든) 다시 읽는다.
  const savedChain = current?.chain_id;
  useEffect(() => {
    let live = true; // 네트워크가 연달아 바뀌면 늦게 온 옛 목록을 버린다
    setTrusted(null);
    invoke<string[]>("get_trusted_addrs")
      .then((a) => live && setTrusted(a))
      .catch(() => live && setTrusted([]));
    return () => {
      live = false;
    };
  }, [savedChain]);

  useEffect(() => {
    invoke<boolean>("get_autostart")
      .then(setAutostart)
      .catch(() => setAutostart(false));
    invoke<boolean>("get_menubar")
      .then(setMenubar)
      .catch(() => setMenubar(true));
  }, []);

  // 언마운트 시 닫힘 표시 + 타이머 정리.
  // 마운트 때 false 로 되돌린다 — 개발 모드(StrictMode)는 effect 를 한 번 걷었다 다시 붙여서, 되돌리지 않으면
  // 첫 저장부터 «닫힘» 으로 보고 busy 를 영영 안 푼다.
  useEffect(() => {
    closed.current = false;
    return () => {
      closed.current = true;
      if (savedTimer.current) clearTimeout(savedTimer.current);
    };
  }, []);

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
      await invoke("remove_trusted_addr", { to: addr, chainId: savedChain });
      setTrusted((prev) => (prev ? prev.filter((a) => a !== addr) : prev));
    } catch {
      /* 철회 실패는 목록 유지 — 다음 시도에서 다시 */
    }
  }

  // 직접 추가 — 검증(32바이트 base58)은 백엔드가 한다. 실패하면 이유를 그대로 돌려준다.
  async function addTrusted(addr: string): Promise<string | null> {
    const a = addr.trim();
    try {
      await invoke("add_trusted_addr", { to: a, chainId: savedChain });
      setTrusted((prev) => (prev && !prev.includes(a) ? [...prev, a] : prev));
      return null;
    } catch (e) {
      return String(e);
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

  // 저장 직후 아직 옛 current 가 내려와 있는 동안은 갈아 끼우지 않는다 — 저장한 값이 잠깐 옛 값으로 돌아가 보이고,
  // 그 사이 고치면 옛 체인을 기준으로 보내 «다른 창에서 네트워크가 바뀌었어요» 로 거절됐다.
  const staleCurrent = useRef<Settings | null>(null);
  // 다른 창이 저장해 current 가 바뀌었고 이 폼은 손대지 않았다면 새 값으로 갈아 끼운다.
  useEffect(() => {
    if (current && current === staleCurrent.current) return;
    staleCurrent.current = null;
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

  // 지금 고른 네트워크의 한도 — 없으면(옛 백엔드) 평평한 칸.
  const lim: NetLimits | null = s
    ? (s.limits?.[String(s.chain_id)] ?? { single_usdc: s.single_usdc, daily_usdc: s.daily_usdc, auto_approve_usdc: s.auto_approve_usdc })
    : null;
  const autopayOff = !lim || !(Number(lim.auto_approve_usdc) > 0);
  function limitField(key: LimitKey, value: string) {
    setS((prev) => {
      if (!prev) return prev;
      const id = String(prev.chain_id);
      const cur = prev.limits?.[id] ?? { single_usdc: prev.single_usdc, daily_usdc: prev.daily_usdc, auto_approve_usdc: prev.auto_approve_usdc };
      return { ...prev, limits: { ...prev.limits, [id]: { ...cur, [key]: value } } };
    });
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
      const changed: string[] = (Object.keys(s) as (keyof Settings)[]).filter(
        (k) => k !== "limits" && !(LIMIT_KEYS as readonly string[]).includes(k) && (!from || s[k] !== from[k]),
      );
      // 네트워크를 바꾼 저장이면 RPC 는 늘 보낸다 — 옛 체인용 커스텀 RPC(다른 창이 넣었을 수도)가 새 체인에 남지 않게.
      if (from && s.chain_id !== from.chain_id && !changed.includes("rpc_url")) changed.push("rpc_url");
      // 체인별 한도는 칸 하나씩 — 다른 창이 고친 다른 네트워크 한도를 되돌리지 않게.
      for (const [id, l] of Object.entries(s.limits ?? {})) {
        for (const k of LIMIT_KEYS) {
          if (!from || from.limits?.[id]?.[k] !== l[k]) changed.push(`limits.${id}.${k}`);
        }
      }
      await invoke("save_settings_edit", {
        form: s,
        changed,
        baseChainId: from?.chain_id ?? s.chain_id,
      });
      if (closed.current) return; // 저장 도중 닫혔으면 늦은 setState 를 막는다
      // 설정에 머문다. 폼 기준을 지금 값으로 — 부모가 다시 읽어 온 값(백엔드가 정규화한 것)이 오면 위 effect 가 갈아 끼운다.
      base.current = s;
      staleCurrent.current = current;
      setSaved(true);
      setDirty(false);
      setBusy(false);
      onSaved();
      if (savedTimer.current) clearTimeout(savedTimer.current);
      savedTimer.current = setTimeout(() => setSaved(false), 1600);
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

  // 수동 닫기 — 닫힘 표시 후 닫는다(늦은 setState 방지).
  function close() {
    closed.current = true;
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
                    <span className={cn("text-md", navSel === i ? cn(ink, "font-semibold") : inkSoft)}>{g}</span>
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
              <Loader2 size={24} className="animate-spin text-[var(--color-ok)]" />
              <p className="mt-3 text-md text-[var(--color-ink-500)]">
                {t("설정 불러오는 중…", "Loading settings…")}
              </p>
            </div>
          </motion.section>
        ) : (
          // 저장 중(busy)엔 폼 전체를 잠가, 그 사이 입력이 조용히 버려지지 않게 한다.
          <fieldset disabled={busy} className="flex flex-col gap-3 win:gap-0 border-0 p-0 m-0 min-w-0">
            {/* 한도 — 단일/하루 누적 한도(USDC) */}
            <Section
              icon={<Gauge size={13} className="text-[var(--color-ok)]" />}
              title={t("한도", "Limits")}
              delay={0}
            >
              <p className="text-md leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "한도를 넘으면 결제를 막아줍니다. (0으로 두면 한도 무제한)",
                  "Payments over a limit are blocked. (0 means no limit.)",
                )}
              </p>

              <div className="mt-5">
                <LimitGroup
                  token="USDC"
                  single={lim!.single_usdc}
                  daily={lim!.daily_usdc}
                  onSingle={(v) => limitField("single_usdc", v)}
                  onDaily={(v) => limitField("daily_usdc", v)}
                />
              </div>
            </Section>

            {/* 자율 결제 — 한도 이하 AI 결제를 비번 없이 자동 승인 */}
            <Section
              icon={<Zap size={13} className="text-[var(--color-ok)]" />}
              title={t("자율 결제", "Autopay")}
              delay={0.04}
            >
              <p className="text-md leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "설정 금액 이하의 결제를 자동으로 승인합니다. (0으로 두면 상시 비밀번호)",
                  "Payments up to the amount you set are approved automatically. (0 means always ask for the password.)",
                )}
              </p>

              <div className="mt-5 max-w-[calc(50%-6px)]">
                <UnitInput
                  label={t("자율 결제 금액", "Autopay amount")}
                  unit="USDC"
                  value={lim!.auto_approve_usdc}
                  onChange={(v) => limitField("auto_approve_usdc", v)}
                />
              </div>

              {/* 잠금·알림은 자율 결제에만 의미가 있다 — 금액이 0(꺼짐)이면 흐리게. 미리 맞춰 둘 수는 있다. */}
              <div className={cn("transition-opacity duration-[var(--duration-base)]", autopayOff && "opacity-40")}>
                <RowGroup>
                  <LockRow value={s.auto_lock_mins} onChange={(v) => field("auto_lock_mins", v)} />
                  <ToggleRow
                    title={t("자리 비우면 자동 잠금", "Lock when you step away")}
                    desc={t(
                      "다른 앱으로 전환하거나 화면이 전환되면 세션을 즉시 잠가 보호합니다.",
                      "Switching to another app or another screen locks the session at once.",
                    )}
                    checked={s.lock_on_blur}
                    onToggle={() => toggle("lock_on_blur")}
                  />
                  <ToggleRow
                    title={t("자율 결제 알림", "Autopay notifications")}
                    desc={t(
                      "자동 승인된 결제를 macOS 알림으로 알려줍니다.",
                      "A macOS notification tells you about payments approved automatically.",
                    )}
                    checked={s.notify_auto}
                    onToggle={() => toggle("notify_auto")}
                  />
                  {/* 알림이 꺼져 있으면 의미 없는 토글이라 행 자체를 접는다. */}
                  {s.notify_auto && (
                    <ToggleRow
                      title={t("알림에 금액 숨기기", "Hide amounts in notifications")}
                      desc={t("알림에 금액이 보이지 않게 합니다.", "Notifications don't show the amount.")}
                      checked={s.notify_hide_amount}
                      onToggle={() => toggle("notify_hide_amount")}
                    />
                  )}
                  <ToggleRow
                    title={t("화이트리스트에만 자율 결제", "Autopay to the whitelist only")}
                    desc={t(
                      "화이트리스트로 등록한 주소에만 자율 결제를 허용합니다.",
                      "Autopay only goes to addresses on your whitelist.",
                    )}
                    checked={s.auto_trusted_only}
                    onToggle={() => toggle("auto_trusted_only")}
                  />
                </RowGroup>
              </div>

              <TrustedList
                addrs={trusted}
                onAdd={addTrusted}
                onRevoke={revokeTrusted}
                autoLearn={s.auto_learn_trusted ?? false}
                onToggleAutoLearn={() => toggle("auto_learn_trusted")}
                locked={s.chain_id !== savedChain}
              />
            </Section>

            {/* 네트워크 — 데브넷 ↔ 메인넷 런타임 전환.
                메인넷 선택 시엔 실돈 경고를 콜아웃으로 올려 위계를 준다. */}
            <Section
              icon={<Network size={13} className="text-[var(--color-ok)]" />}
              title={t("네트워크", "Network")}
              delay={0.08}
            >
              <div className="grid grid-cols-2 gap-2">
                {CHAINS.map((c) => {
                  const active = c.id === s.chain_id;
                  return (
                    <button
                      key={c.id}
                      type="button"
                      onClick={() => selectChain(c.id)}
                      className={cn(
                        "rounded-[var(--radius-card)] border px-2 py-3 text-sm tracking-tight transition-colors",
                        active
                          ? "border-[var(--color-ok)] bg-[var(--color-ok)]/10 text-[var(--color-ok)] font-medium"
                          : "border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] text-[var(--color-ink-500)] hover:border-[var(--color-ink-300)]",
                      )}
                    >
                      {c.name}
                      <span className="block mt-0.5 text-sm text-[var(--color-ink-300)]">
                        {c.testnet ? t("연습용", "Practice") : t("실제 자금", "Real funds")}
                      </span>
                    </button>
                  );
                })}
              </div>
              {!chain.testnet && (
                <p className="mt-2.5 text-sm text-[var(--color-danger)]">
                  {t("메인넷은 실제 USDC가 오가요.", "Mainnet moves real USDC.")}
                </p>
              )}
            </Section>

            {/* RPC 서버 — 프라이버시: 공개 RPC는 내 IP↔지갑을 볼 수 있음 */}
            <Section
              icon={<Globe size={13} className="text-[var(--color-ok)]" />}
              title={t("RPC 서버", "RPC server")}
              delay={0.12}
            >
              <p className="text-md leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  "메인넷이나 프라이버시가 중요하면 본인 엔드포인트(Helius·QuickNode 등)를 넣으세요.",
                  "On mainnet, or if privacy matters, use your own endpoint (Helius, QuickNode and so on).",
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
                className={cn(inputBase, "mt-4 text-md")}
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
                  className={cn(inputBase, "mt-2 font-mono text-sm")}
                />
              )}
            </Section>

            {/* 앱 — 상시 구동: 창을 닫아도 백그라운드에서 결제 요청을 받는다 */}
            <Section
              icon={<Power size={13} className="text-[var(--color-ok)]" />}
              title={t("앱", "App")}
              delay={0.16}
            >
              <RowGroup>
                <ToggleRow
                  title={t("로그인 시 자동 시작", "Start at login")}
                  checked={autostart ?? false}
                  onToggle={toggleAutostart}
                />
                <ToggleRow
                  title={t("메뉴바에 두기", "Show in menu bar")}
                  checked={menubar ?? true}
                  onToggle={toggleMenubar}
                />
                <LanguageRow dirty={dirty} />
              </RowGroup>
            </Section>

            {error && <p className="text-sm text-[var(--color-danger)] font-mono break-all win:mb-3">{error}</p>}

            <div className={cn("mt-2 flex items-center gap-3", isWindow && navSel === groups.length - 1 && "hidden")}>
              <button type="button" onClick={save} disabled={busy || !dirty} className={cn(primaryBtn, "win:w-auto win:px-8")}>
                {busy && <Loader2 size={15} className="animate-spin" />}
                {t("저장하기", "Save")}
              </button>
              <AnimatePresence>
                {saved && (
                  <motion.span
                    initial={{ opacity: 0 }}
                    animate={{ opacity: 1 }}
                    exit={{ opacity: 0 }}
                    transition={{ duration: 0.18 }}
                    className="shrink-0 inline-flex items-center gap-1 text-sm text-[var(--color-ink-500)]"
                  >
                    <Check size={13} />
                    {t("저장됨", "Saved")}
                  </motion.span>
                )}
              </AnimatePresence>
            </div>
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

    </main>
    </NavCtx.Provider>
  );
}
