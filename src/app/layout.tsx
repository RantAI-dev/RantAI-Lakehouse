import type { Metadata } from "next";
import { headers } from "next/headers";
import { geist, geistMono } from "@rantai/design-system/fonts/fonts";
import { ThemeProvider } from "@rantai/design-system/components/theme-provider";
import { Toaster } from "@rantai/design-system/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AppFrame } from "@/components/app-shell/app-frame";
import { TableProviders } from "@/components/app-shell/table-providers";
import { AuthProvider } from "@/features/auth/auth-provider";
import { ReactGrabDev } from "@/components/app-shell/react-grab-dev";
import { NONCE_HEADER } from "@/lib/csp";
import { dashboardFontVariables } from "./fonts";
import "./globals.css";

/**
 * `SEC-47`: every route renders per request.
 *
 * The console's `Content-Security-Policy` carries a fresh nonce for each
 * request (`src/proxy.ts`, `src/lib/csp.ts`), and Next applies that nonce to
 * its own inline bootstrap and chunks only while it renders a page for a
 * request. A page prerendered at build time has no nonce in its scripts, and
 * with `script-src 'strict-dynamic'` a browser refuses every one of them, so
 * the console would not hydrate at all. Reading the nonce below already makes
 * a route request-rendered; this export states the requirement itself, so
 * moving the theme provider does not quietly turn pages static again. It is
 * the price of a nonce policy: no static shell, no CDN page cache.
 */
export const dynamic = "force-dynamic";

export const metadata: Metadata = {
  title: "Rantai Lake",
  description: "Rantai Lake - Data platform",
  manifest: "/site.webmanifest",
  appleWebApp: { title: "Rantai Lake" },
};

/**
 * App-wide root layout.
 *
 * Wires up the global providers in this exact order (outer → inner):
 * `ThemeProvider` (light/dark via next-themes, from the Rantai Design System) →
 * `TooltipProvider` (shadcn tooltips) → `AuthProvider` (session + route
 * guard) → `TableProviders` (nuqs URL state + React Query cache for the
 * Advanced Data Table) → `SidebarProvider` (collapsible sidebar
 * context). Inside, every page gets the `AppSidebar` on the left and the
 * `AppNavbar` on top, with the routed page rendered inside the
 * `<div className="flex-1 p-4">{children}</div>` container.
 *
 * Fonts (Geist + Geist Mono) and semantic color tokens come from the shared
 * design system so the app matches the Rantai Lake visual identity.
 *
 * Note: This is a server component — keep the body free of client-only state.
 * Theme class (`dark` / light) is applied by next-themes on `<html>`.
 */
export default async function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  // `SEC-47`: Next applies the request's nonce to its own scripts on its own;
  // `next-themes` renders one inline script of its own (the theme class before
  // first paint) and only a nonce passed here lets the browser run it. Without
  // it the page paints light and only switches after hydration.
  const nonce = (await headers()).get(NONCE_HEADER) ?? undefined;
  return (
    <html
      lang="en"
      suppressHydrationWarning
      className={`${geist.variable} ${geistMono.variable} ${dashboardFontVariables} font-sans antialiased`}
    >
      <head>
        <meta name="theme-color" content="#050A30" />
        {/* The favicon follows the browser colour scheme: light = navy logo, dark = white logo. */}
        <link rel="icon" type="image/png" href="/icon-light-32.png" media="(prefers-color-scheme: light)" />
        <link rel="icon" type="image/png" href="/icon-dark-32.png" media="(prefers-color-scheme: dark)" />
        <link rel="apple-touch-icon" href="/apple-touch-icon.png" />
      </head>
      {/* Browser extensions (Grammarly, ColorZilla, password managers) put
          their own attributes on <body> before React hydrates; in dev that
          showed as a permanent "1 Issue". This only covers <body>'s own
          attributes, never a mismatch inside it. */}
      <body suppressHydrationWarning>
        <ThemeProvider nonce={nonce}>
          <TooltipProvider>
            <AuthProvider>
              <TableProviders>
                <AppFrame>{children}</AppFrame>
              </TableProviders>
            </AuthProvider>
            {/* Umpan balik aksi mutasi (simpan/hapus/test) — dipanggil lewat
                `notify` di `@/lib/notify`, bukan `toast()` langsung, supaya
                pesan error ServiceError diterjemahkan seragam. */}
            <Toaster position="bottom-right" richColors closeButton />
            {/* Dev-only: hover an element + ⌘C to copy its source location
                for a coding agent. Compiles away in production builds. */}
            <ReactGrabDev />
          </TooltipProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
