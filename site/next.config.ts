import path from "node:path";
import type { NextConfig } from "next";
import createNextIntlPlugin from "next-intl/plugin";
import { parseSiteUrl } from "./src/lib/site-url";

// WEB-5 D2: the site's origin is required at build time and has no default.
// Checked here, when Next loads its config, so a missing or wrong value stops
// `next build` (and `next dev`) before any page renders, with a message that
// names the variable, instead of failing later inside metadata.
parseSiteUrl(process.env.NEXT_PUBLIC_SITE_URL);

const nextConfig: NextConfig = {
  // WEB-5 D1: a static export. `next build` writes plain files to `out/`,
  // which any static host serves: no server, no API, no cookies.
  output: "export",
  // The image optimizer is a server feature; an export cannot run it.
  images: { unoptimized: true },
  // `/id/` is written as `id/index.html`, which every static host serves for
  // the directory URL without rewrite rules.
  trailingSlash: true,
  poweredByHeader: false,
  // The site sits inside the console's repository, which has its own
  // lockfile. Without this Turbopack takes the repository root as the
  // project and compiles the console's `src/proxy.ts` into the site.
  turbopack: { root: path.resolve(__dirname) },
};

// WEB-5 D3: next-intl reads its per-locale messages from this file.
const withNextIntl = createNextIntlPlugin("./src/i18n/request.ts");

export default withNextIntl(nextConfig);
