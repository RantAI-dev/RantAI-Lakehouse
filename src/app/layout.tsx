import type { Metadata } from "next";
import { geist, geistMono } from "@rantai/design-system/fonts/fonts";
import { ThemeProvider } from "@rantai/design-system/components/theme-provider";
import { Toaster } from "@rantai/design-system/ui/sonner";
import { TooltipProvider } from "@/components/ui/tooltip";
import { AppFrame } from "@/components/app-shell/app-frame";
import { TableProviders } from "@/components/app-shell/table-providers";
import { AuthProvider } from "@/features/auth/auth-provider";
import { ReactGrabDev } from "@/components/app-shell/react-grab-dev";
import "./globals.css";

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
export default function RootLayout({
  children,
}: Readonly<{
  children: React.ReactNode;
}>) {
  return (
    <html
      lang="en"
      suppressHydrationWarning
      className={`${geist.variable} ${geistMono.variable} font-sans antialiased`}
    >
      <head>
        <meta name="theme-color" content="#050A30" />
        {/* Favicon ikut skema warna browser: terang=logo navy, gelap=logo putih. */}
        <link rel="icon" type="image/png" href="/icon-light-32.png" media="(prefers-color-scheme: light)" />
        <link rel="icon" type="image/png" href="/icon-dark-32.png" media="(prefers-color-scheme: dark)" />
        <link rel="apple-touch-icon" href="/apple-touch-icon.png" />
      </head>
      <body>
        <ThemeProvider>
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
