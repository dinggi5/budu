// 도움말 화면 — 팝오버는 섹션 카드 스택, 큰 창은 주제 목록 + 본문 2단.
// 콘텐츠는 helpContent.tsx 한 곳에 있다.

import { useState } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { motion } from "framer-motion";
import { ExternalLink, HelpCircle } from "lucide-react";
import { cn } from "@/lib/cn";
import { GITHUB_URL, HELP_SECTIONS } from "@/lib/helpContent";
import { cardBase, pageShell, ScreenHeader } from "@/components/ui";
import { Eyebrow, PageTitle, Selectable, ink, inkSoft, muted, sunken, useWidth } from "@/components/win";
import { isWindow } from "@/lib/win";
import { t } from "@/lib/i18n";

export function HelpScreen({ onClose }: { onClose: () => void }) {
  if (isWindow) return <HelpWindow />;
  return (
    <main className={cn(pageShell, "justify-start gap-4")}>
      <ScreenHeader icon={<HelpCircle size={12} />} title={t("도움말", "Help")} onClose={onClose} />
      <div className="w-full max-w-md flex flex-col gap-3 pb-4">
        {HELP_SECTIONS.map((s, i) => (
          <motion.section
            key={s.id}
            initial={{ opacity: 0, y: 8 }}
            animate={{ opacity: 1, y: 0 }}
            transition={{ duration: 0.3, delay: i * 0.04, ease: [0.4, 0, 0.2, 1] }}
            className={cn(cardBase, "px-6 py-5")}
          >
            <div className="flex items-start gap-3.5">
              <div className="shrink-0 w-10 h-10 rounded-full flex items-center justify-center bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)] text-[var(--color-accent)]">
                {s.icon}
              </div>
              <div className="flex-1 min-w-0 pt-0.5">
                <h2 className="text-[15px] tracking-tight text-[var(--color-ink-900)] dark:text-[#E8E5DD]">
                  {s.title}
                </h2>
                <div className="mt-2 text-[12.5px] leading-relaxed text-[var(--color-ink-500)]">
                  {s.body}
                </div>
              </div>
            </div>
          </motion.section>
        ))}

        <button
          type="button"
          onClick={() => openUrl(GITHUB_URL).catch(() => {})}
          className="mt-1 inline-flex items-center justify-center gap-1.5 text-[12px] text-[var(--color-ink-500)] hover:text-[var(--color-accent)] transition-colors"
        >
          <ExternalLink size={12} />
          {t("GitHub에서 전체 문서 보기", "Read the full docs on GitHub")}
        </button>
      </div>
    </main>
  );
}

/** 큰 창 — 왼쪽 단 = 주제 목록(고르면 회색 둥근 면), 오른쪽 = 고른 주제 하나(장부와 같은 2단). 본문 14px. */
function HelpWindow() {
  // 창 폭이 아니라 실제 자리(레일·오른쪽 패널 뺀 것) — 장부와 같은 기준.
  const [page, pageW] = useWidth<HTMLDivElement>();
  const wide = pageW >= 690;
  const [id, setId] = useState(HELP_SECTIONS[0]?.id);
  const sec = HELP_SECTIONS.find((s) => s.id === id) ?? HELP_SECTIONS[0];
  const list = (
    <ul className="flex flex-col gap-0.5">
      {HELP_SECTIONS.map((s) => (
        <li key={s.id}>
          <Selectable on={s.id === sec.id} onClick={() => setId(s.id)} className="px-3 py-2">
            <span className={cn("flex items-start gap-2 text-[14px] leading-snug", s.id === sec.id ? cn(ink, "font-semibold") : inkSoft)}>
              <span className="shrink-0 mt-0.5 inline-flex text-[var(--color-ink-500)] [&_svg]:w-[15px] [&_svg]:h-[15px]">{s.icon}</span>
              <span className="min-w-0">{s.title}</span>
            </span>
          </Selectable>
        </li>
      ))}
    </ul>
  );
  return (
    <div ref={page} className="flex h-screen min-w-0 flex-1 bg-[var(--color-ivory-100)] dark:bg-[var(--color-night-900)]">
      {wide && (
        <nav className={cn("h-full w-[228px] shrink-0 overflow-y-auto px-3 pt-14 pb-6", sunken)}>
          <Eyebrow className="px-3 mb-2">{t("도움말", "Help")}</Eyebrow>
          {list}
          <button
            type="button"
            onClick={() => openUrl(GITHUB_URL).catch(() => {})}
            className={cn("mt-6 px-3 inline-flex items-center gap-1.5 text-[12px] hover:text-[var(--color-accent)] transition-colors", muted)}
          >
            <ExternalLink size={12} />
            {t("GitHub 전체 문서", "Full docs on GitHub")}
          </button>
        </nav>
      )}
      <section className="flex-1 min-w-0 h-full overflow-y-auto px-6 min-[1100px]:px-10 pt-14 pb-12">
        {!wide && (
          <select
            value={sec.id}
            onChange={(e) => setId(e.target.value)}
            aria-label={t("주제", "Topic")}
            className={cn("mb-5 h-8 max-w-full rounded-full px-3 text-[13px] outline-none bg-[var(--color-ivory-200)] dark:bg-[var(--color-night-700)]", ink)}
          >
            {HELP_SECTIONS.map((s) => (
              <option key={s.id} value={s.id}>
                {s.title}
              </option>
            ))}
          </select>
        )}
        <motion.div key={sec.id} initial={{ opacity: 0, y: 6 }} animate={{ opacity: 1, y: 0 }} transition={{ duration: 0.2, ease: [0.4, 0, 0.2, 1] }}>
          <PageTitle title={sec.title} />
          <div className={cn("mt-5 text-[15px] leading-[1.75] max-w-[72ch]", inkSoft)}>{sec.body}</div>
          {!wide && (
            <button
              type="button"
              onClick={() => openUrl(GITHUB_URL).catch(() => {})}
              className={cn("mt-8 inline-flex items-center gap-1.5 text-[12px] hover:text-[var(--color-accent)] transition-colors", muted)}
            >
              <ExternalLink size={12} />
              {t("GitHub 전체 문서", "Full docs on GitHub")}
            </button>
          )}
        </motion.div>
      </section>
    </div>
  );
}
