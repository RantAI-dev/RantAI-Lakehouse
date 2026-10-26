import type { NextConfig } from "next";
import { devOrigins } from "./src/lib/dev-origins";

// `OPS-11`: Next.js 16.4 serves the dev server's own resources (HMR, chunks)
// only to `localhost` unless the host is listed here, so a dev console opened
// by a LAN address never finishes loading. `CONSOLE_ALLOWED_DEV_ORIGINS`
// (comma-separated hostnames, `*.` wildcards allowed, see `.env.example`) adds
// hosts; `127.0.0.1` is always allowed (`BI-1 QA fix`, see `src/lib/dev-origins.ts`).
// It affects `next dev` only: a production build ignores it.
const allowedDevOrigins = devOrigins(process.env.CONSOLE_ALLOWED_DEV_ORIGINS);

const nextConfig: NextConfig = {
  allowedDevOrigins,

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

  // Proxy /api/* to the Rust backend service. Browser URLs stay the same;
  // Next.js only serves the UI while the Rust axum service owns the API.
  async rewrites() {
    const target = process.env.RUST_API_URL ?? "http://localhost:8080";
    return [{ source: "/api/:path*", destination: `${target}/api/:path*` }];
  },
};

export default nextConfig;
