"use client";

import * as React from "react";
import { usePathname } from "next/navigation";
import { SidebarProvider, SidebarInset } from "@/components/ui/sidebar";
import { AppSidebar } from "@/components/app-shell/app-sidebar";
import { AppNavbar } from "@/components/app-shell/app-navbar";
import { CopilotDock, dockHiddenOn } from "@/features/copilot/copilot-dock";
import { cn } from "@/lib/utils";
import { CopilotSidebar } from "@/features/copilot/copilot-sidebar";
import { CopilotProvider } from "@/features/copilot/use-copilot";
import { CommandPalette } from "@/components/command-palette";
import { isPublicPath, useAuth } from "@/features/auth/auth-provider";
import { SsoNotice } from "@/features/auth/sso-notice";
import { VersionBanner } from "@/features/version/version-banner";
import { AppShellSkeleton } from "@/components/app-shell/app-shell-skeleton";

import { RouteGuard } from "@/components/app-shell/route-guard";

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
export function AppFrame({ children }: Readonly<{ children: React.ReactNode }>) {
  const pathname = usePathname() ?? "/";
  if (isPublicPath(pathname)) return <>{children}</>;

  return (
    <SidebarProvider>
      <AuthenticatedShell>{children}</AuthenticatedShell>
    </SidebarProvider>
  );
}

function AuthenticatedShell({ children }: Readonly<{ children: React.ReactNode }>) {
  const { status, hasPermission } = useAuth();
  const pathname = usePathname();

  if (status !== "authenticated") {
    // `status === "unauthenticated"` still renders this (rather than
    // `null`) for one tick while `AuthProvider`'s redirect effect fires —
    // a frame-shaped skeleton is a better transient state than a blank
    // page, and it never lingers since the redirect is synchronous with
    // the status flip.
    return <AppShellSkeleton pathname={pathname ?? "/"} />;
  }

  const hasAssistant =
    hasPermission("assistant:use") || hasPermission("assistant:ask");

  if (hasAssistant) {
    return (
      <CopilotProvider>
        <AuthenticatedFrame hasAssistant={true}>{children}</AuthenticatedFrame>
      </CopilotProvider>
    );
  }

  return <AuthenticatedFrame hasAssistant={false}>{children}</AuthenticatedFrame>;
}

function AuthenticatedFrame({
  children,
  hasAssistant,
}: Readonly<{ children: React.ReactNode; hasAssistant: boolean }>) {
  const pathname = usePathname();

  return (
    <>
      <AppSidebar />
      {/*
        Owner QA 2026-11-14, composer (DESIGN.md §3, "Elemen melayang diletakkan di tengah area
        konten"): two variables every page and the dock share.
        - `--content-left`: where the content area starts, past the sidebar (none on a phone, the
          icon rail when it is collapsed); the floating composer centres on what is right of it.
        - `--content-bottom`: the room kept under the content. With the dock it is the composer's
          own height (46 px) and its 16 px offset plus a 14 px gap, so a page ends just above the
          composer; without it, the frame's usual 1.5rem.
      */}
      <SidebarInset
        className={cn(
          "min-w-0 bg-muted/25",
          "md:[--content-left:var(--sidebar-width)] md:peer-data-[collapsible=icon]:[--content-left:var(--sidebar-width-icon)] md:peer-data-[collapsible=offcanvas]:[--content-left:0px]",
          hasAssistant && !dockHiddenOn(pathname) ? "[--content-bottom:4.75rem]" : "[--content-bottom:1.5rem]",
        )}
      >
        <AppNavbar />
        <div className="flex flex-1 min-h-0 min-w-0">
          {/* The Copilot dock is `fixed bottom-4`, so without this the
              last thing on every page sits underneath it. Pages used to
              add their own bottom spacing (or forget to). Where the dock
              is hidden the space is dropped: /copilot sizes itself to the
              viewport, and the extra padding made the window scroll.
              Owner QA 2026-11-14, composer: the room is `--content-bottom`,
              just the composer and a small gap (it was 7rem here plus a
              5rem spacer from the dock, about 12rem of empty page). */}
          <main className="flex-1 min-w-0 p-4 pb-(--content-bottom) sm:p-5 sm:pb-(--content-bottom) lg:p-6 lg:pb-(--content-bottom)">
            {/* `SEC-39`: the SSO callback redirects into the console with
                a notice code in the query when the sign-in succeeded with
                a caveat (a group overage on Entra ID). Mounted once here
                so every landing page shows it; `useSearchParams` needs the
                boundary even though this whole tree is a client
                component. */}
            <React.Suspense fallback={null}>
              <SsoNotice />
            </React.Suspense>
            {/* REL-2 S1-T5: the console and the server it talks to may
                not be the same build (an update that half-landed). The
                banner names both versions; it renders nothing when they
                match. */}
            <VersionBanner />
            <RouteGuard>{children}</RouteGuard>
          </main>
          {hasAssistant && <CopilotSidebar />}
        </div>
        {hasAssistant && <CopilotDock />}
      </SidebarInset>
      <CommandPalette />
    </>
  );
}
