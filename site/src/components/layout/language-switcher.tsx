"use client";

import Link from "next/link";
import { useLocale, useTranslations } from "next-intl";
import { usePathname } from "@/i18n/navigation";
import { localizedPath } from "@/i18n/paths";
import { routing, type Locale } from "@/i18n/routing";
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
 * The href is built with `localizedPath`, not next-intl's `Link locale=`:
 * that one keeps the `/en/` prefix when switching to the default language so
 * a middleware can redirect it, and this export has no middleware, so the
 * link would point at a file that does not exist (found in T2).
 */
export function LanguageSwitcher({ className }: Readonly<{ className?: string }>) {
  const t = useTranslations("common.language");
  const locale = useLocale() as Locale;
  const pathname = usePathname();

  return (
    <nav
      aria-label={t("label")}
      className={cn("flex items-center rounded-full bg-[#E8F4FF]/[0.07] p-0.5", className)}
    >
      {routing.locales.map((l) => {
        const current = l === locale;
        return (
          <Link
            key={l}
            href={localizedPath(l, pathname)}
            hrefLang={l}
            lang={l}
            aria-current={current ? "true" : undefined}
            aria-label={current ? t(l) : t("switchTo", { language: t(l) })}
            onClick={() => {
              try {
                localStorage.setItem(LOCALE_CHOICE_KEY, l);
              } catch {
                // Storage can be off (private mode); the link still works.
              }
            }}
            className={cn(
              "rounded-full px-2.5 py-1 text-[12px] font-medium transition-colors",
              current ? "bg-[#E8F4FF] text-[#020514]" : "text-[#E8F4FF]/70 hover:text-[#E8F4FF]",
            )}
          >
            {t(`short.${l}`)}
          </Link>
        );
      })}
    </nav>
  );
}
