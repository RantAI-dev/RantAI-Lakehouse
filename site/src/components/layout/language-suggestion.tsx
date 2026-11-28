"use client";

import * as React from "react";
import { AnimatePresence, m, useReducedMotion } from "framer-motion";
import { Languages, X } from "lucide-react";
import { usePathname } from "@/i18n/navigation";
import { localizedPath } from "@/i18n/paths";
import { LOCALE_CHOICE_KEY } from "./language-switcher";

export type SuggestionText = { label: string; text: string; view: string; dismiss: string; reopen: string };

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
 *
 * WEB-5 audit #6: it must not sit over the content. After the hero (or
 * 8 s, whichever comes first) it folds into a small "ID" pill in the same
 * corner, which opens it again; while the footer is in view it is hidden.
 */
const FOLD_MS = 8000;

export default function LanguageSuggestion({ text }: Readonly<{ text: SuggestionText }>) {
  const pathname = usePathname();
  const reduce = useReducedMotion();
  const [state, setState] = React.useState<"off" | "open" | "pill">("off");
  const [footerInView, setFooterInView] = React.useState(false);
  const first = React.useRef(true);
  const held = React.useRef(false);

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
    const t = window.setTimeout(() => setState("open"), 1500);
    return () => window.clearTimeout(t);
  }, []);

  // Open: fold into the pill after 8 s, or (the first time) once the hero has scrolled away.
  React.useEffect(() => {
    if (state !== "open") return;
    let timer = 0;
    const fold = () => {
      if (held.current) {
        timer = window.setTimeout(fold, 2000);
        return;
      }
      setState("pill");
    };
    timer = window.setTimeout(fold, FOLD_MS);
    const hero = first.current ? document.getElementById("hero") : null;
    first.current = false;
    const io = hero
      ? new IntersectionObserver(([e]) => {
          if (!e.isIntersecting) fold();
        })
      : null;
    if (hero) io?.observe(hero);
    return () => {
      window.clearTimeout(timer);
      io?.disconnect();
    };
  }, [state]);

  // Hidden while the footer is in view, in either form.
  React.useEffect(() => {
    if (state === "off") return;
    const footer = document.querySelector("footer");
    if (!footer) return;
    const io = new IntersectionObserver(([e]) => setFooterInView(e.isIntersecting));
    io.observe(footer);
    return () => io.disconnect();
  }, [state]);

  const remember = (choice: string) => {
    try {
      localStorage.setItem(LOCALE_CHOICE_KEY, choice);
    } catch {
      // No storage: it shows again next visit, which is harmless.
    }
    setState("off");
  };

  const motion = {
    initial: reduce ? false : { opacity: 0, y: 12 },
    animate: { opacity: 1, y: 0 },
    exit: { opacity: 0, y: 8 },
    transition: { duration: 0.3, ease: [0.22, 1, 0.36, 1] as const },
  };
  const hold = {
    onPointerEnter: () => (held.current = true),
    onPointerLeave: () => (held.current = false),
    onFocus: () => (held.current = true),
    onBlur: () => (held.current = false),
  };

  return (
    <AnimatePresence mode="wait">
      {state === "open" && !footerInView && (
        <m.aside
          key="open"
          lang="id"
          aria-label={text.label}
          {...motion}
          {...hold}
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
      {state === "pill" && !footerInView && (
        <m.button
          key="pill"
          type="button"
          lang="id"
          aria-label={text.reopen}
          onClick={() => setState("open")}
          {...motion}
          className="fixed bottom-3 left-3 z-[90] inline-flex h-10 items-center gap-1.5 rounded-full bg-[#020514]/85 px-3.5 font-sans text-[13px] font-semibold tracking-[0.04em] text-[#E8F4FF] shadow-[0_18px_40px_-16px_rgba(0,0,0,0.8)] ring-1 ring-[#E8F4FF]/15 backdrop-blur-xl transition-[background-color,box-shadow] duration-200 ease-out hover:bg-[#050A30]/90 hover:ring-[#8fcbff]/40 focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-[#44A6FA] sm:bottom-6 sm:left-6"
        >
          <Languages className="size-4 text-[#44A6FA]" aria-hidden />
          ID
        </m.button>
      )}
    </AnimatePresence>
  );
}
