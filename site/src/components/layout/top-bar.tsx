"use client";

import * as React from "react";
import { localizedPath } from "@/i18n/paths";
import type { Locale } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { LINKS, NAV_ANCHORS } from "@/lib/home-content";
import { LanguageSwitcher } from "./language-switcher";
import { CtaLink } from "@/components/shared/primitives";
import { MascotMark } from "@/components/shared/mascot-svg";

/**
 * S0 — fixed top bar. Transparent over the hero; a backdrop blur and a 1 px
 * foam hairline arrive once the page has scrolled 40 px (plan §4 S0). The
 * links are plain anchors to the sections; on phones only the wordmark, the
 * language switch and the install button stay, because four links would not
 * fit beside them. The EN · ID switch is WEB-5 D3; the button is D8.
 *
 * WEB-5 D15: the bar is shared by the home page and the contact page. On
 * the home page the section links are in-page anchors; on the contact page
 * they lead back to the same sections of the home page in the visitor's
 * language. The Contact link marks itself as the current page there.
 */
export type TopBarPage = "home" | "contact";

export function TopBar({
  locale,
  page = "home",
  nav,
  cta,
  language,
}: {
  locale: Locale;
  page?: TopBarPage;
  nav: CommonCopy["nav"];
  cta: CommonCopy["cta"];
  language: CommonCopy["language"];
}) {
  const home = localizedPath(locale, "/");
  const anchor = (hash: string) => (page === "home" ? hash : `${home}${hash}`);

  // WEB-5 D5: no React state per scroll event. The scrolled look is a
  // `data-scrolled` attribute written straight to the header, and only when
  // it flips, so scrolling never re-renders the bar. A plain passive
  // listener rather than motion's `useScroll`, whose scroll-timeline code
  // was about 4 KB of the first JavaScript for one comparison (measured).
  const headerRef = React.useRef<HTMLElement>(null);
  React.useEffect(() => {
    const apply = () => {
      const el = headerRef.current;
      if (!el) return;
      const next = window.scrollY > 40;
      if (el.hasAttribute("data-scrolled") !== next) el.toggleAttribute("data-scrolled", next);
    };
    apply();
    window.addEventListener("scroll", apply, { passive: true });
    return () => window.removeEventListener("scroll", apply);
  }, []);

  return (
    <header
      ref={headerRef}
      className="fixed inset-x-0 top-0 z-50 border-b border-transparent bg-transparent backdrop-blur-0 transition-[background-color,backdrop-filter,border-color] duration-500 data-[scrolled]:border-[#E8F4FF]/10 data-[scrolled]:bg-[#020514]/55 data-[scrolled]:backdrop-blur-xl"
    >
      <div className="mx-auto flex h-16 w-full max-w-[1280px] items-center justify-between gap-2 px-4 md:gap-4 md:px-6">
        <a href={anchor("#hero")} className="flex items-center gap-2 text-[15px] md:gap-2.5 font-medium tracking-tight text-[#E8F4FF]">
          {/* O2: the official mark at its own ratio, sized by height. */}
          <MascotMark className="h-5 w-auto" />
          {/* Never wraps: beside the language switch a 360 px phone has room for one line. */}
          <span className="whitespace-nowrap">RantAI Lakehouse</span>
        </a>
        {/* WEB-5 D15: five links with the Contact link. From 768 to 1023 px they sit tighter, or the
            install button ran 38 px past a 768 px screen (measured). */}
        <nav aria-label={nav.label} className="hidden items-center md:flex lg:gap-1">
          {NAV_ANCHORS.map((l) => (
            <a
              key={l.href}
              href={anchor(l.href)}
              className="rounded-full px-2.5 py-1.5 text-[14px] text-[#E8F4FF]/75 lg:px-3.5 transition-colors hover:text-[#E8F4FF]"
            >
              {nav[l.key]}
            </a>
          ))}
          <a
            href={localizedPath(locale, "/contact")}
            aria-current={page === "contact" ? "page" : undefined}
            className="rounded-full px-2.5 py-1.5 text-[14px] text-[#E8F4FF]/75 lg:px-3.5 transition-colors hover:text-[#E8F4FF] aria-[current=page]:text-[#E8F4FF]"
          >
            {nav.contact}
          </a>
        </nav>
        <div className="flex items-center gap-1.5 md:gap-3">
          <LanguageSwitcher locale={locale} text={language} />
          <CtaLink href={LINKS.install} size="sm">
            {cta.install}
          </CtaLink>
        </div>
      </div>
    </header>
  );
}
