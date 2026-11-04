import { hasLocale } from "next-intl";
import { getRequestConfig } from "next-intl/server";
import { NAMESPACES } from "./namespaces";
import { routing } from "./routing";

/**
 * Messages are split by area: `src/messages/<locale>/<namespace>.json`, and
 * each file is a top-level namespace (`common.json` -> `t("common.x")`). A
 * new namespace is added to `namespaces.ts` and in both languages; the
 * parity test (`src/messages/messages.test.ts`) fails on a file one
 * language lacks.
 */
export default getRequestConfig(async ({ requestLocale }) => {
  // Every layout and page calls `setRequestLocale`, so this is the locale
  // the file being written belongs to; there is no request to read it from.
  const requested = await requestLocale;
  const locale = hasLocale(routing.locales, requested) ? requested : routing.defaultLocale;

  const entries = await Promise.all(
    NAMESPACES.map(async (ns) => [ns, (await import(`../messages/${locale}/${ns}.json`)).default]),
  );

  return { locale, messages: Object.fromEntries(entries) };
});
