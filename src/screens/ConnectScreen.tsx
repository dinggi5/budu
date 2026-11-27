// AI 연결 화면 — 지금까지 앱 밖(릴리스 페이지의 .mcpb, 터미널의 claude mcp add)에
// 있던 연결을 앱 안으로 들인다: 설치 → 지갑 생성 → 연결이 이 창 안에서 끝난다.
//
// 감지(설치·등록)는 3초 폴링 — 전부 로컬 파일 읽기라 싸고, 사용자가 Claude 쪽에서
// '설치'를 누르고 돌아오면 화면이 스스로 따라잡는다. 맨 위 상태 카드는 배지와 같은 셋이다
//: 쓰는 중(하트비트) / 준비됨(등록이 살아 있음) / 연결 필요.

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import {
  Bot,
  Check,
  Copy,
  ExternalLink,
  Loader2,
  MessageSquare,
  Plug,
  Terminal,
} from "lucide-react";
import { cn } from "@/lib/cn";
import { GITHUB_URL } from "@/lib/helpContent";
import { useCopy } from "@/lib/useCopy";
import type { AgentStatus, ConnectError, ConnectStatus } from "@/lib/types";
import { enter, flatCard, LedgerHead, pageColumn, pageShell, primaryBtn, ScreenHeader, secondaryBtn } from "@/components/ui";
import { isWindow } from "@/lib/win";
import { Eyebrow, GhostButton, PageTitle, Tag, hairline, ink, inkSoft, muted } from "@/components/win";
import { t } from "@/lib/i18n";

const em = "text-[var(--color-ink-700)] dark:text-[#E8E5DD]";

/** 명령·경로를 보여주는 모노 박스 (helpContent 의 코드 박스와 같은 결). */
/** 큰 창 상황판의 명령 한 줄 — 복사하기 전에 무엇이 복사되는지 보이게. */
function Mono({ text }: { text: string }) {
  return <p className={cn("mt-1.5 font-mono text-[12px] break-all select-text", muted)}>{text}</p>;
}

function CodeBox({ text }: { text: string }) {
  return (
    <div className="px-3 py-2 rounded-[10px] bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)] border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)] font-mono text-[11px] leading-relaxed text-[var(--color-ink-900)] dark:text-[#E8E5DD] select-all break-all">
      {text}
    </div>
  );
}

/** 임시 실행(디스크 이미지 등) + 설치본이 다른 버전일 때의 경고
 * .
 *
 *  🔴 클라이언트 카드 **안**에 두면 안 된다. 처음엔 "등록 안 된 Claude Code" 와
 *  "다른 AI 앱" 두 갈래에만 넣었는데, 정작 흔한 경우들이 다 빠졌다:
 *    - 이미 등록돼 있으면(cli_registered) 그 카드는 "등록돼 있어요" 한 줄로 끝난다.
 *    - Claude 데스크톱은 확장 런처가 **설치본**의 budu-mcp 를 찾아 exec 하므로
 *      똑같이 영향을 받는데 그 카드에는 아예 안 그렸다.
 *  버전이 갈린다는 건 특정 클라이언트가 아니라 **이 맥의 상태**다 → 화면 위에 한 번. */
function StaleInstallNote({ installed, running }: { installed: string; running: string }) {
  return (
    <div
      className={cn(
        "px-4 py-3 rounded-[var(--radius-card)]",
        "bg-amber-50 dark:bg-amber-950/30",
        "border border-amber-200 dark:border-amber-900/50",
      )}
    >
      <p className="text-[12px] leading-relaxed text-amber-800 dark:text-amber-300">
        {t(
          <>
            지금 이 앱은 <b>{running}</b> 버전인데, 이 맥에 설치된 Budu는 <b>{installed}</b> 버전이에요.
            어느 쪽으로 연결하든 AI는 <b>설치본</b>의 MCP를 실행해요 — 이 앱을{" "}
            <b>응용 프로그램</b> 폴더에 덮어써서 두 버전을 맞춘 뒤 쓰는 게 좋아요.
          </>,
          <>
            This app is <b>{running}</b>, but the Budu installed on this Mac is <b>{installed}</b>.
            However you connect, the AI runs the MCP from the <b>installed</b> copy — copy this app
            over the one in <b>Applications</b> so the two match, then connect.
          </>,
        )}
      </p>
    </div>
  );
}

/** 카드 우상단 상태 라벨. */
/** 상황판 한 줄 — 아이콘 · 이름 + 한 줄 설명 · 상태 꼬리표 · 오른쪽 손잡이. 줄 사이 선은 아주 옅게. */
function BoardRow({
  icon,
  name,
  sub,
  tag,
  action,
  children,
}: {
  icon: React.ReactNode;
  name: string;
  sub?: string;
  tag: React.ReactNode;
  action?: React.ReactNode;
  children?: React.ReactNode;
}) {
  return (
    <li className={cn("flex items-start gap-4 py-4 border-b last:border-b-0", hairline)}>
      <span className="mt-0.5 w-9 h-9 shrink-0 rounded-[10px] grid place-items-center bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)] text-[var(--color-ink-500)]">
        {icon}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-center gap-2">
          <span className={cn("text-[15px] font-semibold", ink)}>{name}</span>
          {tag}
        </div>
        {sub && <p className={cn("mt-0.5 text-[13px] leading-relaxed", muted)}>{sub}</p>}
        {children}
      </div>
      {action && <div className="shrink-0 pt-0.5">{action}</div>}
    </li>
  );
}

function StateTag({ ok, label }: { ok?: boolean; label: string }) {
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1 text-[11px] tracking-tight",
        ok ? "text-[var(--color-accent)]" : "text-[var(--color-ink-300)]",
      )}
    >
      {ok && <Check size={11} />}
      {label}
    </span>
  );
}

function ClientCard({
  icon,
  title,
  tag,
  children,
}: {
  icon: React.ReactNode;
  title: string;
  tag?: React.ReactNode;
  children: React.ReactNode;
}) {
  // 큰 창 — 카드 없이 장부 묶음 머리, 본문은 14px.
  if (isWindow) {
    return (
      <section className="w-full">
        <LedgerHead title={title} tag={tag} />
        <div className="pt-3.5 text-[14px] leading-relaxed text-[var(--color-ink-500)]">{children}</div>
      </section>
    );
  }
  return (
    <section className={cn(flatCard, "px-5 py-4")}>
      <div className="flex items-center justify-between">
        <h2 className="flex items-center gap-2 text-[13px] tracking-tight text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
          <span className="text-[var(--color-ink-300)]">{icon}</span>
          {title}
        </h2>
        {tag}
      </div>
      <div className="mt-3 text-[12px] leading-relaxed text-[var(--color-ink-500)]">{children}</div>
    </section>
  );
}

/** 앱 번들에 Claude 데스크톱 확장(budu.mcpb)이 들어 있나. 개발 6에서 싣고 true 로 바꾼다. */
const DESKTOP_EXT_BUNDLED = false;

export function ConnectScreen({
  agent,
  ready,
  onClose,
}: {
  agent: AgentStatus;
  /** 「준비됨」 — 배지와 같은 값. 이 화면은 3초마다 자기 감지를 따로 돌린다. */
  ready: boolean | null;
  onClose: () => void;
}) {
  const [status, setStatus] = useState<ConnectStatus | null>(null);
  const [copied, copy] = useCopy();
  const [pathCopied, copyPath] = useCopy();
  const [restoreCopied, copyRestore] = useCopy();
  const [codexCopied, copyCodex] = useCopy();
  // Claude 데스크톱: 열기 시도 결과 — 성공하면 "설치 창에서 '설치'를 누르라"는 다음 단계 안내.
  const [desktopBusy, setDesktopBusy] = useState(false);
  const [desktopMsg, setDesktopMsg] = useState<{ ok: boolean; text: string } | null>(null);
  // Claude Code: 등록 대행 결과.
  const [cliBusy, setCliBusy] = useState(false);
  const [cliError, setCliError] = useState<ConnectError | null>(null);
  const [codexBusy, setCodexBusy] = useState(false);
  const [codexError, setCodexError] = useState<ConnectError | null>(null);

  const refresh = useCallback(() => {
    invoke<ConnectStatus>("get_connect_status").then(setStatus).catch(() => {});
  }, []);

  useEffect(() => {
    refresh();
    const h = setInterval(refresh, 3000);
    return () => clearInterval(h);
  }, [refresh]);

  const connectDesktop = async () => {
    setDesktopBusy(true);
    setDesktopMsg(null);
    try {
      await invoke("connect_claude_desktop");
      setDesktopMsg({
        ok: true,
        text: t(
          "Claude 데스크톱에 설치 창이 떴어요 — 거기서 '설치'를 누르면 끝나요.",
          "Claude desktop opened its installer — press Install there and you're done.",
        ),
      });
    } catch (e) {
      setDesktopMsg({ ok: false, text: String(e) });
    } finally {
      setDesktopBusy(false);
    }
  };

  const connectCode = async () => {
    if (cliBusy) return; // 연타로 remove→add 가 엇갈려 돌지 않게.
    setCliBusy(true);
    setCliError(null);
    try {
      await invoke("connect_claude_code");
      refresh(); // 등록 직후 상태 태그가 바로 "등록됨"으로 바뀌게.
    } catch (e) {
      // 백엔드는 ConnectError 객체를 준다. 직렬화가 어긋나거나 invoke 자체가 던지면
      // 문자열일 수 있으니(그때 String(e) 는 "[object Object]" 가 아니라 진짜 메시지),
      // 모양을 보고 갈라 받는다 — 사용자에게 [object Object] 를 보여주지 않으려고.
      const err = e as Partial<ConnectError> | undefined;
      setCliError(
        typeof err?.message === "string"
          ? { message: err.message, restore_command: err.restore_command ?? null }
          : { message: String(e), restore_command: null },
      );
    } finally {
      setCliBusy(false);
    }
  };

  // Codex — Claude Code 와 같은 대행(`codex mcp add budu -- <경로>`, 백엔드 connect_codex).
  const connectCodex = async () => {
    if (codexBusy) return;
    setCodexBusy(true);
    setCodexError(null);
    try {
      await invoke("connect_codex");
      refresh();
    } catch (e) {
      const err = e as Partial<ConnectError> | undefined;
      setCodexError(
        typeof err?.message === "string"
          ? { message: err.message, restore_command: err.restore_command ?? null }
          : { message: String(e), restore_command: null },
      );
    } finally {
      setCodexBusy(false);
    }
  };

  // 수동 등록 명령. mcp_path 는 이 빌드의 실제 사이드카 경로 — dev 에서도 맞는 걸 보여준다.
  // 셸 인용: 앱이 공백 든 경로에 있으면 인용 없는 복사 명령이 경로를
  // 여러 인자로 쪼개 command 가 반토막 난 채 등록된다. 작은따옴표로 감싸고 안의 '는 탈출.
  const mcpPath = status?.mcp_path ?? "/Applications/Budu.app/Contents/MacOS/budu-mcp";
  const shellQuoted = `'${mcpPath.replace(/'/g, `'\\''`)}'`;
  const cliCommand = `claude mcp add --scope user budu -- ${shellQuoted}`;
  // 등록 대행이 실패했을 때의 수동 명령. 실패 시 옛 등록을 원복해 두므로(백엔드) budu
  // 항목이 남아 있을 수 있고, bare add 는 "already exists" 로 거부된다
  // — 지우고 다시 등록하는 시퀀스로 안내한다(항목이 없으면 remove 만 실패하고 add 는 된다).
  const reRegisterCommand = `claude mcp remove --scope user budu; ${cliCommand}`;
  // 임시 위치(디스크 이미지·Translocation) 실행 + 설치본 없음 — 등록할 유효 경로 자체가
  // 없다. 이 상태에선 모든 클라이언트 카드가 경로 대신 "옮기기" 안내를 보여준다.
  const tempNoPath = !!status?.temp_location && !status.mcp_path;
  // 임시 실행이라 설치본 경로를 등록하는데, 그 설치본이 이 앱과 다른 버전인 경우
  //. 등록 자체는 막지 않는다 — 설치본 경로가 어차피 오래 갈
  // 유일한 경로다. 다만 지금 AI 에 붙을 바이너리가 이 화면과 다른 버전이라는 건 말한다.
  const staleInstall = status?.installed_version_mismatch ?? null;
  // 대행 실패 때 백엔드가 준 "옛 등록 되살리기" 명령. 지역 const 로 받아야 JSX 안에서
  // 좁혀진다(상태 변수는 콜백 안에서 다시 null 일 수 있다고 본다).
  const restoreCmd = cliError?.restore_command ?? null;
  // 「준비됨」은 배지 값을 쓰되, 이 화면에서 방금 등록했으면 3초 감지가 먼저 안다 — 둘 중 하나면 참.
  const isReady =
    !!ready ||
    !!status?.cli_registered ||
    !!status?.codex_registered ||
    (!!status?.desktop_ext_installed && !status.desktop_ext_disabled);

  // 큰 창 = 상황판(연결 대상마다 켜짐/꺼짐으로 상황을 보인다). 팝오버는 아래 옛 카드 그대로.
  if (isWindow) {
    const quotedPath = shellQuoted;
    const codexCommand = `codex mcp add budu -- ${quotedPath}`;
    const codeState: { tone: "ok" | "idle" | "bad"; label: string } = status?.cli_registered
      ? { tone: "ok", label: t("연결됨", "Connected") }
      : status?.cli_registered_other
        ? { tone: "bad", label: t("옛 경로로 등록됨", "Registered to an old path") }
        : { tone: "idle", label: t("꺼짐", "Off") };
    return (
      <main className="h-screen min-w-0 flex-1 overflow-y-auto px-6 min-[1100px]:px-10 pt-14 pb-12 bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]">
        <PageTitle title={t("AI 연결", "Connect an AI")} />
        <div className="mt-6 flex items-center gap-2 text-[13px]">
          <span className={cn("w-2 h-2 rounded-full", agent.connected ? "bg-emerald-500" : isReady ? "bg-[var(--color-accent)]" : "bg-[var(--color-ink-300)]")} aria-hidden />
          <span className={inkSoft}>
            {agent.connected
              ? t(`지금 ${agent.client || "AI"} 가 붙어 있어요`, `${agent.client || "An AI"} is attached now`)
              : isReady
                ? t("준비돼 있어요. AI 를 켜면 알아서 붙어요", "Ready. Open the AI and it attaches on its own")
                : t("아직 연결된 AI 가 없어요", "No AI connected yet")}
          </span>
        </div>
        {staleInstall && status && (
          <div className="mt-4 max-w-[640px]">
            <StaleInstallNote installed={staleInstall} running={status.app_version} />
          </div>
        )}
        {tempNoPath && (
          <p className="mt-4 text-[13px] text-amber-600 dark:text-amber-500">
            {t("디스크 이미지에서 실행 중이에요. Budu 를 응용 프로그램 폴더로 옮긴 뒤 연결하세요.", "Running from a disk image. Move Budu to Applications first, then connect.")}
          </p>
        )}

        <Eyebrow className="mt-10 mb-1">{t("연결 대상", "Apps")}</Eyebrow>
        <ul>
          <BoardRow
            icon={<Terminal size={16} />}
            name="Claude Code"
            sub={
              status == null
                ? t("상태를 읽는 중이에요.", "Checking…")
                : tempNoPath
                  ? t("응용 프로그램 폴더로 옮긴 뒤 연결할 수 있어요.", "Move Budu to Applications to connect.")
                  : status.cli_registered_other
                    ? t("budu 등록이 다른 경로(옛 설치 등)를 가리켜요. 다시 등록하면 이 앱으로 바로잡아요.", "The budu entry points somewhere else (an older install). Registering again fixes it.")
                    : status.cli_path
                      ? undefined
                      : t("터미널에 다음 명령어를 입력하시면 자동으로 연결합니다.", "Enter this command in Terminal to connect.")
            }
            tag={<Tag tone={codeState.tone}>{codeState.label}</Tag>}
            action={
              status == null || tempNoPath ? null : status.cli_path ? (
                <GhostButton onClick={() => void connectCode()} disabled={cliBusy}>
                  {cliBusy ? <Loader2 size={13} className="animate-spin" /> : null}
                  {status.cli_registered
                    ? t("다시 등록", "Re-register")
                    : status.cli_registered_other
                      ? t("이 앱으로 다시 등록", "Point it at this app")
                      : t("연결", "Connect")}
                </GhostButton>
              ) : (
                <GhostButton onClick={() => copy(status.cli_registered_other ? reRegisterCommand : cliCommand)}>
                  {copied ? <Check size={13} /> : <Copy size={13} />}
                  {t("명령 복사", "Copy command")}
                </GhostButton>
              )
            }
          >
            {status != null && !tempNoPath && !status.cli_path && !cliError && <Mono text={status.cli_registered_other ? reRegisterCommand : cliCommand} />}
            {cliError && (
              <div role="alert" className="mt-2 space-y-2 max-w-[640px]">
                <p className="text-[12px] text-red-500 break-words">{cliError.message}</p>
                <p className={cn("text-[12px]", muted)}>{t("직접 등록하려면 이 명령을 터미널에 붙여 넣으세요.", "To register by hand, paste this into Terminal.")}</p>
                <Mono text={reRegisterCommand} />
                <button type="button" onClick={() => copy(reRegisterCommand)} className={cn("text-[12px] underline underline-offset-2", inkSoft)}>
                  {copied ? t("복사됨", "Copied") : t("명령 복사", "Copy command")}
                </button>
                {/* remove 는 되돌려 둔 옛 등록도 지운다 — 팝오버와 같은 경고. */}
                {restoreCmd && (
                  <>
                    <p className="pt-1 text-[12px] text-amber-700 dark:text-amber-400">
                      {t(
                        "위 명령의 remove 는 되돌려 둔 기존 budu 등록도 지워요. 뒤의 add 가 또 실패하면 아래 명령으로 원래대로 되돌릴 수 있어요.",
                        "The remove above also deletes the budu entry we put back. If the add fails again, this restores it.",
                      )}
                    </p>
                    <Mono text={restoreCmd} />
                    <button type="button" onClick={() => copyRestore(restoreCmd)} className={cn("text-[12px] underline underline-offset-2", inkSoft)}>
                      {restoreCopied ? t("복사됨", "Copied") : t("되돌리기 명령 복사", "Copy restore command")}
                    </button>
                  </>
                )}
              </div>
            )}
          </BoardRow>
          <BoardRow
            icon={<MessageSquare size={16} />}
            name={t("Claude 데스크톱", "Claude desktop")}
            tag={<Tag tone="idle">{t("준비 중", "Coming soon")}</Tag>}
          />
          <BoardRow
            icon={<Terminal size={16} />}
            name="Codex"
            sub={
              status == null || tempNoPath
                ? undefined
                : status.codex_registered_other
                  ? t("budu 등록이 다른 경로(옛 설치 등)를 가리켜요. 다시 등록하면 이 앱으로 바로잡아요.", "The budu entry points somewhere else (an older install). Registering again fixes it.")
                  : status.codex_path
                    ? undefined
                    : t("터미널에 다음 명령어를 입력하시면 자동으로 연결합니다.", "Enter this command in Terminal to connect.")
            }
            tag={
              <Tag tone={status?.codex_registered ? "ok" : status?.codex_registered_other ? "bad" : "idle"}>
                {status?.codex_registered ? t("연결됨", "Connected") : status?.codex_registered_other ? t("옛 경로로 등록됨", "Registered to an old path") : t("꺼짐", "Off")}
              </Tag>
            }
            action={
              status == null || tempNoPath ? null : status.codex_path ? (
                <GhostButton onClick={() => void connectCodex()} disabled={codexBusy}>
                  {codexBusy ? <Loader2 size={13} className="animate-spin" /> : null}
                  {status.codex_registered ? t("다시 등록", "Re-register") : status.codex_registered_other ? t("이 앱으로 다시 등록", "Point it at this app") : t("연결", "Connect")}
                </GhostButton>
              ) : (
                <GhostButton onClick={() => copyCodex(codexCommand)}>
                  {codexCopied ? <Check size={13} /> : <Copy size={13} />}
                  {t("명령 복사", "Copy command")}
                </GhostButton>
              )
            }
          >
            {status != null && !tempNoPath && (!status.codex_path || codexError) && <Mono text={codexCommand} />}
            {codexError && (
              <div role="alert" className="mt-2 space-y-2 max-w-[640px]">
                <p className="text-[12px] text-red-500 break-words">{codexError.message}</p>
                {codexError.restore_command && <Mono text={codexError.restore_command} />}
              </div>
            )}
          </BoardRow>
          <BoardRow
            icon={<Plug size={16} />}
            name={t("다른 AI (Cursor 등)", "Other AIs (Cursor etc.)")}
            sub={t("MCP 를 지원하는 프로그램에 다음 경로를 서버로 등록하세요.", "Register this path as a server in any MCP-capable app.")}
            tag={<Tag tone="idle">{t("직접 추가", "Manual")}</Tag>}
            action={
              status == null || tempNoPath ? null : (
                <GhostButton onClick={() => copyPath(mcpPath)}>
                  {pathCopied ? <Check size={13} /> : <Copy size={13} />}
                  {t("경로 복사", "Copy path")}
                </GhostButton>
              )
            }
          >
            {status != null && !tempNoPath && <Mono text={mcpPath} />}
          </BoardRow>
        </ul>
        <button
          type="button"
          onClick={() => openUrl(GITHUB_URL).catch(() => {})}
          className={cn("mt-8 inline-flex items-center gap-1.5 text-[12px] hover:text-[var(--color-accent)] transition-colors", muted)}
        >
          <ExternalLink size={12} />
          {t("GitHub 문서", "Docs on GitHub")}
        </button>
      </main>
    );
  }

  return (
    <main className={pageShell}>
      <ScreenHeader
        icon={<Plug size={12} />}
        title={t("AI 연결", "Connect an AI")}
        sub={t("Claude 가 이 지갑으로 결제를 요청할 수 있게 등록해요. 키는 이 맥을 떠나지 않아요.", "Register Budu so Claude can ask this wallet to pay. The key never leaves this Mac.")}
        onClose={onClose}
      />

      <motion.div {...enter} className={cn(pageColumn, "flex flex-col gap-3 win:gap-10")}>
        {/* 지금 연결 상태 — 최종 진실(1초 폴링 배지와 같은 소스) */}
        <section className={cn(flatCard, "px-5 py-4")}>
          {agent.connected ? (
            <div className="flex items-center gap-2.5">
              <span className="relative flex w-2 h-2 shrink-0" aria-hidden>
                <span className="absolute inline-flex w-full h-full rounded-full bg-[var(--color-accent)] opacity-60 animate-ping" />
                <span className="relative inline-flex w-2 h-2 rounded-full bg-[var(--color-accent)]" />
              </span>
              <p className="text-[13px] text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
                {t(
                  <>
                    <b className={em}>연결돼 있어요.</b> AI가 결제를 요청하면 승인 팝업이 떠요.
                  </>,
                  <>
                    <b className={em}>Connected.</b> When the AI asks to pay, an approval window
                    opens.
                  </>,
                )}
              </p>
            </div>
          ) : isReady ? (
            <div className="flex items-start gap-2.5">
              <span
                className="mt-[7px] inline-flex w-2 h-2 shrink-0 rounded-full bg-[var(--color-accent)] opacity-70"
                aria-hidden
              />
              <p className="text-[12px] leading-relaxed text-[var(--color-ink-500)]">
                {t(
                  <>
                    <b className={em}>준비돼 있어요.</b> 다시 누를 건 없어요 — Claude를 켜면 알아서
                    붙고, Budu가 꺼져 있어도 결제를 요청할 때 깨워요.
                  </>,
                  <>
                    <b className={em}>You're set.</b> Nothing to press again — open Claude and it
                    attaches on its own, and it wakes Budu when it needs to pay.
                  </>,
                )}
              </p>
            </div>
          ) : (
            <p className="text-[12px] leading-relaxed text-[var(--color-ink-500)]">
              {t(
                <>
                  <b className={em}>아직 연결된 AI가 없어요.</b> 아래에서 쓰는 앱을 골라 연결하면,
                  AI가 잔액을 읽고 결제를 <b className={em}>요청</b>할 수 있어요. 승인은 언제나 이
                  앱에서 해요.
                </>,
                <>
                  <b className={em}>No AI is connected yet.</b> Pick the app you use below, and it
                  can read your balance and <b className={em}>ask</b> to pay. Approving always
                  happens here.
                </>,
              )}
            </p>
          )}
        </section>

        {/* 이 맥의 상태 경고 — 클라이언트와 무관하므로 카드 밖, 한 번만 */}
        {staleInstall && status && (
          <StaleInstallNote installed={staleInstall} running={status.app_version} />
        )}

        {/* Claude 데스크톱 — 동봉 확장(.mcpb) 열기 = 설치 다이얼로그 직행 */}
        <ClientCard
          icon={<MessageSquare size={14} />}
          title={t("Claude 데스크톱", "Claude desktop")}
          tag={
            status &&
            (status.desktop_ext_installed && status.desktop_ext_disabled ? (
              <StateTag label={t("확장 꺼져 있음", "Extension turned off")} />
            ) : status.desktop_ext_installed ? (
              <StateTag ok label={t("확장 설치됨", "Extension installed")} />
            ) : status.desktop_installed ? undefined : (
              <StateTag label={t("앱 미설치", "App not installed")} />
            ))
          }
        >
          {status && !status.desktop_installed ? (
            <div className="space-y-2.5">
              <p>
                {t(
                  "Claude 데스크톱 앱이 이 맥에 없어요. 먼저 설치하고 다시 오세요.",
                  "The Claude desktop app isn't on this Mac. Install it first, then come back.",
                )}
              </p>
              <button
                type="button"
                onClick={() => openUrl("https://claude.ai/download").catch(() => {})}
                className={cn(secondaryBtn, "w-full")}
              >
                <ExternalLink size={13} /> {t("claude.ai/download 열기", "Open claude.ai/download")}
              </button>
            </div>
          ) : !DESKTOP_EXT_BUNDLED ? (
            // 확장(.mcpb)을 아직 앱에 안 싣는다. 버튼을 살려 두면 누를 때마다
            // «동봉된 확장이 없어요» 로 실패하고, 재설치로도 안 고쳐진다 — 그래서 사실만 말한다.
            <p className="text-[var(--color-ink-500)]">
              {t(
                "Claude 데스크톱 연결은 준비 중이에요. 지금은 아래 Claude Code 연결을 써 주세요.",
                "Connecting Claude desktop isn't ready yet. For now, use the Claude Code connection below.",
              )}
            </p>
          ) : (
            <div className="space-y-2.5">
              <p>
                {status?.desktop_ext_disabled
                  ? t(
                      "확장은 깔려 있는데 꺼져 있어요. Claude 설정 → 확장 프로그램에서 Budu를 켜면 돼요.",
                      "The extension is installed but turned off. Turn Budu on in Claude's Settings → Extensions.",
                    )
                  : t(
                      "버튼을 누르면 Claude에 확장 설치 창이 떠요. 거기서 '설치'만 누르면 돼요.",
                      "Press the button and Claude opens its extension installer. Press Install there.",
                    )}
              </p>
              <button
                type="button"
                onClick={() => void connectDesktop()}
                disabled={desktopBusy || !status}
                className={cn(
                  status?.desktop_ext_installed ? secondaryBtn : primaryBtn,
                  "w-full",
                )}
              >
                {desktopBusy ? (
                  <Loader2 size={14} className="animate-spin" />
                ) : status?.desktop_ext_installed ? (
                  t("확장 다시 설치", "Reinstall the extension")
                ) : (
                  t("Claude 데스크톱에 연결", "Connect Claude desktop")
                )}
              </button>
              {desktopMsg && (
                <p
                  className={cn(
                    "text-[11px] leading-relaxed",
                    desktopMsg.ok ? "text-[var(--color-accent)]" : "text-red-500/90",
                  )}
                >
                  {desktopMsg.text}
                </p>
              )}
            </div>
          )}
        </ClientCard>

        {/* Claude Code — claude CLI 등록 대행 (없으면 명령 복사 폴백) */}
        <ClientCard
          icon={<Terminal size={14} />}
          title="Claude Code"
          tag={
            status &&
            (status.cli_registered ? (
              <StateTag ok label={t("등록됨", "Registered")} />
            ) : status.cli_registered_other ? (
              <StateTag label={t("다른 경로 등록됨", "Points elsewhere")} />
            ) : status.cli_path ? undefined : (
              <StateTag label={t("CLI 못 찾음", "CLI not found")} />
            ))
          }
        >
          {status?.cli_registered ? (
            <p>
              {t(
                <>
                  등록돼 있어요. 아무 폴더에서나 <b className={em}>claude</b>를 실행하면 자동으로
                  연결돼요.
                </>,
                <>
                  Registered. Run <b className={em}>claude</b> in any folder and it connects on its
                  own.
                </>,
              )}
            </p>
          ) : tempNoPath ? (
            // 임시 경로를 등록해 주면 마운트 해제 순간 죽는 등록이 남는다.
            <p>
              {t(
                <>
                  앱이 임시 위치(디스크 이미지 등)에서 실행 중이에요. Budu를{" "}
                  <b className={em}>응용 프로그램</b> 폴더로 옮겨서 실행한 뒤 다시 연결하세요.
                </>,
                <>
                  The app is running from a temporary location (a disk image, for example). Move
                  Budu to <b className={em}>Applications</b>, open it from there, and connect again.
                </>,
              )}
            </p>
          ) : status?.cli_path ? (
            <div className="space-y-2.5">
              <p>
                {status.cli_registered_other
                  ? t(
                      "budu 등록이 있지만 다른 경로(옛 설치 등)를 가리키고 있어요. 다시 등록하면 이 앱으로 바로잡아요.",
                      "A budu entry exists, but it points somewhere else (an older install). Registering again points it at this app.",
                    )
                  : t(
                      "버튼 한 번으로 등록돼요. 다음 claude 실행부터 어느 폴더에서든 연결돼요.",
                      "One button registers it. From the next claude run on, it works in any folder.",
                    )}
              </p>
              <button
                type="button"
                onClick={() => void connectCode()}
                disabled={cliBusy}
                className={cn(primaryBtn, "w-full")}
              >
                {cliBusy ? (
                  <Loader2 size={14} className="animate-spin" />
                ) : status.cli_registered_other ? (
                  t("이 앱으로 다시 등록", "Point it at this app")
                ) : (
                  t("Claude Code에 연결", "Connect Claude Code")
                )}
              </button>
              {cliError && (
                <div className="space-y-2">
                  <p className="text-[11px] leading-relaxed text-red-500/90">{cliError.message}</p>
                  {/* 대행이 실패한 사람에게 남은 길 = 수동 등록. 에러 문구가 "아래 명령"을
                      가리키므로 여기서 실제로 보여준다. */}
                  <CodeBox text={reRegisterCommand} />
                  <button
                    type="button"
                    onClick={() => copy(reRegisterCommand)}
                    className={cn(secondaryBtn, "w-full")}
                  >
                    {copied ? <Check size={13} /> : <Copy size={13} />} {t("명령 복사", "Copy command")}
                  </button>
                  {/* 🔴 위 시퀀스의 remove 는 우리가 방금 되돌려 놓은 옛 등록도 지운다.
                      대행이 실패한 환경에서는 손으로 쳐도 add 가 또 실패하기 쉬워서,
                      그대로 두면 멀쩡하던 등록만 잃는다. */}
                  {restoreCmd && (
                    <div className="space-y-2 pt-1">
                      <p className="text-[11px] leading-relaxed text-amber-700 dark:text-amber-400">
                        {t(
                          <>
                            위 명령의 <b>remove</b>는 되돌려 둔 기존 budu 등록도 지워요. 뒤의 add가
                            또 실패하면 아래 명령으로 원래대로 되돌릴 수 있어요.
                          </>,
                          <>
                            The <b>remove</b> above also deletes the budu entry we put back. If the
                            add after it fails again, this command restores it.
                          </>,
                        )}
                      </p>
                      <CodeBox text={restoreCmd} />
                      <button
                        type="button"
                        onClick={() => copyRestore(restoreCmd)}
                        className={cn(secondaryBtn, "w-full")}
                      >
                        {restoreCopied ? <Check size={13} /> : <Copy size={13} />}{" "}
                        {t("되돌리기 명령 복사", "Copy restore command")}
                      </button>
                    </div>
                  )}
                </div>
              )}
            </div>
          ) : (
            <div className="space-y-2.5">
              <p>
                {t(
                  "claude 명령을 찾지 못했어요. Claude Code를 쓰고 있다면 터미널에서 직접 등록하세요:",
                  "Couldn't find the claude command. If you use Claude Code, register it yourself in a terminal:",
                )}
              </p>
              <CodeBox text={cliCommand} />
              <button
                type="button"
                onClick={() => copy(cliCommand)}
                className={cn(secondaryBtn, "w-full")}
              >
                {copied ? <Check size={13} /> : <Copy size={13} />} {t("명령 복사", "Copy command")}
              </button>
            </div>
          )}
        </ClientCard>

        {/* 그 외 MCP 클라이언트 — 경로만 있으면 어디든 */}
        <ClientCard icon={<Bot size={14} />} title={t("다른 AI 앱 (Cursor 등)", "Other AI apps (Cursor and friends)")}>
          {tempNoPath ? (
            // 이 상태의 mcpPath 폴백(/Applications/…)은 아직 없는 파일이다 — 복사하게
            // 두면 죽은 경로를 등록한다. 같은 "옮기기" 안내로 대체.
            <p>
              {t(
                <>
                  앱이 임시 위치(디스크 이미지 등)에서 실행 중이에요. Budu를{" "}
                  <b className={em}>응용 프로그램</b> 폴더로 옮겨서 실행하면 등록할 경로가 생겨요.
                </>,
                <>
                  The app is running from a temporary location (a disk image, for example). Move
                  Budu to <b className={em}>Applications</b> and open it from there to get a path
                  worth registering.
                </>,
              )}
            </p>
          ) : (
          <div className="space-y-2.5">
            <p>
              {t(
                "MCP를 지원하는 앱이면 어디든 붙어요. 서버로 이 경로를 등록하면 돼요:",
                "Any app that speaks MCP can use this. Register this path as the server:",
              )}
            </p>
            <CodeBox text={mcpPath} />
            <div className="flex items-center justify-between gap-2">
              <button
                type="button"
                onClick={() => copyPath(mcpPath)}
                className="inline-flex items-center gap-1 text-[11px] text-[var(--color-ink-500)] hover:text-[var(--color-accent)] transition-colors"
              >
                {pathCopied ? <Check size={11} /> : <Copy size={11} />} {t("경로 복사", "Copy path")}
              </button>
              <button
                type="button"
                onClick={() => openUrl(`${GITHUB_URL}#readme`).catch(() => {})}
                className="inline-flex items-center gap-1 text-[11px] text-[var(--color-ink-500)] hover:text-[var(--color-accent)] transition-colors"
              >
                <ExternalLink size={11} /> {t("GitHub 문서", "GitHub docs")}
              </button>
            </div>
          </div>
          )}
        </ClientCard>
      </motion.div>

      <div />
    </main>
  );
}
