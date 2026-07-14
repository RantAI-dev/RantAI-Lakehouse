import type { Metadata } from "next";
import { geist, geistMono } from "@rantai/design-system/fonts/fonts";
import { ThemeProvider } from "@rantai/design-system/components/theme-provider";
import { TooltipProvider } from "@/components/ui/tooltip";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { AppSidebar } from "@/components/app-sidebar";
import { AppNavbar } from "@/components/app-navbar";
import "./globals.css";

export const metadata: Metadata = {
  title: "Rantai Lake",
  description: "Rantai Lake - Data platform",
};

/**
 * App-wide root layout.
 *
 * Wires up the global providers in this exact order (outer → inner):
 * `ThemeProvider` (forced dark, from the Rantai Design System) →
 * `TooltipProvider` (shadcn tooltips) → `SidebarProvider` (collapsible sidebar
 * context). Inside, every page gets the `AppSidebar` on the left and the
 * `AppNavbar` on top, with the routed page rendered inside the
 * `<div className="flex-1 p-4">{children}</div>` container.
 *
 * Fonts (Geist + Geist Mono) and the dark theme come from the shared design
 * system so the app matches the Rantai Lake visual identity.
 *
 * Note: This is a server component — keep the body free of client-only state.
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
      className={`dark ${geist.variable} ${geistMono.variable} font-sans antialiased`}
    >
      <head>
        <meta name="theme-color" content="#050A30" />
      </head>
      <body>
        <ThemeProvider>
          <TooltipProvider>
            <SidebarProvider>
              <AppSidebar />
              <SidebarInset>
                <AppNavbar />
                <div className="flex-1 p-4">{children}</div>
              </SidebarInset>
            </SidebarProvider>
          </TooltipProvider>
        </ThemeProvider>
      </body>
    </html>
  );
}
