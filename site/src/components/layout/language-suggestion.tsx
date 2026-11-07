"use client";

import * as React from "react";
import { AnimatePresence, m, useReducedMotion } from "framer-motion";
import { Languages, X } from "lucide-react";
import { usePathname } from "@/i18n/navigation";
import { localizedPath } from "@/i18n/paths";
import { LOCALE_CHOICE_KEY } from "./language-switcher";

export type SuggestionText = { label: string; text: string; view: string; dismiss: string };

/**
 * Offers the Indonesian page to a visitor whose browser prefers Indonesian,
 * instead of redirecting them (WEB-5 D3: `localeDetection: false`; a
 * redirect by language confuses search engines and visitors who chose
 * English on purpose). The RantAI site's
 * `components/layout/language-suggestion.tsx`: the same behaviour, place,
 * sizes and words, in this site's own look (owner QA finding O1: the
 * verbatim copy was that site's white light-theme card). It takes the top
 * bar's glass (the abyss at 85 % with a blur and a foam hairline), the
 * primary button of the page for "Lihat" and a quiet close button, all in
 * the logo's palette (`lib/palette.ts`). It renders outside `.lk-root`, so
 * it sets its own focus ring, the same one the page uses.
 *
 * Rendered on English pages only, and written in Indonesian, since that is
 * who sees it: the layout passes the `id` locale's `common.suggestion`
 * strings as `text`. Shown once; picking a language or closing it is
 * remembered.
 */
export default function LanguageSuggestion({ text }: Readonly<{ text: SuggestionText }>) {
  const pathname = usePathname();
  const reduce = useReducedMotion();
  const [open, setOpen] = React.useState(false);

  React.useEffect(() => {
    let chosen: string | null = null;
    try {
      chosen = localStorage.getItem(LOCALE_CHOICE_KEY);
    } catch {
      // No storage: treat as not chosen.
    }
    if (chosen) return;
    const prefersIndonesian = (navigator.languages ?? [navigator.language])
      .slice(0, 3)
      .some((l) => /^(id|in)\b/i.test(l));
    if (!prefersIndonesian) return;
    const t = window.setTimeout(() => setOpen(true), 1500);
    return () => window.clearTimeout(t);
  }, []);

  const remember = (choice: string) => {
    try {
      localStorage.setItem(LOCALE_CHOICE_KEY, choice);
    } catch {
      // No storage: it shows again next visit, which is harmless.
    }
    setOpen(false);
  };

  return (
    <AnimatePresence>
      {open && (
        <m.aside
          lang="id"
          aria-label={text.label}
          initial={reduce ? false : { opacity: 0, y: 16 }}
          animate={{ opacity: 1, y: 0 }}
          exit={{ opacity: 0, y: 12 }}
          transition={{ duration: 0.4, ease: [0.22, 1, 0.36, 1] }}
          className="fixed inset-x-3 bottom-3 z-[90] flex items-center gap-1.5 rounded-2xl bg-[#020514]/85 py-2 pr-1 pl-3.5 font-sans text-[#E8F4FF] shadow-[0_18px_40px_-16px_rgba(0,0,0,0.8)] ring-1 ring-[#E8F4FF]/15 backdrop-blur-xl sm:right-auto sm:bottom-6 sm:left-6 sm:max-w-[calc(100vw-2rem)] sm:gap-3 sm:py-2.5 sm:pr-2 sm:pl-3.5"
        >
          <Languages className="size-4 shrink-0 text-[#44A6FA] max-sm:hidden" aria-hidden />
          <p className="min-w-0 flex-1 text-[13.5px] sm:flex-initial sm:text-[14px]">{text.text}</p>
          {/* A plain link: a language switch is a full page load (other root layout). */}
          <a
            href={localizedPath("id", pathname)}
            hrefLang="id"
            onClick={() => remember("id")}
            className="inline-flex h-10 shrink-0 items-center rounded-full bg-[var(--lk-primary)] px-3.5 text-[13px] font-medium text-white transition-[background-color,transform] duration-200 ease-out hover:bg-[var(--lk-primary-hover)] active:bg-[var(--lk-primary-press)] active:scale-[0.97] focus-visible:outline-2 focus-visible:outline-offset-3 focus-visible:outline-[#44A6FA] sm:h-auto sm:px-3.5 sm:py-1.5"
          >
            {text.view}
          </a>
          <button
            type="button"
            onClick={() => remember("en")}
            aria-label={text.dismiss}
            className="grid size-10 shrink-0 place-items-center rounded-full text-[#E8F4FF]/70 transition-[background-color,color,transform] duration-200 ease-out hover:bg-[#E8F4FF]/10 hover:text-[#E8F4FF] active:scale-90 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#44A6FA] sm:size-8"
          >
            <X className="size-4" />
          </button>
        </m.aside>
      )}
    </AnimatePresence>
  );
}
