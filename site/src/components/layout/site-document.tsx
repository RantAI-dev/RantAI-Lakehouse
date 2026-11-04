import { NextIntlClientProvider } from "next-intl";
import { getMessages, getTranslations } from "next-intl/server";
import { routing, type Locale } from "@/i18n/routing";
import { fontVariables } from "@/lib/fonts";
import { LanguageSuggestion, type SuggestionText } from "./language-suggestion";

/**
 * The `<html>` shell of one language, shared by the two layouts that write
 * files: `app/(en)` (English, `out/index.html`) and `app/[locale]`
 * (Indonesian, `out/id/index.html`). WEB-5 D3: a static export has no
 * middleware, so each language is its own set of files and `lang` is fixed
 * per file (D10).
 *
 * Only the `common` messages reach the browser: section text is read on the
 * server and handed to the client leaves as props, so the page does not ship
 * its own copy twice (D5).
 */
export async function SiteDocument({
  locale,
  children,
}: Readonly<{ locale: Locale; children: React.ReactNode }>) {
  const messages = await getMessages({ locale });
  // The offer of Indonesian is written in Indonesian (see the component).
  const suggestion =
    locale === routing.defaultLocale
      ? ((await getTranslations({ locale: "id", namespace: "common" })).raw("suggestion") as SuggestionText)
      : null;

  return (
    <html lang={locale} className={fontVariables}>
      <body>
        <NextIntlClientProvider locale={locale} messages={{ common: messages.common }}>
          {children}
          {suggestion && <LanguageSuggestion text={suggestion} />}
        </NextIntlClientProvider>
      </body>
    </html>
  );
}
