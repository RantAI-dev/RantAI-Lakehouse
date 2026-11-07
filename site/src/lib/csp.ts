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
 * - `'unsafe-inline'` scripts: Next's export writes its bootstrap and the
 *   page data as inline scripts, and a nonce or a hash per build would need
 *   a server (rantai-page has the same exception, for the same reason).
 *   Inline styles: React's `style` attributes and motion's transforms.
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
export const CONTENT_SECURITY_POLICY = [
  "default-src 'self'",
  "script-src 'self' 'unsafe-inline'",
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
