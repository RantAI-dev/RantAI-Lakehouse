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
 * The app frame (sidebar + navbar + AI dock + command palette). For PUBLIC
 * read-only routes (`/public/*`, `/embed/*`, `/login`) this frame is
 * SKIPPED — the page renders plain, with no console chrome, fit for
 * sharing with an outside party (e.g. a manager) or for pages that precede
 * a session (login).
 *
 * For other routes (which need a session), the chrome only renders once
 * `AuthProvider` confirms `status === "authenticated"`. This is NOT a
 * security boundary (the server still returns 401/403 without a valid
 * session cookie) — it merely prevents a "flash" of an empty/broken
 * console before `AuthProvider`'s redirect effect to `/login` has a
 * chance to run.
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
