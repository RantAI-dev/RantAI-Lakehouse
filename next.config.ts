import type { NextConfig } from "next";
import { devOrigins } from "./src/lib/dev-origins";

// `OPS-11`: Next.js 16.4 serves the dev server's own resources (HMR, chunks)
// only to `localhost` unless the host is listed here, so a dev console opened
// by a LAN address never finishes loading. `CONSOLE_ALLOWED_DEV_ORIGINS`
// (comma-separated hostnames, `*.` wildcards allowed, see docs/CONFIGURATION.md) adds
// hosts; `127.0.0.1` is always allowed (`BI-1 QA fix`, see `src/lib/dev-origins.ts`).
// It affects `next dev` only: a production build ignores it.
const allowedDevOrigins = devOrigins(process.env.CONSOLE_ALLOWED_DEV_ORIGINS);

const nextConfig: NextConfig = {
  allowedDevOrigins,

  // `OPS-40`: `next dev` writes a managed agent-rules block into the project
  // root's `AGENTS.md` whenever it detects an AI coding agent
  // (`node_modules/next/dist/server/lib/generate-agent-files.js`). That
  // writer is not new in 16.4.5: it was already active on 16.4.3 and rewrote
  // this repository's `AGENTS.md` on 2026-11-04
  // (`docs/superpowers/plans/2026-11-04-web-3-console-cleanup.md:462`), before
  // the advisories behind Next's own security gate were published. On 16.4.3
  // that gate stops the dev server, so the console cannot be started locally
  // at all; on 16.4.5 the dev server starts again, and the writer would run
  // on every `bun run dev` and rewrite the agent-rules file every session
  // reads first: this repository's own root `AGENTS.md`. Keep the writer off;
  // the guard in `src/app/agent-rules.test.ts` fails if it is switched back on.
  agentRules: false,

  // Jejak build mandiri (server + node_modules minimal) supaya image Docker
  // konsol tetap ramping — dipakai stack demo `rantai-lake-demo`.
  output: "standalone",

  experimental: {
    // The /api rewrite below is a proxy, and Next.js 16 buffers a proxied
    // request body only up to `proxyClientMaxBodySize` (default 10 MB); past
    // it the body is cut, not refused. An upload of more than 10 MB then
    // reaches the API as a broken multipart form and the browser gets a bare
    // 500, so the 50 MB the Upload file page promises silently did not work.
    // This mirrors the API's `MAX_REQUEST_BODY_BYTES` in
    // `rust/crates/lakehouse-api/src/routes/uploads.rs` (`MAX_UPLOAD_BYTES`,
    // 50 MiB, plus 1 MiB of multipart framing). Keep the two equal: a smaller
    // value here cuts uploads the API would take, and a larger one buys
    // nothing because the API refuses the excess itself.
    proxyClientMaxBodySize: 51 * 1024 * 1024,
    // Next.js's /api proxy gives up on an upstream that has not answered in 30
    // seconds (`proxyTimeout`, 30000 ms by default, in next's `proxy-request.js`).
    // `SRC-13` decision D9 lets a table listing take up to 120 seconds (a schema of
    // 10,000 tables) and the API's route timeout for the two discovery paths is 130
    // (`DISCOVERY_REQUEST_TIMEOUT` in `rust/crates/lakehouse-api/src/routes/mod.rs`), so with the
    // default the console would report a failure for a listing the API was still
    // answering. Keep this equal to the LARGEST route timeout in that function: the
    // chart and table downloads (`EXPORT_REQUEST_TIMEOUT`, 310 s; BI-1 review fix
    // (BLOCKER) R2) are longer than the discovery paths, and the proxy would cut a
    // long Excel download at the old 130 s while the API was still sending it.
    proxyTimeout: 310_000,
  },

  // WEB-18: the History page is gone (the chats are listed in a conversation's
  // sidebar); its old address opens the newest conversation, which
  // `/copilot` resolves from `open=latest`. A real HTTP redirect, not a page
  // calling `redirect()`: in this app a page's redirect only takes effect
  // when the client renders the page, and `RouteGuard` (`app-frame.tsx`)
  // renders "You don't have access" instead for an address it has no
  // `ROUTE_ACCESS` entry for, so the redirect never ran.
  async redirects() {
    return [{ source: "/copilot/history", destination: "/copilot?open=latest", permanent: false }];
  },

  // Proxy /api/* to the Rust backend service. Browser URLs stay the same;
  // Next.js only serves the UI while the Rust axum service owns the API.
  async rewrites() {
    const target = process.env.RUST_API_URL ?? "http://localhost:8080";
    return [
      // The API's health route lives at `/health`, NOT under `/api/`, so
      // the console could not reach it through the rewrite below (it
      // would ask the API for `/api/health`, which is not a route). This
      // one exposes it as `/api/health`, and it comes FIRST: the rewrites
      // are matched top to bottom, so the general `/api/:path*` below
      // never sees this path. The console's own `/health` PAGE (the
      // platform dashboard) is a filesystem route, and filesystem routes
      // are served before any rewrite, so the two cannot collide.
      { source: "/api/health", destination: `${target}/health` },
      { source: "/api/:path*", destination: `${target}/api/:path*` },
    ];
  },
};

export default nextConfig;
