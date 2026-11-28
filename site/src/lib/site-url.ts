/**
 * The site's own public address, from `NEXT_PUBLIC_SITE_URL` at build time.
 *
 * Why it is required and has no default (WEB-5 D2, principle 3): every
 * canonical URL, hreflang link, Open Graph URL, the sitemap and the robots
 * file are absolute, and a guessed origin would publish wrong ones that
 * search engines then keep. Hosting is not decided yet, so the builder names
 * the origin; a build without one, or with one that is not an absolute
 * `https://` origin, stops with the message below instead of shipping
 * `localhost` links.
 *
 * Only an origin is accepted (no path, query or fragment): the export is
 * built for the root of its host, and a path here would need a `basePath`
 * the site does not set.
 */

export const SITE_URL_ENV = "NEXT_PUBLIC_SITE_URL";

export class SiteUrlError extends Error {
  constructor(detail: string) {
    super(
      `${SITE_URL_ENV} ${detail}. Set it to the absolute https:// origin the site is served from, ` +
        `for example ${SITE_URL_ENV}=https://lakehouse.example.com bun run build (see site/README.md).`,
    );
    this.name = "SiteUrlError";
  }
}

/** Parses and checks the value; returns the origin with no trailing slash. */
export function parseSiteUrl(value: string | undefined): string {
  const raw = value?.trim();
  if (!raw) throw new SiteUrlError("is not set");
  let url: URL;
  try {
    url = new URL(raw);
  } catch {
    throw new SiteUrlError(`is not a URL (${JSON.stringify(raw)})`);
  }
  if (url.protocol !== "https:") throw new SiteUrlError(`must use https:// (got ${url.protocol}//)`);
  if (url.username || url.password) throw new SiteUrlError("must not carry credentials");
  if (url.pathname !== "/" || url.search || url.hash) {
    throw new SiteUrlError(`must be an origin only, with no path, query or fragment (got ${JSON.stringify(raw)})`);
  }
  return url.origin;
}

/**
 * The checked origin for code that runs in the build. `next.config.ts` has
 * already refused a bad value before any page renders, so this only throws
 * when a module is used outside a Next build (a test that forgot to set it).
 */
export function siteUrl(): string {
  return parseSiteUrl(process.env.NEXT_PUBLIC_SITE_URL);
}
