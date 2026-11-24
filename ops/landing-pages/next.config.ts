import type { NextConfig } from "next";

/**
 * The published landing page's Next config.
 *
 * A separate config (and a separate project directory) from the console's,
 * because the two builds want opposite things: the console is a standalone
 * server with `/api` rewrites, request-time CSP nonces and `force-dynamic`
 * layouts (`SEC-47`), while this page is a file tree served by GitHub Pages.
 * `distDir` stays `.next` and the export lands in `out`; the build runner
 * (`scripts/build-landing-pages.ts`) copies it to `out-landing` and adds the
 * `CNAME` and `.nojekyll` files GitHub Pages needs.
 */
const nextConfig: NextConfig = {
  // A static export: the artifact is plain HTML/CSS/JS, no server.
  output: "export",
  // The image optimizer needs a server; GitHub Pages has none.
  images: { unoptimized: true },
  // `OPS-40`: keep `next dev`/`next build` from writing Next's managed
  // agent-rules block into this repository's `AGENTS.md`, same reason as the
  // console's config (`src/app/agent-rules.test.ts`).
  agentRules: false,
};

export default nextConfig;
