// 시드 백업 플로우.
// onboard: 새 지갑 생성 직후 (비번 보유) → 안내 → 12단어 → 확인 퀴즈
// review:  나중에 다시 보기 → 비번 입력 → 안내 → 12단어 → 확인(퀴즈 없음)

import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { motion } from "framer-motion";
import { ArrowLeft, Check, Eye, EyeOff, KeyRound, Loader2 } from "lucide-react";
import { cn } from "@/lib/cn";
import {
  cardBase,
  flatCard,
  enter,
  pageShell,
  primaryBtn,
  ScreenHeader,
  shell,
  FlowIcon,
  PwInput,
} from "@/components/ui";
import { t } from "@/lib/i18n";
import { isWindow } from "@/lib/win";
import { Eyebrow, ink, muted } from "@/components/win";

/** 큰 창 백업 왼쪽 단 — 12단어를 지키는 법 둘. */
function BackupGuide() {
  const items: [string, string][] = [
    [t("종이에 적기", "Write it on paper"), t("종이에 적은 후 안전한 금고에 보관하세요.", "Write it on paper, then keep it in a safe.")],
    [t("외부 저장 금지", "Never store it elsewhere"), t("스크린샷·메모 앱·메일 등에 시드 기록을 남기지 마세요.", "Don't leave your seed in screenshots, notes apps, email or anywhere else.")],
  ];
  // 최소 폭 — 옆 판이 폭을 먹어도 글자가 한 자씩 세로로 떨어지지 않게.
  return (
    <div className="min-w-[220px]">
      <Eyebrow className="mb-3">{t("지키는 법", "Keeping it safe")}</Eyebrow>
      <ol className="flex flex-col gap-6">
        {items.map(([h, b], i) => (
          <li key={h} className="flex gap-4">
            <span className="num w-7 h-7 shrink-0 rounded-full grid place-items-center text-md font-semibold bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)] text-[var(--color-ink-700)] dark:text-[#B5AFA2]">
              {i + 1}
            </span>
            <div className="min-w-0">
              <p className={cn("text-md font-semibold", ink)}>{h}</p>
              <p className={cn("mt-1 text-md leading-relaxed", muted)}>{b}</p>
            </div>
          </li>
        ))}
      </ol>
    </div>
  );
}

type BackupStep = "password" | "loading" | "intro" | "words" | "verify";
type Challenge = { index: number; answer: string; options: string[] };

function shuffle<T>(arr: T[]): T[] {
  const a = [...arr];
  for (let i = a.length - 1; i > 0; i--) {
    const j = Math.floor(Math.random() * (i + 1));
    [a[i], a[j]] = [a[j], a[i]];
  }
  return a;
}

/** 12단어 중 3개를 골라 객관식 확인 문제를 만든다. 보기는 같은 시드의 다른 단어들. */
function makeChallenges(words: string[]): Challenge[] {
  const idxs = shuffle(words.map((_, i) => i)).slice(0, 3).sort((a, b) => a - b);
  return idxs.map((index) => {
    const answer = words[index];
    const decoyPool = shuffle([...new Set(words.filter((w) => w !== answer))]);
    const options = shuffle([answer, ...decoyPool.slice(0, 3)]);
    return { index, answer, options };
  });
}

export function BackupFlow({
  mode,
  initialPassword,
  onComplete,
  onExit,
}: {
  mode: "onboard" | "review";
  initialPassword?: string;
  onComplete: () => void;
  onExit: () => void;
}) {
  const [step, setStep] = useState<BackupStep>(initialPassword ? "loading" : "password");
  const [pw, setPw] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [words, setWords] = useState<string[] | null>(null);
  const [revealed, setRevealed] = useState(false);
  const [challenges, setChallenges] = useState<Challenge[] | null>(null);
  // 가져온 지갑은 15~24단어일 수 있다 — 복호화 전엔 모르니 그때만 「단어」로 부른다.
  const n = words?.length ?? 12;

  const reveal = useCallback(async (password: string) => {
    setBusy(true);
    setError(null);
    try {
      const w = await invoke<string[]>("reveal_mnemonic", { password });
      setWords(w);
      setStep("intro");
    } catch (e) {
      setError(String(e));
      setStep("password");
    } finally {
      setBusy(false);
    }
  }, []);

  // onboard: 비번을 들고 있으니 바로 열람한다.
  useEffect(() => {
    if (initialPassword) void reveal(initialPassword);
  }, [initialPassword, reveal]);

  async function finish() {
    try {
      await invoke("mark_backed_up");
    } catch {
      /* 표시 실패해도 백업 자체는 됐으니 통과 */
    }
    onComplete();
  }

  const wideReview = isWindow && mode === "review";
  return (
    <main className={mode === "review" ? pageShell : shell}>
      <ScreenHeader
        icon={<KeyRound size={12} />}
        title={t("시드 백업", "Back up your words")}
        onClose={onExit}
        closeLabel={mode === "onboard" ? t("나중에", "Later") : t("닫기", "Close")}
        railed={mode === "review"}
      />

      {/* 큰 창 — 왼쪽 = 지키는 법 안내, 오른쪽 = 단계 판(가라앉은 둥근 면). 넓은 판에 칸 하나만 떠 있던 «폰 화면을 아이패드에 띄운» 모양을 걷는다. */}
      <div className={cn(wideReview ? "w-full grid" : "contents", wideReview && " grid-cols-1 min-[820px]:grid-cols-[minmax(220px,1fr)_minmax(0,440px)] gap-10 min-[1100px]:gap-16 items-start")}>
      {wideReview && <BackupGuide />}
      <motion.section {...enter} className={cn(
          mode === "review" ? flatCard : cardBase,
          "max-w-md px-8 py-9",
          // 큰 창 — 페이지 제목이 이미 위에 있으니 가운데 아이콘을 빼고 단계 제목·설명을 왼쪽 선에 맞춘다.
          mode === "review" &&
            "win:max-w-none win:[&_.flow-icon]:hidden win:[&_h1]:mt-0 win:[&_h1]:text-left win:[&_h1]:text-lg win:[&_h1]:font-semibold win:[&_p]:text-left",
          wideReview && "win:!bg-[var(--color-ivory-200)]/60 win:dark:!bg-[var(--color-night-800)] win:!rounded-[20px] win:!px-7 win:!py-7",
        )}>
        {/* 0) 비번 입력 (review 진입 시) */}
        {step === "password" && (
          <>
            <FlowIcon><KeyRound size={22} /></FlowIcon>
            <h1 className="mt-5 text-center text-lg tracking-tight">
              {t("시드 키 보기", "Show your seed")}
            </h1>
            <p className="mt-2 text-center text-md leading-relaxed text-[var(--color-ink-500)]">
              {t("키를 복호화하려면 비밀번호가 필요해요.", "Your password is needed to decrypt the key.")}
            </p>
            <div className="mt-7">
              <PwInput
                value={pw}
                onChange={setPw}
                placeholder={t("비밀번호", "Password")}
                autoFocus
                onEnter={() => pw && reveal(pw)}
              />
            </div>
            {error && <p className="mt-4 text-sm text-[var(--color-danger)] font-mono break-all">{error}</p>}
            <button type="button" onClick={() => reveal(pw)} disabled={!pw || busy} className={cn(primaryBtn, "mt-6")}>
              {busy ? <Loader2 size={15} className="animate-spin" /> : <Eye size={15} />}
              {t("단어 보기", "Show words")}
            </button>
          </>
        )}

        {/* 로딩 (onboard 초기 복호화) */}
        {step === "loading" && (
          <div className="flex flex-col items-center py-10">
            <Loader2 size={26} className="animate-spin text-[var(--color-ok)]" />
            <p className="mt-4 text-md text-[var(--color-ink-500)]">{t("시드 준비 중…", "Getting your words…")}</p>
            {error && <p className="mt-4 text-sm text-[var(--color-danger)] font-mono break-all">{error}</p>}
          </div>
        )}

        {/* 1) 안내 */}
        {step === "intro" && (
          <>
            <FlowIcon><KeyRound size={22} /></FlowIcon>
            <h1 className="mt-5 text-center text-lg tracking-tight">
              {t("복구 시드 백업", "Back up your recovery words")}
            </h1>
            <p className="mt-3 text-center text-md leading-relaxed text-[var(--color-ink-500)]">
              {t(
                <>
                  이 {n}단어가 자산의{" "}
                  <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">진짜 소유 증명</b>
                  이에요. 비밀번호를 잊어도 이 단어만 있으면 가져오기나 다른 표준 지갑에서 자산을
                  되찾을 수 있어요.
                </>,
                <>
                  These {n} words are{" "}
                  <b className="text-[var(--color-ink-700)] dark:text-[#E8E5DD]">
                    the real proof the funds are yours
                  </b>
                  . Even if you forget your password, they bring the funds back — here through
                  Import, or in any standard wallet.
                </>,
              )}
            </p>
            <ul className="mt-5 space-y-2 text-sm leading-relaxed text-[var(--color-ink-500)]">
              <li>{t("• 종이에 적어 안전한 곳에 보관하세요.", "• Write them on paper and keep it somewhere safe.")}</li>
              <li>{t("• 누구에게도 보여주지 마세요 — 단어 = 자산.", "• Never show them to anyone — the words are the money.")}</li>
              <li>{t("• 스크린샷·캡처는 피하세요.", "• Avoid screenshots and screen recordings.")}</li>
            </ul>
            <button type="button" onClick={() => setStep("words")} className={cn(primaryBtn, "mt-7")}>
              <Eye size={15} />
              {t(`${n}단어 보기`, `Show the ${n} words`)}
            </button>
          </>
        )}

        {/* 2) 12단어 표시 */}
        {step === "words" && words && (
          <>
            <div className="flex items-center justify-between">
              <p className="text-sm tracking-[0.04em] text-[var(--color-ink-500)]">
                {t(`복구 시드 (${n}단어)`, `Recovery phrase (${n} words)`)}
              </p>
              <button
                type="button"
                onClick={() => setRevealed((v) => !v)}
                className="inline-flex items-center gap-1.5 text-sm text-[var(--color-ink-500)] hover:text-[var(--color-ink-900)] dark:hover:text-[#E8E5DD] transition-colors"
              >
                {revealed ? <EyeOff size={13} /> : <Eye size={13} />}
                {revealed ? t("가리기", "Hide") : t("보기", "Show")}
              </button>
            </div>

            {/* 클립보드 경로를 두지 않는다 — 복사 버튼은 물론 드래그 선택도 막는다
                (select-none 상시). 클립보드에 들어간 시드는 클립보드 매니저가 디스크에 평문으로
                남기고, 유니버설 클립보드로 근처 아이폰·아이패드에 자동 전파된다 — "키는 이 맥을
                안 떠난다"는 약속이 조용히 깨지는 경로라 경고가 아니라 경로 제거로 막는다. */}
            <div className="relative mt-4">
              <div className={cn("grid grid-cols-2 gap-2 transition-all select-none", !revealed && "blur-sm")}>
                {words.map((w, i) => (
                  <div
                    key={i}
                    className={cn(
                      "flex items-center gap-2.5 h-11 px-3 rounded-[10px]",
                      "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
                      "border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]",
                    )}
                  >
                    <span className="w-4 text-right num text-sm text-[var(--color-ink-300)]">{i + 1}</span>
                    <span className="font-mono text-md text-[var(--color-ink-900)] dark:text-[#E8E5DD]">{w}</span>
                  </div>
                ))}
              </div>
              {!revealed && (
                <button
                  type="button"
                  onClick={() => setRevealed(true)}
                  className="absolute inset-0 flex items-center justify-center text-md text-[var(--color-ink-700)] dark:text-[#B5AFA2]"
                >
                  <span className="inline-flex items-center gap-2 px-4 py-2 rounded-[var(--radius-pill)] bg-[var(--color-ivory-50)]/80 dark:bg-[var(--color-night-800)]/80 border border-[var(--color-ivory-300)] dark:border-[var(--color-night-700)]">
                    <Eye size={14} /> {t("탭해서 보기", "Tap to reveal")}
                  </span>
                </button>
              )}
            </div>

            <p className="mt-4 text-center text-sm leading-relaxed text-[var(--color-ink-300)]">
              {t(
                "번호 순서대로 종이에 적어 주세요 — 복구엔 순서까지 맞아야 해요.",
                "Write them on paper in numbered order — recovery needs the order too.",
              )}
            </p>

            {mode === "onboard" ? (
              <button
                type="button"
                onClick={() => {
                  setChallenges(makeChallenges(words));
                  setStep("verify");
                }}
                className={cn(primaryBtn, "mt-5")}
              >
                {t("적었어요, 확인하기", "I wrote them down — check me")}
              </button>
            ) : (
              <button type="button" onClick={finish} className={cn(primaryBtn, "mt-5")}>
                <Check size={15} /> {t("확인했어요", "Done")}
              </button>
            )}
          </>
        )}

        {/* 3) 확인 퀴즈 (onboard 전용) */}
        {step === "verify" && challenges && (
          <BackupQuiz challenges={challenges} onPass={finish} onBack={() => setStep("words")} />
        )}
      </motion.section>
      </div>

      <div />
    </main>
  );
}

function BackupQuiz({
  challenges,
  onPass,
  onBack,
}: {
  challenges: Challenge[];
  onPass: () => void;
  onBack: () => void;
}) {
  const [qi, setQi] = useState(0);
  const [wrong, setWrong] = useState(false);
  const c = challenges[qi];

  function pick(opt: string) {
    if (opt === c.answer) {
      setWrong(false);
      if (qi + 1 >= challenges.length) onPass();
      else setQi(qi + 1);
    } else {
      setWrong(true);
    }
  }

  return (
    <div>
      <div className="flex items-center gap-2">
        <button
          type="button"
          onClick={onBack}
          className="text-[var(--color-ink-300)] hover:text-[var(--color-ink-700)] transition-colors"
          aria-label={t("뒤로", "Back")}
        >
          <ArrowLeft size={16} />
        </button>
        <p className="text-sm tracking-[0.04em] text-[var(--color-ink-500)]">
          {t(
            `백업 확인 (${qi + 1}/${challenges.length})`,
            `Backup check (${qi + 1}/${challenges.length})`,
          )}
        </p>
      </div>

      <p className="mt-6 text-center text-md tracking-tight">
        {t(
          <>
            <span className="num text-[var(--color-ok)]">{c.index + 1}</span>번째 단어는?
          </>,
          <>
            Which word is <span className="num text-[var(--color-ok)]">#{c.index + 1}</span>?
          </>,
        )}
      </p>

      <div className="mt-5 grid grid-cols-2 gap-2">
        {c.options.map((opt) => (
          <button
            key={opt}
            type="button"
            onClick={() => pick(opt)}
            className={cn(
              "h-11 rounded-[var(--radius-card)] font-mono text-md",
              "border border-[var(--color-ivory-400)] dark:border-[var(--color-night-700)]",
              "bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]",
              "text-[var(--color-ink-900)] dark:text-[#E8E5DD]",
              "transition-all duration-[var(--duration-base)] ease-[cubic-bezier(0.4,0,0.2,1)]",
              "hover:border-[var(--color-ok)] hover:translate-y-[-1px]",
              "active:translate-y-0",
            )}
          >
            {opt}
          </button>
        ))}
      </div>

      {wrong && (
        <p className="mt-4 text-center text-sm text-[var(--color-danger)]">
          {t("틀렸어요. 다시 확인하고 골라주세요.", "Not that one. Check your notes and pick again.")}
        </p>
      )}
    </div>
  );
}
