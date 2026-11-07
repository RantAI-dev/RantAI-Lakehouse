// WEB-5 D18: write the Content Security Policy into every exported page, as
// the first element after `<meta charset>`, so it covers every script and
// stylesheet the page loads. Runs as the second half of `bun run build`.
//
// Why a post-build step: a `<meta http-equiv>` applies only to what the
// parser meets after it, and React places a `<meta>` rendered in the layout
// after Next's own `<script src>` tags (measured: 3,200 bytes later). Writing
// it here also keeps it out of React's tree, so hydration never sees it.
// The policy itself is `src/lib/csp.ts`, tested in `src/lib/csp.test.ts`.

import { readdirSync, readFileSync, writeFileSync, existsSync } from "node:fs";
import { join } from "node:path";
import { CONTENT_SECURITY_POLICY } from "../src/lib/csp";

const OUT = join(import.meta.dir, "../out");
if (!existsSync(OUT)) {
  console.error("csp-meta: out/ does not exist; `next build` must run first.");
  process.exit(1);
}
const CHARSET = '<meta charSet="utf-8"/>';
const META = `<meta http-equiv="Content-Security-Policy" content="${CONTENT_SECURITY_POLICY}"/>`;

let failed = false;
let written = 0;
for (const file of readdirSync(OUT, { recursive: true }).map(String).filter((f) => f.endsWith(".html"))) {
  const path = join(OUT, file);
  let html = readFileSync(path, "utf8");
  if (html.includes('http-equiv="Content-Security-Policy"')) {
    console.error(`csp-meta: ${file} already has a policy; it is written only here.`);
    failed = true;
    continue;
  }
  const at = html.indexOf(CHARSET);
  if (at < 0) {
    console.error(`csp-meta: ${file} has no ${CHARSET}; the policy has nowhere to go first.`);
    failed = true;
    continue;
  }
  html = html.slice(0, at + CHARSET.length) + META + html.slice(at + CHARSET.length);
  const csp = html.indexOf(META);
  const firstLoad = Math.min(...["<script", "<link", "<style"].map((t) => html.indexOf(t)).filter((i) => i >= 0));
  if (csp > firstLoad) {
    console.error(`csp-meta: in ${file} the policy would come after the first script or stylesheet.`);
    failed = true;
    continue;
  }
  writeFileSync(path, html);
  written++;
}
console.log(`csp-meta: the policy is first in <head> in ${written} pages`);
process.exit(failed || written === 0 ? 1 : 0);
