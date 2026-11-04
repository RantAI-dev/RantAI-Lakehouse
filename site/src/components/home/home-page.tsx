import { getTranslations } from "next-intl/server";
import { LanguageSwitcher } from "@/components/layout/language-switcher";
import type { Locale } from "@/i18n/routing";

/**
 * The home page, shared by its two routes (`app/(en)/page.tsx` and
 * `app/[locale]/page.tsx`), hence `components/home/` rather than a route's
 * `_components/` (WEB-5 D9). Placeholder until T3 moves the landing here.
 */
export async function HomePage({ locale }: Readonly<{ locale: Locale }>) {
  const t = await getTranslations({ locale, namespace: "home.hero" });
  return (
    <main>
      <LanguageSwitcher />
      <p>{t("eyebrow")}</p>
      <h1>{t("headline")}</h1>
    </main>
  );
}
