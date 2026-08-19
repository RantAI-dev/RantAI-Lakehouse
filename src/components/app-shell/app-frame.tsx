"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { AppSidebar } from "@/components/app-shell/app-sidebar";
import { AppNavbar } from "@/components/app-shell/app-navbar";
import { CopilotDock } from "@/features/copilot/copilot-dock";
import { CopilotProvider } from "@/features/copilot/use-copilot";
import { CommandPalette } from "@/components/command-palette";

/**
 * Kerangka aplikasi (sidebar + navbar + dock AI + command palette). Untuk rute
 * PUBLIK read-only (`/public/*`) kerangka ini DILEWATI — halaman dirender polos
 * tanpa chrome konsol, cocok untuk dibagikan ke pihak luar (mis. atasan).
 */
export function AppFrame({ children }: { children: React.ReactNode }) {
  const pathname = usePathname();
  if (pathname?.startsWith("/public")) return <>{children}</>;

  return (
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
  );
}
