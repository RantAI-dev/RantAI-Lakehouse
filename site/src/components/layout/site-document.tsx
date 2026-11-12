import { getTranslations } from "next-intl/server";
import { routing, type Locale } from "@/i18n/routing";
import { MotionFeatures } from "@/components/shared/motion-features";
import { fontVariables } from "@/lib/fonts";
import { DeferredLanguageSuggestion } from "./deferred-language-suggestion";
import type { SuggestionText } from "./language-suggestion";

/**
 * The `<html>` shell of one language, shared by the two layouts that write
 * files: `app/(en)` (English, `out/index.html`) and `app/[locale]`
 * (Indonesian, `out/id/index.html`). WEB-5 D3: a static export has no
 * middleware, so each language is its own set of files and `lang` is fixed
 * per file (D10).
 *
 * No messages reach the browser as a whole: every text is read on the
 * server and handed to the client leaves as props, and there is no
 * `NextIntlClientProvider`, so neither the message files nor next-intl's
 * client runtime ship (D5, measured in T5).
 */
export async function SiteDocument({
  locale,
  children,
}: Readonly<{ locale: Locale; children: React.ReactNode }>) {
  // The offer of Indonesian is written in Indonesian (see the component).
  const suggestion =
    locale === routing.defaultLocale
      ? ((await getTranslations({ locale: "id", namespace: "common" })).raw("suggestion") as SuggestionText)
      : null;

  return (
    // `suppressHydrationWarning`: `motion-features.tsx` adds `motion-ready`
    // to this element's classes in the browser, on purpose.
    <html lang={locale} className={fontVariables} suppressHydrationWarning>
      <body>
        <MotionFeatures>
          {children}
          {suggestion && <DeferredLanguageSuggestion text={suggestion} />}
        </MotionFeatures>
      </body>
    </html>
  );
}
