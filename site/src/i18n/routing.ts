import { defineRouting } from "next-intl/routing";

/**
 * English lives at the root (`/`), Indonesian under `/id/` (WEB-5 D3, the
 * RantAI site's setup). No redirect by browser language: Indonesian visitors
 * are offered the switch instead (`components/layout/language-suggestion`).
 *
 * The export has no middleware, so this object is not used to route
 * requests: the two locales are two sets of files (`app/(en)` writes
 * `out/index.html`, `app/[locale]` writes `out/id/index.html`). It still
 * drives `Link`, `usePathname` and the paths helper, so links agree with
 * those files.
 */
export const routing = defineRouting({
  locales: ["en", "id"],
  defaultLocale: "en",
  localePrefix: "as-needed",
  localeDetection: false,
});

export type Locale = (typeof routing.locales)[number];

/** Every locale except the default: the ones `app/[locale]` writes. */
export const prefixedLocales = routing.locales.filter((l) => l !== routing.defaultLocale);
