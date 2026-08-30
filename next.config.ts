import type { NextConfig } from "next";

const nextConfig: NextConfig = {
  // Jejak build mandiri (server + node_modules minimal) supaya image Docker
  // konsol tetap ramping — dipakai stack demo `rantai-lake-demo`.
  output: "standalone",

  // Proxy /api/* to the Rust backend service. Browser URLs stay the same;
  // Next.js only serves the UI while the Rust axum service owns the API.
  async rewrites() {
    const target = process.env.RUST_API_URL ?? "http://localhost:8080";
    return [{ source: "/api/:path*", destination: `${target}/api/:path*` }];
  },
};

export default nextConfig;
