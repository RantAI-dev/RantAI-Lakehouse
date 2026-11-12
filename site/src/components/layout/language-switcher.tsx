"use client";

import { usePathname } from "@/i18n/navigation";
import { localizedPath } from "@/i18n/paths";
import { routing, type Locale } from "@/i18n/routing";
import type { CommonCopy } from "@/lib/copy";
import { cn } from "@/lib/utils";

/** Remembered choice; the suggestion stops appearing once it is set. */
export const LOCALE_CHOICE_KEY = "rantai-lakehouse-locale-choice";

/**
 * EN · ID toggle (WEB-5 D3, after the RantAI site's
 * `components/layout/language-switcher.tsx`, recoloured for the night
 * palette). Each link goes to the same page in the other language and
 * remembers the choice in `localStorage` (never a cookie: D1). `hrefLang`
 * and `lang` tell assistive tech and crawlers which language each link is
 * in, and the current one carries `aria-current` (D10).
 *
 * Plain `<a>` elements: the two languages have different root layouts, so
 * a switch is a full page load whatever the link is, and `next/link` would
 * only add its prefetch code to the first JavaScript (D5).
 *
 * The href is built with `localizedPath`, not next-intl's `Link locale=`:
 * that one keeps the `/en/` prefix when switching to the default language so
 * a middleware can redirect it, and this export has no middleware, so the
 * link would point at a file that does not exist (found in T2).
 *
 * The labels arrive as props from the server, not through next-intl's
 * client hooks: those would ship its ICU message parser to the browser for
 * four short strings (measured in T5, WEB-5 D5).
 */
export function LanguageSwitcher({
  locale,
  text,
  className,
}: Readonly<{ locale: Locale; text: CommonCopy["language"]; className?: string }>) {
  const pathname = usePathname();

  return (
    <nav
      aria-label={text.label}
      className={cn("flex items-center rounded-full bg-[#E8F4FF]/[0.07] p-0.5", className)}
    >
      {routing.locales.map((l) => {
        const current = l === locale;
        return (
          <a
            key={l}
            href={localizedPath(l, pathname)}
            hrefLang={l}
            lang={l}
            aria-current={current ? "true" : undefined}
            // WEB-5 review fix R2 (WCAG 2.5.3, label in name): the accessible name
            // starts with the visible "EN"/"ID", so a voice command that says what
            // the link shows still finds it.
            aria-label={`${text.short[l]}: ${current ? text[l] : text.switchTo.replace("{language}", text[l])}`}
            onClick={() => {
              try {
                localStorage.setItem(LOCALE_CHOICE_KEY, l);
              } catch {
                // Storage can be off (private mode); the link still works.
              }
            }}
            className={cn(
              "rounded-full px-2 py-1 text-[12px] font-medium transition-colors md:px-2.5",
              current ? "bg-[#E8F4FF] text-[#020514]" : "text-[#E8F4FF]/70 hover:text-[#E8F4FF]",
            )}
          >
            {text.short[l]}
          </a>
        );
      })}
    </nav>
  );
}
