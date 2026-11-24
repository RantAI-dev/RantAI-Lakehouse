import type { Metadata, Viewport } from "next";
import { geist, geistMono } from "@rantai/design-system/fonts/fonts";
import { displayFont } from "@/features/landing/landing-fonts";
import "./globals.css";

/**
 * The static root layout for the published landing page.
 *
 * Why this exists instead of reusing `src/app/layout.tsx`: that layout is the
 * console's, and it depends on what a static export cannot have. It reads the
 * request's CSP nonce through `headers()` and marks every route
 * `force-dynamic` (`SEC-47`), it mounts `AuthProvider`/`AppFrame` and asks the
 * API for the workspace's branding (`ADM-2`), and the console's Next config
 * sets `output: "standalone"` with `/api` rewrites. `output: "export"` over
 * that tree fails, so `ops/landing-pages` is a minimal Next project whose only
 * route is this page. It shares the landing components and the design system,
 * nothing else.
 *
 * It renders at build time — no `headers()`, no `force-dynamic`, no request —
 * which is exactly what a static export needs.
 */

const DESCRIPTION =
  "One lakehouse with dashboards built in. Bring every source in, keep it in open tables, and ask for answers in plain language.";

export const metadata: Metadata = {
  metadataBase: new URL("https://lakehouse.rantai.dev"),
  title: { absolute: "RantAI Lakehouse — your data has a home" },
  description: DESCRIPTION,
  applicationName: "RantAI Lakehouse",
  alternates: { canonical: "/" },
  openGraph: {
    title: "RantAI Lakehouse — your data has a home",
    description: DESCRIPTION,
    siteName: "RantAI Lakehouse",
    type: "website",
    url: "/",
  },
  manifest: "/site.webmanifest",
};

/** Browser chrome takes the abyss colour so the page reaches the screen edges. */
export const viewport: Viewport = {
  themeColor: "#020514",
};

/**
 * The product's favicons, the same files the console's root layout falls back
 * to when a workspace has none (`workspaceIconLinks`, `ADM-2`).
 */
function IconLinks() {
  return (
    <>
      <link
        rel="icon"
        href="/icon-light-32.png"
        type="image/png"
        media="(prefers-color-scheme: light)"
      />
      <link
        rel="icon"
        href="/icon-dark-32.png"
        type="image/png"
        media="(prefers-color-scheme: dark)"
      />
      <link rel="apple-touch-icon" href="/apple-touch-icon.png" />
      {/* Browsers request `/favicon.ico` on their own; the build's `public/`
          symlink carries the console's own file there. */}
      <link rel="icon" href="/favicon.ico" sizes="any" />
    </>
  );
}

export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      className={`${geist.variable} ${geistMono.variable} font-sans antialiased`}
    >
      <head>
        <IconLinks />
      </head>
      <body className={displayFont.variable}>{children}</body>
    </html>
  );
}
