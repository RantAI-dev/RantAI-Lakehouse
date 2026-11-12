import { prefixedLocales, routing, type Locale } from "./routing";

/**
 * A path in a given language: English at the root, Indonesian under `/id`.
 * Paths end in `/` because the export writes `id/index.html`
 * (`trailingSlash: true`), so the canonical URL is the one the host serves
 * without a redirect.
 */
export function localizedPath(locale: Locale, path: string): string {
  const clean = path === "/" ? "/" : `/${path.replace(/^\/+|\/+$/g, "")}/`;
  if (locale === routing.defaultLocale) return clean;
  return `/${locale}${clean}`;
}

/** hreflang links: the page in every language, English as the default. */
export function languageAlternates(path: string): Record<Locale | "x-default", string> {
  return {
    ...(Object.fromEntries(routing.locales.map((l) => [l, localizedPath(l, path)])) as Record<Locale, string>),
    "x-default": localizedPath(routing.defaultLocale, path),
  };
}

/** The path with any language prefix removed: `/id/` -> `/`, `/id/x` -> `/x`. */
export function unprefixedPath(pathname: string): string {
  for (const l of prefixedLocales) {
    if (pathname === `/${l}` || pathname === `/${l}/`) return "/";
    if (pathname.startsWith(`/${l}/`)) return pathname.slice(l.length + 1);
  }
  return pathname || "/";
}
