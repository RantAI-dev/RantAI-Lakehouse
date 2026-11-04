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
 * English on purpose). A copy of the RantAI site's
 * `components/layout/language-suggestion.tsx`.
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
          className="fixed inset-x-3 bottom-3 z-[90] flex items-center gap-1.5 rounded-2xl bg-white py-2 pr-1 pl-3.5 text-neutral-950 shadow-[0_18px_40px_-16px_rgba(0,0,0,0.35)] ring-1 ring-black/[0.07] sm:right-auto sm:bottom-6 sm:left-6 sm:max-w-[calc(100vw-2rem)] sm:gap-3 sm:py-2.5 sm:pr-2 sm:pl-3.5"
        >
          <Languages className="size-4 shrink-0 text-neutral-500 max-sm:hidden" aria-hidden />
          <p className="min-w-0 flex-1 text-[13.5px] sm:flex-initial sm:text-[14px]">{text.text}</p>
          {/* A plain link: a language switch is a full page load (other root layout). */}
          <a
            href={localizedPath("id", pathname)}
            hrefLang="id"
            onClick={() => remember("id")}
            className="inline-flex h-9 shrink-0 items-center rounded-full bg-neutral-950 px-3.5 text-[13px] font-medium text-white transition-colors hover:bg-neutral-800 sm:h-auto sm:px-3.5 sm:py-1.5"
          >
            {text.view}
          </a>
          <button
            type="button"
            onClick={() => remember("en")}
            aria-label={text.dismiss}
            className="grid size-10 shrink-0 place-items-center rounded-full text-neutral-500 transition-colors hover:bg-neutral-100 hover:text-neutral-950 sm:size-8"
          >
            <X className="size-4" />
          </button>
        </m.aside>
      )}
    </AnimatePresence>
  );
}
