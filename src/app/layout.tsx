import type { Metadata } from "next";
import { geist, geistMono } from "@rantai/design-system/fonts/fonts";
import { ThemeProvider } from "@rantai/design-system/components/theme-provider";
import { TooltipProvider } from "@/components/ui/tooltip";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { AppSidebar } from "@/components/app-shell/app-sidebar";
import { AppNavbar } from "@/components/app-shell/app-navbar";
import { CopilotDock } from "@/features/copilot/copilot-dock";
import { CopilotProvider } from "@/features/copilot/use-copilot";
import { CommandPalette } from "@/components/command-palette";
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
 * `TooltipProvider` (shadcn tooltips) → `SidebarProvider` (collapsible sidebar
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
            <SidebarProvider>
              <CopilotProvider>
                <AppSidebar />
                <SidebarInset className="min-w-0 bg-muted/25">
                  <AppNavbar />
                  <div className="flex-1 p-4 sm:p-5 lg:p-6">{children}</div>
                  <CopilotDock />
                </SidebarInset>
                <CommandPalette />
              </CopilotProvider>
            </SidebarProvider>
          </TooltipProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
