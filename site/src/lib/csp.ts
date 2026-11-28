import { createHash } from "node:crypto";

/**
 * The site's Content Security Policy (WEB-5 D18), from rantai-page's
 * `next.config.ts` policy, cut to what this site loads. `bun run build`
 * writes it into every exported page as a `<meta http-equiv>`, first in
 * `<head>` (`scripts/csp-meta.ts`), so it holds on any static host before a
 * host is chosen (D2). `next dev` has none: hot reload needs eval and a
 * websocket, as on rantai-page.
 *
 * - Everything comes from this origin: scripts, styles, fonts, the tiles,
 *   the icons and the scene's chunk. There is no third-party host to allow.
 * - Scripts: `'self'` plus the SHA-256 of each inline script on that page,
 *   computed at build time from the exported HTML (`scripts/csp-meta.ts`),
 *   never written by hand. Next's export writes its bootstrap and the page
 *   data as inline scripts; a hash lets exactly those run, and any other
 *   inline script (an injected one) is refused. There is no `'unsafe-inline'`
 *   for scripts. `application/ld+json` blocks are data, not executed, so
 *   they need no hash. Limit: a hash policy is per page, so it lives in
 *   each page's own meta tag and changes with every build.
 * - Styles keep `'unsafe-inline'`: React's `style` attributes (several
 *   hundred on the home page) and motion's transforms are inline styles,
 *   and a CSP cannot hash style attributes without `'unsafe-hashes'` and a
 *   hash per value. Inline styles cannot run script; this is the known limit.
 * - `img-src data: blob:`: three.js and the browser hand textures to WebGL
 *   this way.
 * - `form-action 'none'`: the contact form posts nowhere; it opens an e-mail
 *   from script (D15), so a submit without JavaScript is refused rather than
 *   sent to the host with the visitor's message in it.
 * - `frame-ancestors`, `report-uri` and `sandbox` are ignored in a meta tag
 *   (and `frame-ancestors` logs a console error there), so they are not in
 *   it: framing is refused by the host headers in `site/README.md`.
 *   `upgrade-insecure-requests` is left to HSTS for the same reason: every
 *   URL on the page is relative to the page's own origin.
 */
export function contentSecurityPolicy(scriptHashes: readonly string[]): string {
  return [
    "default-src 'self'",
    ["script-src 'self'", ...scriptHashes.map((h) => `'${h}'`)].join(" "),
    "style-src 'self' 'unsafe-inline'",
    "img-src 'self' data: blob:",
    "font-src 'self'",
    "connect-src 'self'",
    "media-src 'self'",
    "worker-src 'self' blob:",
    "manifest-src 'self'",
    "frame-src 'none'",
    "object-src 'none'",
    "base-uri 'self'",
    "form-action 'none'",
  ].join("; ");
}

/** The `'sha256-…'` source value for one inline script's exact text. */
export function scriptHash(text: string): string {
  return `sha256-${createHash("sha256").update(text, "utf8").digest("base64")}`;
}

/**
 * The inline, executable scripts of an exported page, as their exact text:
 * every `<script>` without `src`, except data blocks (`application/ld+json`),
 * which the browser never runs.
 */
export function inlineScripts(html: string): string[] {
  const found: string[] = [];
  for (const m of html.matchAll(/<script\b([^>]*)>([\s\S]*?)<\/script>/g)) {
    const attrs = m[1];
    if (/\bsrc=/.test(attrs) || /type="application\/ld\+json"/.test(attrs)) continue;
    found.push(m[2]);
  }
  return found;
}
