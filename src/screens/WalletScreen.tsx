// 지갑 메인 화면 — 1초 폴링(AI 연결·세션·결제 요청)과 화면 고르기.
// 큰 창 = 레일 + 장부 페이지(홈·작업·내역·판매자). 팝오버 = 미니(잔액·작업 바·「창 열기」) — 나머지는 큰 창에서.

import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { AnimatePresence, motion } from "framer-motion";
import { Maximize2, PanelRight, ShieldAlert, ShieldCheck } from "lucide-react";
import type {
  Account,
  AgentStatus,
  Balances,
  HistoryEntry,
  JobView,
  PaymentRequest,
  SessionStatus,
  Settings,
  SpendView,
} from "@/lib/types";
import { cn } from "@/lib/cn";
import { accountName } from "@/lib/format";
import { chainFromId, ChainProvider } from "@/lib/chain";
import { useCopy } from "@/lib/useCopy";
import { shell, modalOverlay, modalCard, secondaryBtn, ActionButton, HeaderIconButton } from "@/components/ui";
import { Rail, WindowHome, type RailPage } from "@/screens/WindowHome";
import { BalanceCard } from "@/components/BalanceCard";
import { ReceiveCard } from "@/components/ReceiveCard";
import { SendSolSheet } from "@/components/SendSolSheet";
import { PaymentApprovalModal } from "@/components/PaymentApprovalModal";
import { BackupNag, LockBanner, SettingsBrokenBanner } from "@/components/banners";
import { SessionBar, UnlockSessionModal } from "@/components/SessionBar";
import { JobBar } from "@/components/JobBar";
import { BackupFlow } from "@/screens/BackupFlow";
import { ConnectScreen } from "@/screens/ConnectScreen";
import { HelpScreen } from "@/screens/HelpScreen";
import { JobPanel } from "@/screens/ledger/JobPanel";
import { LedgerPage } from "@/screens/ledger/LedgerPage";
import { SellersPage } from "@/screens/ledger/SellersPage";
import type { Scope } from "@/screens/ledger/parts";
import { IconButton, useMedia, usePref } from "@/components/win";
import { SettingsScreen } from "@/screens/SettingsScreen";
import { t, takeReturnPage } from "@/lib/i18n";
import { isApprover, isWindow } from "@/lib/win";
import { notifySync, useSync } from "@/lib/sync";
import { ApprovalElsewhereBanner } from "@/components/banners";

type Mode = "balance" | "receive" | "sendSol";


/** 언어를 바꿔 다시 읽은 큰 창이면 설정으로 돌아간다 — 모듈이 읽힐 때 한 번 꺼내 두고, 첫 마운트가 한 번만 쓴다
 *  (StrictMode 의 두 번 부르기·나중 재마운트에 다시 설정이 열리지 않게). */
let pendingReturn: string | null = isWindow ? takeReturnPage() : null;
function returnToSettings(): boolean {
  return pendingReturn?.startsWith("settings") ?? false;
}

/** 내역 화면이 한 번에 읽는 줄 수. */
const HISTORY_PAGE = 200;
export function WalletScreen({
  address,
  initialBackedUp,
  accounts,
  active,
}: {
  /** 활성 계정의 주소 — 계정을 바꾸면 이 값이 바뀌고 잔액·내역이 따라온다. */
  address: string;
  initialBackedUp: boolean;
  accounts: Account[];
  active: number;
}) {
  const [mode, setMode] = useState<Mode>("balance");
  // 활성 계정 객체 — 목록에 없으면(있을 수 없지만) 주소만으로 만든다.
  const account: Account =
    accounts.find((a) => a.index === active) ?? { index: active, address, label: "" };
  const [copied, copy] = useCopy();
  const [balances, setBalances] = useState<Balances | null>(null);
  const [balanceError, setBalanceError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [backedUp, setBackedUp] = useState(initialBackedUp);
  // 다른 창에서 백업을 마쳤으면 App 이 다시 읽은 값이 여기로 내려온다. 내려가는 쪽(true → false)은
  // 없다 — 백업 표식은 지우는 길이 없다.
  useEffect(() => {
    if (initialBackedUp) setBackedUp(true);
  }, [initialBackedUp]);
  const [showBackup, setShowBackup] = useState(false);
  // 설정에서 언어를 바꿔 창을 다시 읽었으면 설정으로 돌아온다(큰 창에서만 — 팝오버엔 설정이 없다).
  const [showSettings, setShowSettings] = useState(() => returnToSettings());
  // 언어를 «앱» 묶음에서 바꿨으면 그 묶음으로 돌아간다.
  const [settingsGroup, setSettingsGroup] = useState<"app" | undefined>(() => (pendingReturn === "settings:app" ? "app" : undefined));
  useEffect(() => {
    pendingReturn = null;
  }, []);
  const [settings, setSettings] = useState<Settings | null>(null);
  // 설정 파일이 있는데 못 읽는 중인가 — 그때 위 settings 는 기본값이고, 그 사실을 배너로 알린다.
  const [settingsBroken, setSettingsBroken] = useState(false);
  const [spend, setSpend] = useState<SpendView | null>(null);
  const [locked, setLocked] = useState(false);
  const [showHistory, setShowHistory] = useState(false);
  const [history, setHistory] = useState<HistoryEntry[] | null>(null);
  const [showHelp, setShowHelp] = useState(false);
  const [showConnect, setShowConnect] = useState(false);
  // 큰 창의 판매자 페이지. 작업은 개발 9 에 내역(장부)과 합쳤다 — 장부의 왼쪽 단에서 고른다.
  const [ledgerScope, setLedgerScope] = useState<Scope>("all");
  const [showSellers, setShowSellers] = useState(false);
  // AI 에이전트(MCP)가 보낸 결제 승인 요청. 1초마다 폴링한다.
  const [pending, setPending] = useState<PaymentRequest | null>(null);
  // AI(MCP 클라이언트)가 지금 이 지갑에 연결돼 있는지 — 메인 화면 배지용.
  const [agent, setAgent] = useState<AgentStatus>({ connected: false, client: "" });
  // 「준비됨」 — Claude 를 켜면 이 앱으로 붙는 등록이 있는가. null = 아직 모름.
  // 하트비트(1초)와 따로 5초마다 — 설정 파일 읽기라 싸지만 매초 볼 만큼 자주 바뀌지 않는다.
  const [ready, setReady] = useState<boolean | null>(null);
  const loadReady = useCallback(() => {
    invoke<boolean>("get_connect_ready")
      .then(setReady)
      .catch(() => {});
  }, []);
  useEffect(() => {
    loadReady();
    const h = setInterval(loadReady, 5000);
    return () => clearInterval(h);
  }, [loadReady]);
  // 자율 결제 세션 상태(메모리의 잠금 해제 키). 잠금 해제 시 한도 이하는 비번 없이 자동 승인.
  const [session, setSession] = useState<SessionStatus>({ unlocked: false, remaining_secs: 0, auto_limit: "0" });
  // 사람이 연 작업 예산 — 1초 폴링.
  const [jobView, setJobView] = useState<JobView | null>(null);
  const [showUnlock, setShowUnlock] = useState(false);
  // 결제 요청별 자율 승인 1회 시도 가드(중복 시도·진행 중 모달 깜빡임 방지).
  const autoTried = useRef<Set<string>>(new Set());
  const autoBusy = useRef<string | null>(null);

  // 활성 체인 — settings.chain_id 로 파생. ChainProvider 로 내려준다.
  const chain = chainFromId(settings?.chain_id);
  // 예산은 연 체인·계정에서만 쓰인다(job.rs matches) — 다른 체인·계정을 보는 동안 «열린 작업 · N 남음» 을 보이면 거짓이다
  //. 판매자 화면·승인 시트는 원래 이렇게 걸렀고, 장부·패널·홈·작업 바도 같은 값을 쓴다.
  const jobHere =
    jobView != null && jobView.chain_id === chain.id && jobView.from.trim() === address.trim() ? jobView : null;

  // 내역은 한 번에 HISTORY_PAGE 줄씩 — 「더 보기」가 늘린다(200건을 넘친 기록도 보관 파일에 남는다).
  // 여러 곳이 부르는 loadHistory 가 늘 지금 줄 수로 읽게 ref 로 들고, 버튼 표시는 state 로 다시 그린다.
  const [historyLimit, setHistoryLimit] = useState(HISTORY_PAGE);
  const historyLimitRef = useRef(HISTORY_PAGE);
  const [historyMore, setHistoryMore] = useState(false);
  // 요청마다 세대 번호 — 늦게 온 옛 응답(200줄)이 새 응답(400줄)을 덮어 「더 보기」가 사라지지 않게.
  // 「마지막으로 **반영한** 세대보다 새것」이면 받는다 — 「가장 최근 요청의 응답만」이면
  // 그 최근 요청이 실패할 때 앞선 정상 응답까지 버려졌다.
  const historySeq = useRef(0);
  const historyShown = useRef(0);
  const loadHistory = useCallback(() => {
    const seq = ++historySeq.current;
    const limit = historyLimitRef.current;
    // 한 줄 더 달라고 해서 «더 있다» 를 판정한다 — 기록이 딱 limit 개일 때 «최근 N건 기준» 이 거짓으로 뜨지 않게.
    invoke<HistoryEntry[]>("get_history", { limit: limit + 1 })
      .then((list) => {
        if (seq > historyShown.current) {
          historyShown.current = seq;
          setHistoryMore(list.length > limit);
          setHistory(list.slice(0, limit));
        }
      })
      .catch(() => {});
  }, []);
  const setHistoryPage = useCallback(
    (limit: number) => {
      historyLimitRef.current = limit;
      setHistoryLimit(limit);
      loadHistory();
    },
    [loadHistory],
  );

  // 긴급 잠금 토글.
  const toggleLock = useCallback(async () => {
    const next = !locked;
    try {
      await invoke("set_locked", { locked: next });
      setLocked(next);
      notifySync();
    } catch {
      /* 토글 실패는 조용히 무시 */
    }
  }, [locked]);

  // 잔액 요청 세대 번호 — 늦게 도착한 옛 응답이 더 새 값을 덮지 않게.
  // 체인 전환 직후가 전형: 배경 30초 요청이 옛 체인에 나가 있는 사이 전환이 새 요청을
  // 쏘면, 옛 응답이 나중에 도착해 새 체인 화면에 옛 체인 잔액을 그린다. 수동·배경이
  // 같은 카운터를 쓰므로 마지막으로 시작한 요청만 화면에 닿는다.
  const balanceReq = useRef(0);
  // 수동 새로고침 진행 카운터(ref) — 배경 갱신이 수동 진행 중에 세대를 올리면 수동 쪽
  // finally 가 스피너를 못 끄고 영영 돈다. 배경은 수동이 하나라도 진행 중이면 쉰다.
  // (겹친 수동끼리는 세대 번호가 정리한다: 마지막 수동만 화면·스피너를 만진다.)
  const manualBusy = useRef(0);
  const refreshBalances = useCallback(async () => {
    const req = ++balanceReq.current;
    manualBusy.current += 1;
    setRefreshing(true);
    setBalanceError(null);
    try {
      const b = await invoke<Balances>("get_balances", { addrHex: address });
      if (balanceReq.current === req) setBalances(b);
    } catch (e) {
      if (balanceReq.current === req) setBalanceError(String(e));
    } finally {
      manualBusy.current -= 1;
      if (balanceReq.current === req) setRefreshing(false);
    }
  }, [address]);

  // 입금 자동 반영의 배경 갱신 — 스피너·에러 없이 조용히. 실패하면 기존 값을
  // 유지한다(외부 입금 감지가 목적이라, 일시적 RPC 오류로 멀쩡한 화면을 더럽힐 이유가 없다).
  // in-flight 가드: 느린 RPC 에서 30초 주기가 요청을 겹겹이 쌓지 않게 —
  // 진행 중이면 이번 차례는 그냥 쉰다. 응답 순서 꼬임은 위의 세대 번호가 막는다.
  const silentBusy = useRef(false);
  const refreshBalancesSilent = useCallback(async () => {
    if (silentBusy.current || manualBusy.current > 0) return;
    silentBusy.current = true;
    const req = ++balanceReq.current;
    try {
      const b = await invoke<Balances>("get_balances", { addrHex: address });
      if (balanceReq.current === req) {
        setBalances(b);
        setBalanceError(null);
      }
    } catch {
      /* 다음 주기에 다시 */
    } finally {
      silentBusy.current = false;
    }
  }, [address]);

  // 한도 설정 + 오늘 사용액 (송금 후 갱신).
  const loadLimits = useCallback(() => {
    invoke<Settings>("get_settings").then(setSettings).catch(() => {});
    invoke<boolean>("settings_file_broken").then(setSettingsBroken).catch(() => {});
    // 장부를 못 읽으면 null — 「오늘 남은 한도」를 꽉 찬 값으로 보이지 않게(결제는 백엔드가 막는다).
    invoke<SpendView>("get_today_spend").then(setSpend).catch(() => setSpend(null));
  }, []);

  useEffect(() => {
    void refreshBalances();
    loadLimits();
    invoke<boolean>("is_locked").then(setLocked).catch(() => {});
  }, [refreshBalances, loadLimits]);

  // 계정을 바꾸면 옛 계정의 잔액을 새 이름 아래 1초라도 보여주지 않는다 —
  // 새 조회가 올 때까지 「—」. 첫 마운트엔 어차피 null 이라 무해하다. 받기/보내기 카드도
  // 옛 계정 기준이었으니 잔액 카드로 돌아온다. 내역도 같다: 계정별
  // 파일이라 옛 계정 것을 비우고 재로드를 기다린다 — 안 비우면 IPC 가 실패할 때 영영 남는다.
  useEffect(() => {
    setBalances(null);
    setBalanceError(null);
    setHistory(null);
    setMode("balance");
  }, [address]);

  // 체인이 바뀌면(설정에서 테스트넷↔메인넷 전환) 잔액·내역은 체인별로 다르므로 다시 불러온다.
  // (사용액 spend 는 loadLimits 가 설정 저장 후 onClose 에서 갱신한다.)
  // 계정이 바뀌어도 같다 — 내역은 계정별 파일이다. refreshBalances 가 주소를 물고
  // 있어 주소가 바뀌면 이 효과도 다시 돈다.
  // RPC 주소도 같다 — 다른 창에서 RPC 만 다른 체인 것으로 바꾸면, 배경 갱신은 오류를 삼키고 옛 잔액을
  // 그대로 보였다. 체인·RPC 가 바뀌면 옛 잔액을 먼저 지우고(새 조회가 멈춰도 남의 체인 값을 안 보이게) 오류가 보이는 쪽으로 다시 읽는다.
  const chainId = settings?.chain_id;
  const rpcUrl = settings?.rpc_url;
  const balanceSource = useRef<string | null>(null);
  useEffect(() => {
    if (chainId === undefined) return;
    const source = `${chainId}|${rpcUrl ?? ""}`;
    if (balanceSource.current !== null && balanceSource.current !== source) setBalances(null);
    balanceSource.current = source;
    void refreshBalances();
    loadHistory();
  }, [chainId, rpcUrl, refreshBalances, loadHistory]);

  // 입금 자동 반영: 잔액이 시작·체인 전환·결제 직후·수동 ↻에서만 갱신돼서 외부
  // 입금은 재시작해야 보이던 문제(실사용 발견). 창이 보일 때만 30초 폴링 + 창이 다시
  // 보이거나 포커스가 돌아오면 즉시 1회. 팝오버가 숨으면 WKWebView 가 document.hidden 을
  // 켜므로 숨은 동안은 RPC 를 안 부른다.
  useEffect(() => {
    let last = 0;
    const tick = () => {
      if (document.hidden) return;
      last = Date.now();
      void refreshBalancesSilent();
    };
    const h = setInterval(tick, 30_000);
    // 복귀 1회.
    const onReturn = () => {
      if (document.hidden) return;
      // 설정을 다시 읽는다: 이 화면은 창이 숨어도 마운트된 채라, 사용자가
      // 배너대로 settings.json 을 고쳐도 다음 결제·설정 닫기·재시작 전엔 배너와 체인 라벨이
      // 옛 값에 머문다. 창이 돌아올 때 재확인하면 그 사이가 사라진다(파일 읽기 셋뿐). 아래
      // 잔액 스로틀 **앞**에 둔다: 30초 틱 직후 5초 안에 돌아오면 잔액은
      // 건너뛰어도 되지만 설정은 RPC 가 아니라 건너뛸 이유가 없다.
      loadLimits();
      // 잔액은 직전 갱신과 겹치지 않게 5초 스로틀(RPC).
      if (Date.now() - last < 5_000) return;
      last = Date.now();
      void refreshBalancesSilent();
    };
    document.addEventListener("visibilitychange", onReturn);
    window.addEventListener("focus", onReturn);
    return () => {
      clearInterval(h);
      document.removeEventListener("visibilitychange", onReturn);
      window.removeEventListener("focus", onReturn);
    };
  }, [refreshBalancesSilent, loadLimits]);

  // 다른 창이 상태를 바꿨거나 이 창이 앞으로 돌아왔다 — 한도·설정·잠금·내역을 다시 읽는다.
  // 잔액은 위의 30초·복귀 갱신이 맡는다(RPC 라 여기서 또 부르면 복귀마다 두 번 나간다). 다만 알림은
  // 송금·승인 직후에도 오므로 그때 잔액이 늦지 않게 조용한 갱신을 한 번 — in-flight 가드가 겹침을 막는다.
  useSync(() => {
    loadLimits();
    loadHistory();
    invoke<boolean>("is_locked").then(setLocked).catch(() => {});
    void refreshBalancesSilent();
  });

  // 1초 폴링: ① AI 연결 상태 ② 세션 상태 ③ 결제 요청(=앱 생존 하트비트 갱신, 자율 승인 우선 시도).
  // 새 결제 요청 1건당 자율 승인을 먼저 시도 → 자율 불가면(NEEDS_PASSWORD·차단) 사람 승인 모달.
  // 같은 id면 객체를 유지(입력 안 끊기게).
  useEffect(() => {
    let active = true;
    const tick = async () => {
      invoke<AgentStatus>("get_agent_status")
        .then((s) => {
          if (active) setAgent((prev) => (prev.connected === s.connected && prev.client === s.client ? prev : s));
        })
        .catch(() => {});
      invoke<SessionStatus>("session_status")
        .then((s) => {
          if (active)
            setSession((prev) =>
              prev.unlocked === s.unlocked &&
              prev.remaining_secs === s.remaining_secs &&
              prev.auto_limit === s.auto_limit
                ? prev
                : s,
            );
        })
        .catch(() => {});
      invoke<JobView | null>("job_status")
        .then((v) => {
          if (active)
            setJobView((prev) =>
              prev && v && prev.job === v.job && prev.spent === v.spent && prev.budget === v.budget && prev.secs_left === v.secs_left
                ? prev
                : v,
            );
        })
        .catch(() => {});
      // 체인 확인이 내역을 바꿨으면(정산 확인·만료·한도 환불) 내역·한도·잔액을 다시 읽는다.
      invoke<boolean>("take_chain_updates")
        .then((changed) => {
          if (active && changed) {
            loadHistory();
            loadLimits();
            void refreshBalances();
          }
        })
        .catch(() => {});
      // x402 정산 결과(MCP가 기록)를 내역에 반영. 반영됐으면 내역·잔액 새로고침.
      invoke<number>("apply_x402_settlements")
        .then((n) => {
          if (active && n > 0) {
            loadHistory();
            void refreshBalances();
          }
        })
        .catch(() => {});

      let r: PaymentRequest | null = null;
      try {
        r = await invoke<PaymentRequest | null>("get_pending_request");
      } catch {
        return;
      }
      if (!active) return;
      if (!r) {
        if (autoBusy.current === null) setPending(null);
        return;
      }
      // 팝오버는 승인하지 않는다 — 자율 승인도 모달도 큰 창 몫이다. 「승인하러 가기」 띠만.
      if (!isApprover) {
        const req = r;
        setPending((prev) => (prev?.id === req.id ? prev : req));
        return;
      }
      // 자율 승인 처리 중인 요청이면 모달을 띄우지 않고 기다린다.
      if (autoBusy.current === r.id) return;
      // 이미 자율 시도를 해본 요청이면 곧장 모달(같은 id면 객체 유지).
      if (autoTried.current.has(r.id)) {
        const req = r;
        setPending((prev) => (prev?.id === req.id ? prev : req));
        return;
      }
      // 처음 보는 요청 → 자율 승인 1회 시도.
      autoTried.current.add(r.id);
      autoBusy.current = r.id;
      const req = r;
      try {
        await invoke("auto_approve_payment", { id: req.id });
        // 자율 승인 성공 → 모달 없이 조용히 처리. 잔액/내역 갱신.
        if (active) {
          setPending(null);
          void refreshBalances();
          loadLimits();
          loadHistory();
          notifySync();
        }
      } catch {
        // 자율 불가(세션 잠김·한도 초과·자율 꺼짐) 또는 차단 → 사람 승인 모달.
        // ⚠️ `autoTried` 는 요청 id 당 영구라, **다른 결제가 마침 처리 중이라** 거절된 경우
        //에도 이 요청은 영영 수동이 된다. 일부러 그대로 둔다 — 자동
        // 재시도를 넣으면 「A 가 끝나자마자 B 가 사람 없이 나가는」 순차 재시도를 우리가
        // 만들어 주는 꼴이다. 사람 쪽으로 기우는 실패가 맞는 방향이다.
        if (active) setPending(req);
      } finally {
        autoBusy.current = null;
      }
    };
    void tick();
    const h = setInterval(() => void tick(), 1000);
    return () => {
      active = false;
      clearInterval(h);
    };
  }, [refreshBalances, loadLimits, loadHistory]);

  // 사람 승인이 필요한 동안 창을 전면 고정 (가려진 채 5분 타임아웃을 놓친 실사례 방지).
  // macOS 14+는 포커스 뺏기를 무시해서 항상-위 고정 방식 — 승인이 끝나면(cleanup) 해제.
  const pendingId = pending?.id;
  useEffect(() => {
    if (!pendingId || !isApprover) return;
    // 순서가 뒤집혀도(둘 다 비동기) 백엔드가 대기 요청 유무를 직접 보고 판단하므로 안전하다.
    invoke("raise_main_window").catch(() => {});
    return () => {
      invoke("release_main_window").catch(() => {});
    };
  }, [pendingId]);

  // 세션 잠금 해제 — 비번 한 번 입력 → 메모리에 키 보관, 한도 이하 자동 결제 활성화.
  const unlockSession = useCallback(async (password: string) => {
    await invoke("unlock_session", { password });
    const s = await invoke<SessionStatus>("session_status");
    setSession(s);
  }, []);

  // 세션 수동 잠금 — 메모리 키 즉시 소멸.
  const lockSession = useCallback(async () => {
    try {
      await invoke("lock_session");
    } catch {
      /* 무시 */
    }
    setSession((prev) => ({ ...prev, unlocked: false, remaining_secs: 0 }));
  }, []);

  // 결제 승인 팝업은 어떤 하위 화면(백업/설정/내역) 위에도 떠야 하므로 모든 반환을 감싼다.
  // 활성 체인을 모든 하위 화면·모달에 내려주고(useChain), 결제 승인 팝업도 그 안에 둔다.
  const withModal = (node: React.ReactNode) => (
    <ChainProvider value={chain}>
      {node}
      <AnimatePresence>
        {pending && isApprover && (
          <PaymentApprovalModal
            key={pending.id}
            request={pending}
            locked={locked}
            balances={balances}
            accounts={accounts}
            client={agent.connected ? agent.client : ""}
            onResolved={() => {
              setPending(null);
              void refreshBalances();
              loadLimits();
              loadHistory();
              notifySync();
            }}
          />
        )}
      </AnimatePresence>
    </ChainProvider>
  );

  // 큰 창(시안 B «장부»): 왼쪽 레일이 늘 있고, 하위 화면은 레일 오른쪽에 뜬다. 팝오버는 레일 없이 미니.
  const railPage: RailPage = showBackup
    ? "backup"
    : showSettings
      ? "settings"
      : showHistory
        ? "history"
        : showHelp
          ? "help"
          : showConnect
            ? "connect"
            : showSellers
              ? "sellers"
              : "home";
  // 설정 화면이 저장 중이거나 저장 안 한 변경이 있으면 레일로 그냥 떠나지 않는다 —
  // 저장 중에 떠나면 SettingsScreen 이 언마운트돼 저장 뒤의 다시 읽기(onClose)가 안 불리고, 변경은 말없이 버려진다.
  // `since` = busy 가 켜진 시각. 저장이 응답하지 않으면(닫기도 busy 로 잠긴다) 레일까지 영영 막히지 않게 10초 뒤엔 놓아준다.
  const settingsGuard = useRef({ busy: false, dirty: false, since: 0 });
  const onSettingsGuard = useCallback((g: { busy: boolean; dirty: boolean }) => {
    const prev = settingsGuard.current;
    settingsGuard.current = { ...g, since: g.busy ? (prev.busy ? prev.since : Date.now()) : 0 };
  }, []);
  const [leaveTo, setLeaveTo] = useState<RailPage | null>(null);
  // 설정이 스스로 닫혔으면(예: 뒤에서 저장이 끝남) 묻던 것도 거둔다 — 남아 있으면 «버리고 이동» 이 거짓말이 된다.
  useEffect(() => {
    if (!showSettings) setLeaveTo(null);
  }, [showSettings]);
  useEffect(() => {
    if (!leaveTo) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setLeaveTo(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [leaveTo]);
  const leavePage = (p: RailPage) => {
    if (showSettings && p !== "settings") {
      // 「닫기」 와 같은 뒷일 — 한도 다시 읽기·다른 창 알림.
      loadLimits();
      notifySync();
    }
    if (showConnect && p !== "connect") loadReady();
    setShowBackup(p === "backup");
    setShowSettings(p === "settings");
    setShowHelp(p === "help");
    setShowConnect(p === "connect");
    setShowSellers(p === "sellers");
    // 작업·판매자도 내역을 읽어 묶는다 — 「더 보기」 로 늘린 줄 수는 내역에서만 의미가 있다.
    // 이미 내역이면 「더 보기」로 늘린 줄 수를 되돌리지 않는다.
    if (p === "history" && !showHistory) setHistoryPage(HISTORY_PAGE);
    setShowHistory(p === "history");
  };
  const goPage = (p: RailPage) => {
    if (p === railPage) return;
    if (showSettings) {
      const g = settingsGuard.current;
      if (g.busy && Date.now() - g.since < 10_000) return; // 저장이 끝나면 화면이 스스로 닫힌다
      if (settingsGuard.current.dirty) {
        setLeaveTo(p);
        return;
      }
    }
    leavePage(p);
  };
  // 오른쪽 작업 패널(다른 페이지에서도 열어 둔다) — 홈엔 늘 있고(일지 판), 장부엔 왼쪽 작업 단이
  // 같은 일을 해서 없다. 나머지 페이지는 오른쪽 위 단추로 열고 닫는다. 정하지 않았으면 넓을 때(≥1280)만 저절로 연다 —
  // 기본 크기(960)의 모습은 그대로 두고, 창을 키우면 빈 오른쪽을 패널이 채운다.
  const [panelPref, setPanelPref] = usePref<"auto" | "open" | "closed">("panel", "auto");
  const roomy = useMedia("(min-width: 1280px)");
  // 홈의 패널은 따로 기억한다 — 홈에선 기본으로 열려 있고(일지 판), 닫으면 오른쪽 위 단추로 다시 연다.
  const [homePanel, setHomePanel] = usePref<"open" | "closed">("homePanel", "open");
  const fits = useMedia("(min-width: 900px)");
  const panelOpen = fits && (panelPref === "open" || (panelPref === "auto" && roomy));
  // framed 가 패널에서 부르므로 그보다 먼저(const 는 선언 전에 못 쓴다).
  const endJob = () => {
    invoke("end_job").catch(() => {});
    setJobView(null);
  };
  const openJob = (label: string) => {
    setLedgerScope(`job:${label}`);
    goPage("history");
  };
  const framed = (node: React.ReactNode, panel = true) =>
    isWindow ? (
      <div className="flex h-screen">
        <Rail page={railPage} onPage={goPage} locked={locked} onToggleLock={toggleLock} backedUp={backedUp} />
        <div className="flex-1 min-w-0 flex">{node}</div>
        {panel && panelOpen && (
          <JobPanel
            job={jobHere}
            locked={locked}
            entries={history}
            onEnd={endJob}
            onOpenJob={openJob}
            onClose={() => setPanelPref("closed")}
            className="h-screen w-[300px] shrink-0"
          />
        )}
        {panel && fits && !panelOpen && (
          <div className="fixed top-3 right-4 z-40">
            <IconButton label={t("작업 패널 열기", "Show job panel")} onClick={() => setPanelPref("open")}>
              <PanelRight size={16} />
            </IconButton>
          </div>
        )}
        <AnimatePresence>
          {leaveTo && (
            <motion.div
              key="leave"
              className={modalOverlay}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
            >
              <div className={cn(modalCard, "px-6 py-6")} role="alertdialog" aria-modal="true">
                <p className="text-[15px] font-semibold">{t("저장하지 않은 변경이 있어요", "You have unsaved changes")}</p>
                <p className="mt-2 text-[13px] leading-relaxed text-[var(--color-ink-500)]">
                  {t("지금 떠나면 바꾼 설정이 사라져요.", "Leaving now discards the settings you changed.")}
                </p>
                <div className="mt-5 grid grid-cols-2 gap-2">
                  <button type="button" autoFocus className={secondaryBtn} onClick={() => setLeaveTo(null)}>
                    {t("계속 편집", "Keep editing")}
                  </button>
                  <button
                    type="button"
                    // 보라는 돈이 걸린 곳에만 — 버리기는 붉은 글자의 보통 단추.
                    className={cn(secondaryBtn, "text-red-500")}
                    onClick={() => {
                      const p = leaveTo;
                      setLeaveTo(null);
                      leavePage(p);
                    }}
                  >
                    {t("버리고 이동", "Discard")}
                  </button>
                </div>
              </div>
            </motion.div>
          )}
        </AnimatePresence>
      </div>
    ) : (
      node
    );

  // 시드 백업 화면 (헤더 버튼 또는 경고 배너에서 진입) — 비번 입력부터 시작.
  if (showBackup) {
    return withModal(framed(
      <BackupFlow
        mode="review"
        onComplete={() => {
          setBackedUp(true);
          setShowBackup(false);
          notifySync();
        }}
        onExit={() => setShowBackup(false)}
      />),

    );
  }

  // 설정 화면.
  if (showSettings) {
    return withModal(framed(
      <SettingsScreen
        current={settings}
        spend={spend}
        initialGroup={settingsGroup}
        onGuard={onSettingsGuard}
        onClose={() => {
          setShowSettings(false);
          setSettingsGroup(undefined); // 언어 바꿔 돌아온 한 번만 «앱» 묶음으로 연다.
          loadLimits();
          notifySync();
        }}
      />),

    );
  }


  // 판매자 — 큰 창 전용.
  // 읽어 온 내역이 꽉 찼으면 더 오래된 기록이 있을 수 있다 — 합계가 «최근 N건 기준» 임을 말한다.
  const partial = history && historyMore ? history.length : undefined;
  // 작업과 내역(장부) — 큰 창 전용(팝오버 미니엔 들어오는 길이 없다). 왼쪽 단이 작업 목록이라 오른쪽 패널은 없다.
  if (showHistory) {
    return withModal(
      framed(
        <LedgerPage
          entries={history}
          job={jobHere}
          locked={locked}
          scope={ledgerScope}
          onScope={setLedgerScope}
          onEnd={endJob}
          onMore={partial ? () => setHistoryPage(historyLimit + HISTORY_PAGE) : undefined}
          partial={partial}
        />,
        false,
      ),
    );
  }
  if (showSellers) {
    return withModal(framed(<SellersPage entries={history} job={jobView} partial={partial} address={address} />));
  }

  // 도움말 화면.
  if (showHelp) {
    return withModal(framed(<HelpScreen onClose={() => setShowHelp(false)} />));
  }

  // AI 연결 화면 — 연결 배지가 진입점.
  if (showConnect) {
    return withModal(framed(
      <ConnectScreen
        agent={agent}
        ready={ready}
        onClose={() => {
          setShowConnect(false);
          loadReady();
        }}
      />),

    );
  }

  if (isWindow) {
    const notices = (
      <>
        {locked && <LockBanner onUnlock={toggleLock} />}
        {!locked && (Number(session.auto_limit) > 0 || session.unlocked) && (
          <SessionBar session={session} onUnlock={() => setShowUnlock(true)} onLock={lockSession} />
        )}
        {!backedUp && <BackupNag onBackup={() => setShowBackup(true)} />}
        {settingsBroken && <SettingsBrokenBanner />}
      </>
    );
    const anyNotice =
      locked || Number(session.auto_limit) > 0 || session.unlocked || !backedUp || settingsBroken;
    return withModal(
      <>
        {framed(
          <WindowHome
            account={account}
            address={address}
            balances={balances}
            balanceError={balanceError}
            refreshing={refreshing}
            onRefresh={() => void refreshBalances()}
            agent={agent}
            onConnect={() => goPage("connect")}
            settings={settings}
            spend={spend}
            job={jobHere}
            locked={locked}
            onEndJob={endJob}
            history={history}
            onOpenJob={openJob}
            onReceive={() => setMode("receive")}
            onSendSol={() => setMode("sendSol")}
            panelOpen={homePanel === "open"}
            onPanel={(o) => setHomePanel(o ? "open" : "closed")}
            notices={anyNotice ? notices : null}
          />,
          false,
        )}
        <AnimatePresence>
          {mode === "receive" && (
            <motion.div
              key="receive"
              className={modalOverlay}
              initial={{ opacity: 0 }}
              animate={{ opacity: 1 }}
              exit={{ opacity: 0 }}
              onClick={(e) => {
                if (e.target === e.currentTarget) setMode("balance");
              }}
            >
              <div className="w-full max-w-sm my-auto">
                <ReceiveCard address={address} onClose={() => setMode("balance")} />
              </div>
            </motion.div>
          )}
          {mode === "sendSol" && (
            <SendSolSheet
              key="sendSol"
              balance={balances?.sol}
              onClose={() => setMode("balance")}
              onSent={() => void refreshBalances()}
            />
          )}
          {showUnlock && (
            <UnlockSessionModal
              autoLimit={session.auto_limit}
              onUnlock={unlockSession}
              onClose={() => setShowUnlock(false)}
            />
          )}
        </AnimatePresence>
      </>,
    );
  }

  // 메뉴바 팝오버 = 미니: 잔액 · 작업 바 · 「창 열기」. 내역·설정·백업·연결·도움말은 큰 창에서.
  // 남긴 둘 — 긴급 잠금 단추와 자율 결제 바 — 는 «돈을 지금 멈추는» 손잡이라 메뉴바에서 한 번에 닿아야 한다.
  return withModal(
    <main className={shell}>
      <div className="w-full max-w-md flex flex-col gap-4">
        <header className="flex items-center justify-between text-[12px]">
          {/* 지금 어느 계정·어느 체인인지가 잔액보다 먼저. */}
          <div className="flex min-w-0 items-center gap-2 text-[var(--color-ink-900)] dark:text-[#E8E5DD] font-medium tracking-tight">
            <span className="inline-block w-1.5 h-1.5 rounded-full bg-[var(--color-accent)]" aria-hidden />
            <span className="truncate">{accountName(account)}</span>
            <span
              className={cn(
                "shrink-0 font-normal",
                chain.testnet ? "text-[var(--color-ink-500)]" : "text-[var(--color-accent)] font-medium",
              )}
            >
              · {chain.name}
            </span>
          </div>
          <div className="flex shrink-0 items-center gap-1.5">
            <span
              className={cn(
                "mr-1 inline-flex items-center gap-1.5",
                agent.connected ? "text-emerald-600 dark:text-emerald-500" : "text-[var(--color-ink-300)]",
              )}
            >
              <span className={cn("w-1.5 h-1.5 rounded-full", agent.connected ? "bg-emerald-500" : "bg-[var(--color-ink-300)]")} aria-hidden />
              {agent.connected ? agent.client || t("AI 연결됨", "AI connected") : t("AI 연결 안 됨", "AI not connected")}
            </span>
            <HeaderIconButton
              onClick={toggleLock}
              label={
                locked
                  ? t("긴급 잠금 해제", "Turn off emergency lock")
                  : t("긴급 잠금 (모든 결제 차단)", "Emergency lock (blocks every payment)")
              }
              tone={locked ? "danger" : "default"}
            >
              {locked ? <ShieldAlert size={14} /> : <ShieldCheck size={14} />}
            </HeaderIconButton>
          </div>
        </header>

        {!isApprover && pending && <ApprovalElsewhereBanner />}
        {locked && <LockBanner onUnlock={toggleLock} />}
        {!locked && jobHere && <JobBar job={jobHere} onEnd={endJob} />}
        {!locked && (Number(session.auto_limit) > 0 || session.unlocked) && (
          <SessionBar session={session} onUnlock={() => setShowUnlock(true)} onLock={lockSession} />
        )}
      </div>

      <div className="w-full max-w-md">
        <BalanceCard
          address={address}
          copied={copied}
          onCopy={() => copy(address)}
          balances={balances}
          balanceError={balanceError}
          refreshing={refreshing}
          onRefresh={refreshBalances}
        />
      </div>

      <div className="w-full max-w-md">
        <ActionButton
          icon={<Maximize2 size={15} />}
          label={t("창 열기", "Open window")}
          onClick={() => invoke("open_app_window").catch(() => {})}
          active={false}
        />
      </div>

      <AnimatePresence>
        {showUnlock && (
          <UnlockSessionModal autoLimit={session.auto_limit} onUnlock={unlockSession} onClose={() => setShowUnlock(false)} />
        )}
      </AnimatePresence>
    </main>,
  );
}
