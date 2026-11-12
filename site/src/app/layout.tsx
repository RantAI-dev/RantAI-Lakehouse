import type { Metadata, Viewport } from "next";
import { SITE_NAME } from "@/lib/seo";
import { siteUrl } from "@/lib/site-url";
import "./globals.css";

/**
 * The top layout renders no `<html>` of its own: each language writes its
 * own files with its own `lang` (WEB-5 D3), so the document is rendered one
 * level down by `app/(en)/layout.tsx` and `app/[locale]/layout.tsx` (and by
 * `not-found.tsx`), through `components/layout/site-document.tsx`. This
 * pass-through layout is next-intl's documented shape for a root
 * `not-found` without middleware.
 */
export const metadata: Metadata = {
  // WEB-5 D4: every relative URL in metadata (canonical, hreflang, og:url,
  // og:image) is resolved against the origin the build was given (D2).
  metadataBase: new URL(siteUrl()),
  title: { default: SITE_NAME, template: `%s — ${SITE_NAME}` },
  applicationName: SITE_NAME,
  icons: {
    // The favicon follows the browser colour scheme, as in the console:
    // light = navy logo, dark = white logo.
    icon: [
      { url: "/icon-light-32.png", type: "image/png", media: "(prefers-color-scheme: light)" },
      { url: "/icon-dark-32.png", type: "image/png", media: "(prefers-color-scheme: dark)" },
    ],
    apple: "/apple-touch-icon.png",
  },
  manifest: "/site.webmanifest",
  appleWebApp: { title: SITE_NAME },
};

/** Browser chrome takes the abyss colour so the page reaches the screen edges. */
export const viewport: Viewport = {
  themeColor: "#020514",
};

export default function RootLayout({ children }: Readonly<{ children: React.ReactNode }>) {
  return children;
}
