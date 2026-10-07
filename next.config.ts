import type { NextConfig } from "next";

const nextConfig: NextConfig = {
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
  },

  // Proxy /api/* to the Rust backend service. Browser URLs stay the same;
  // Next.js only serves the UI while the Rust axum service owns the API.
  async rewrites() {
    const target = process.env.RUST_API_URL ?? "http://localhost:8080";
    return [{ source: "/api/:path*", destination: `${target}/api/:path*` }];
  },
};

export default nextConfig;
