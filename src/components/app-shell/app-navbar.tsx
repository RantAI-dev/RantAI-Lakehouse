"use client"

import * as React from "react"
import Link from "next/link"
import { Search, Bell, Sparkles } from "lucide-react"
import { usePathname, useRouter } from "next/navigation"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { cn } from "@/lib/utils"
import { formatRelativeTime } from "@/lib/format"
import { useMentionsRead } from "@/lib/mention-events"
import { pageTitleFor } from "./nav-config"
import { openCommandPalette } from "@/components/command-palette"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { notificationsService } from "@/services"
import { TenantSwitcher } from "./tenant-switcher"
import { useCopilot } from "@/features/copilot/use-copilot"

/*
 * The navbar's boxes, shared with the loading shell (`AppShellSkeleton`),
 * which draws the same bar while the session is checked: one set of classes,
 * so the skeleton cannot drift from the real navbar.
 */
// 48 px tall (owner 2026-11-14, "navbar ini terlalu tinggi"): its 32 px controls, the console's
// height, with 8 px above and below. The sidebar's logo row (`SidebarBrand`) is the same 48 px, so
// their bottom borders make one straight line. Every offset under the navbar reads 3rem from here.
export const NAVBAR_CLASS =
  "sticky top-0 z-20 flex h-12 shrink-0 items-center gap-4 border-b border-border bg-background/90 px-4 backdrop-blur-md sm:px-5"
export const NAVBAR_GRID_CLASS =
  "grid min-h-0 min-w-0 flex-1 grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-3"
export const NAVBAR_TRIGGER_CLASS = "size-8 shrink-0 rounded-lg border-border bg-background shadow-xs"

/** The page's title at the navbar's left, from the nav config (never from the session). */
export function NavbarTitle({ title }: { readonly title: string }) {
  return (
    <div className="hidden min-w-0 sm:block">
      <p className="truncate text-sm font-semibold text-foreground">{title}</p>
    </div>
  )
}

/**
 * The search box in the middle of the navbar. It holds nothing about the
 * person signed in, so the loading shell draws it too; a click there before
 * the palette is mounted does nothing.
 * About 400 px, narrower on a small laptop; hidden on a phone as before.
 */
export function NavbarSearch() {
  return (
    <div className="relative hidden w-72 md:block lg:w-[25rem]">
      <Search
        className="pointer-events-none absolute left-3 top-1/2 size-4 shrink-0 -translate-y-1/2 text-muted-foreground"
        aria-hidden
      />
      <Input
        type="search"
        readOnly
        onClick={() => openCommandPalette()}
        onFocus={() => openCommandPalette()}
        placeholder="Search pages, tables, columns"
        className="h-8 w-full cursor-pointer rounded-lg border-border bg-muted/40 py-2 pl-9 pr-14 text-sm leading-5 shadow-none placeholder:text-muted-foreground focus-visible:bg-background"
        aria-label="Open command palette"
      />
      <kbd className="pointer-events-none absolute right-2 top-1/2 hidden h-5 -translate-y-1/2 items-center rounded border border-border bg-background px-1.5 font-mono text-[10px] text-muted-foreground lg:flex">
        ⌘ K
      </kbd>
    </div>
  )
}

/**
 * Sticky top navbar rendered on every page.
 * Left: sidebar toggle + current page title (derived from nav config) + search.
 * Right: the Copilot, notifications. The account menu is the sidebar's footer
 * card (BI-61 review round 1, SHOULD-FIX 7).
 */
export function AppNavbar() {
  const pathname = usePathname()
  const pageTitle = pageTitleFor(pathname)
  const router = useRouter()
  const { user, activeTenantId, setActiveTenant, hasPermission } = useAuth()
  const copilot = useCopilot()
  const hasAssistant = hasPermission("assistant:use") || hasPermission("assistant:ask")
  const isCopilotPage = pathname?.startsWith("/copilot")
  // Bell dot lights only for a real, currently open/pending item — never
  // on `supported: false` (an unsupported deployment shows no dot, not a
  // stuck-on or stuck-off guess) and never from an invented "unread"
  // count (no per-principal read cursor exists).
  const notifications = useService((signal) => notificationsService.list(signal), [])
  // BI-38 D6: opening a dashboard's comments marks its mentions read and says
  // so here, so the dot and the list reflect it without a page reload.
  useMentionsRead(notifications.reload)
  const hasNotifications =
    notifications.status === "success" &&
    notifications.data.supported &&
    (notifications.data.openAlerts.length > 0 ||
      notifications.data.pendingApprovals.length > 0 ||
      notifications.data.unreadMentions.length > 0)
  // BI-19 part E: unread @mentions in dashboard comments, each a real line the
  // reader can open. Empty when none, or when the deployment cannot say.
  const mentions =
    notifications.status === "success" && notifications.data.supported
      ? notifications.data.unreadMentions
      : []
  const sortedMentions = [...mentions].sort((a, b) => Date.parse(b.at) - Date.parse(a.at))
  // BI-19 E review E-6b: when the mentions could not be read, say so; an empty
  // bell would read as "no mentions".
  const mentionsNote =
    notifications.status === "success" && notifications.data.supported
      ? notifications.data.unreadMentionsUnavailable
      : null

  return (
    <header
      data-print-hide
      className={NAVBAR_CLASS}
    >
      {/* BI-61 review round 1, SHOULD-FIX 8 (owner decision 2026-11-14): three columns, so the
          search sits in the middle of the navbar's own width (the content right of the sidebar),
          whether the sidebar is expanded or collapsed and the side panel open or not. */}
      <div className={NAVBAR_GRID_CLASS}>
        <div className="flex min-w-0 items-center gap-3">
          <SidebarTrigger
            variant="outline"
            size="icon"
            className={NAVBAR_TRIGGER_CLASS}
          />
          {/* Page title only. There used to be a second line reading
              "Rantai Lake workspace" — fixed text that never changed, so
              it conveyed nothing while repeating the brand already shown
              in the sidebar header right next to it. That slot is
              normally used for a breadcrumb or the active workspace
              name, so a static label there would promise a
              multi-workspace feature that does not exist yet
              (`workspaceName` is only used in Settings). */}
          <NavbarTitle title={pageTitle} />
        </div>
        <NavbarSearch />

        <div className="flex min-w-0 items-center justify-end gap-1">
          <TenantSwitcher
            tenants={user?.tenants ?? []}
            activeTenantId={activeTenantId}
            onSwitch={(id) => {
              // Persist the new choice (`apiFetch` reads it on the next
              // request, tenant-scope enforcement applies it server-side),
              // then force a server refetch — every tenant-scoped list
              // route reads `X-Tenant` at request time, so a client-side
              // re-filter would be dishonest (it would still show data
              // fetched under the OLD tenant's scope until the next real
              // request).
              setActiveTenant(id)
              router.refresh()
            }}
          />
          {!isCopilotPage && hasAssistant && (
            <Button
              variant="ghost"
              size="icon"
              onClick={() => copilot.setExpanded((prev) => !prev)}
              className={cn(
                "relative size-8 rounded-lg transition-colors",
                copilot.expanded
                  ? "bg-primary/10 text-primary hover:bg-primary/20 hover:text-primary"
                  : "text-muted-foreground hover:bg-muted hover:text-foreground"
              )}
              aria-label={copilot.expanded ? "Collapse AI Copilot" : "Open AI Copilot"}
              title={copilot.expanded ? "Collapse AI Copilot" : "Open AI Copilot"}
            >
              <Sparkles className="size-4" />
            </Button>
          )}
          {mentions.length > 0 || mentionsNote ? (
            <DropdownMenu>
              <DropdownMenuTrigger
                render={
                  <Button
                    variant="ghost"
                    size="icon"
                    className="relative size-8 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
                    aria-label="Notifications"
                  />
                }
              >
                <Bell className="size-4" />
                {hasNotifications ? (
                  <span className="absolute right-2 top-2 size-1.5 rounded-full bg-primary ring-2 ring-background" />
                ) : null}
              </DropdownMenuTrigger>
              <DropdownMenuContent align="end" className="w-80">
                {/* BI-19 owner QA 2026-10-28, QA-2: a small notification panel, not a bare menu. */}
                <div className="flex items-center justify-between gap-2 px-2 py-1">
                  <span className="text-sm font-medium">Notifications</span>
                  {mentions.length > 0 ? (
                    <Button
                      variant="ghost"
                      size="xs"
                      className="text-muted-foreground"
                      onClick={() => {
                        void notificationsService.markMentionsRead().then(() => notifications.reload()).catch(() => undefined)
                      }}
                    >
                      Mark all as read
                    </Button>
                  ) : null}
                </div>
                <DropdownMenuSeparator />
                {sortedMentions.map((m) => (
                  // QA-3: `?comments=1` opens the Comments dialog on the board.
                  <DropdownMenuItem
                    key={m.id}
                    className="items-start gap-2"
                    render={<Link href={`/dashboards/${encodeURIComponent(m.board)}?comments=1`} />}
                  >
                    <span aria-hidden className="mt-1.5 size-1.5 shrink-0 rounded-full bg-primary" />
                    <span className="flex min-w-0 flex-col">
                      <span className="text-sm">{m.title}</span>
                      <span className="text-xs text-muted-foreground">{formatRelativeTime(m.at)}</span>
                    </span>
                  </DropdownMenuItem>
                ))}
                {mentionsNote ? (
                  <p role="status" className="px-2 py-1.5 text-xs text-muted-foreground">
                    {mentionsNote}
                  </p>
                ) : null}
                <DropdownMenuSeparator />
                <DropdownMenuItem render={<Link href="/alerts" />}>Alerts and approvals</DropdownMenuItem>
              </DropdownMenuContent>
            </DropdownMenu>
          ) : (
            <Button
              variant="ghost"
              size="icon"
              className="relative size-8 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
              aria-label="Notifications"
              render={
                <Link href="/alerts">
                  <Bell className="size-4" />
                  {hasNotifications ? (
                    <span className="absolute right-2 top-2 size-1.5 rounded-full bg-primary ring-2 ring-background" />
                  ) : null}
                </Link>
              }
            />
          )}
        </div>
      </div>
    </header>
  )
}
