"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { AppSidebar } from "@/components/app-shell/app-sidebar";
import { AppNavbar } from "@/components/app-shell/app-navbar";
import { CopilotDock } from "@/features/copilot/copilot-dock";
import { CopilotProvider } from "@/features/copilot/use-copilot";
import { CommandPalette } from "@/components/command-palette";
import { isPublicPath, useAuth } from "@/features/auth/auth-provider";
import { LoadingSkeleton } from "@/components/patterns/page-states";

/**
 * Kerangka aplikasi (sidebar + navbar + dock AI + command palette). Untuk rute
 * PUBLIK read-only (`/public/*`, `/embed/*`, `/login`) kerangka ini DILEWATI —
 * halaman dirender polos tanpa chrome konsol, cocok untuk dibagikan ke pihak
 * luar (mis. atasan) atau untuk halaman yang mendahului sesi (login).
 *
 * Untuk rute lain (yang butuh sesi), chrome hanya dirender setelah
 * `AuthProvider` mengonfirmasi `status === "authenticated"`. Ini BUKAN
 * batas keamanan (server tetap 401/403 tanpa cookie sesi yang valid) —
 * ini semata mencegah "flash" konsol kosong/rusak sebelum
 * `AuthProvider`'s redirect effect ke `/login` sempat jalan.
 */
export function AppFrame({ children }: { children: React.ReactNode }) {
  const pathname = usePathname() ?? "/";
  if (isPublicPath(pathname)) return <>{children}</>;

  return (
    <SidebarProvider>
      <CopilotProvider>
        <AuthenticatedFrame>{children}</AuthenticatedFrame>
      </CopilotProvider>
    </SidebarProvider>
  );
}

function AuthenticatedFrame({ children }: { children: React.ReactNode }) {
  const { status } = useAuth();

  if (status !== "authenticated") {
    // `status === "unauthenticated"` still renders this (rather than
    // `null`) for one tick while `AuthProvider`'s redirect effect fires —
    // a bare loading skeleton is a better transient state than a blank
    // page, and it never lingers since the redirect is synchronous with
    // the status flip.
    return (
      <div className="flex-1 p-4 sm:p-5 lg:p-6">
        <LoadingSkeleton rows={8} />
      </div>
    );
  }

  return (
    <>
      <AppSidebar />
      <SidebarInset className="min-w-0 bg-muted/25">
        <AppNavbar />
        <div className="flex-1 p-4 sm:p-5 lg:p-6">{children}</div>
        <CopilotDock />
      </SidebarInset>
      <CommandPalette />
    </>
  );
}
