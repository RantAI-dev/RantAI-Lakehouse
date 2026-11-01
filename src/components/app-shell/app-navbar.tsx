"use client"

import * as React from "react"
import Link from "next/link"
import { Search, Bell, LogOut, KeyRound, CircleUserRound, Sparkles, Sun, Moon, Monitor } from "lucide-react"
import { usePathname, useRouter } from "next/navigation"
import { useTheme } from "next-themes"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
  DropdownMenuSub,
  DropdownMenuSubTrigger,
  DropdownMenuSubContent,
  DropdownMenuRadioGroup,
  DropdownMenuRadioItem,
} from "@/components/ui/dropdown-menu"
import { cn } from "@/lib/utils"
import { formatRelativeTime } from "@/lib/format"
import { pageTitleFor } from "./nav-config"
import { openCommandPalette } from "@/components/command-palette"
import { useAuth } from "@/features/auth/auth-provider"
import { useService } from "@/hooks/use-service"
import { notificationsService } from "@/services"
import { TenantSwitcher } from "./tenant-switcher"
import { useCopilot } from "@/features/copilot/use-copilot"
import { CONSOLE_VERSION } from "@/features/version/console-version"

/**
 * Sticky top navbar rendered on every page.
 * Left: sidebar toggle + current page title (derived from nav config) + search.
 * Right: theme toggle + notifications entry point.
 */
export function AppNavbar() {
  const pathname = usePathname()
  const pageTitle = pageTitleFor(pathname)
  const router = useRouter()
  const { user, logout, activeTenantId, setActiveTenant } = useAuth()
  const copilot = useCopilot()
  const isCopilotPage = pathname?.startsWith("/copilot")
  // Bell dot lights only for a real, currently open/pending item — never
  // on `supported: false` (an unsupported deployment shows no dot, not a
  // stuck-on or stuck-off guess) and never from an invented "unread"
  // count (no per-principal read cursor exists).
  const notifications = useService((signal) => notificationsService.list(signal), [])
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
      className={cn(
        "sticky top-0 z-20 flex h-16 shrink-0 items-center gap-4 border-b border-border bg-background/90 px-4 backdrop-blur-md sm:px-5"
      )}
    >
      <div className="flex min-h-0 min-w-0 flex-1 items-center justify-between gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <SidebarTrigger
            variant="outline"
            size="icon"
            className="size-9 shrink-0 rounded-lg border-border bg-background shadow-xs"
          />
          {/* Page title only. There used to be a second line reading
              "Rantai Lake workspace" — fixed text that never changed, so
              it conveyed nothing while repeating the brand already shown
              in the sidebar header right next to it. That slot is
              normally used for a breadcrumb or the active workspace
              name, so a static label there would promise a
              multi-workspace feature that does not exist yet
              (`workspaceName` is only used in Settings, and Tenants is
              still preview). */}
          <div className="hidden min-w-0 sm:block">
            <p className="truncate text-sm font-semibold text-foreground">
              {pageTitle}
            </p>
          </div>
          <div className="relative ml-auto hidden w-full max-w-90 md:block">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 size-4 shrink-0 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              type="search"
              readOnly
              onClick={() => openCommandPalette()}
              onFocus={() => openCommandPalette()}
              placeholder="Search pages, tables, columns… (⌘K)"
              className="h-9 w-full cursor-pointer rounded-lg border-border bg-muted/40 py-2 pl-9 pr-14 text-sm leading-5 shadow-none placeholder:text-muted-foreground focus-visible:bg-background"
              aria-label="Open command palette"
            />
            <kbd className="pointer-events-none absolute right-2 top-1/2 hidden h-5 -translate-y-1/2 items-center rounded border border-border bg-background px-1.5 font-mono text-[10px] text-muted-foreground lg:flex">
              ⌘ K
            </kbd>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1">
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
          {!isCopilotPage && (
            <Button
              variant="ghost"
              size="icon"
              onClick={() => copilot.setExpanded((prev) => !prev)}
              className={cn(
                "relative size-9 rounded-lg transition-colors",
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
                    className="relative size-9 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
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
              className="relative size-9 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
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
          <UserMenu userName={user?.name} userEmail={user?.email} onLogout={logout} />
        </div>
      </div>
    </header>
  )
}

/** Sun until mounted, so the server render and the first client render agree. */
function ThemeIcon({
  mounted,
  theme,
  resolvedTheme,
}: Readonly<{
  mounted: boolean
  theme: string | undefined
  resolvedTheme: string | undefined
}>) {
  if (mounted && theme === "system") return <Monitor className="size-4" />
  if (mounted && resolvedTheme === "dark") return <Moon className="size-4" />
  return <Sun className="size-4" />
}

/**
 * Who's signed in, with a logout affordance and theme selection. Renders once `AuthProvider` has resolved a user (see `AppFrame`).
 *
 * Exported for its own test: the account menu's version line (REL-2
 * S1-T5) is the one thing here that says which build the console is, and
 * rendering the whole navbar around it would need the auth, copilot and
 * sidebar providers the menu does not use.
 */
export function UserMenu({
  userName,
  userEmail,
  onLogout,
}: Readonly<{
  userName: string | undefined
  userEmail: string | null | undefined
  onLogout: () => void | Promise<void>
}>) {
  const { theme, setTheme, resolvedTheme } = useTheme()
  const [mounted, setMounted] = React.useState(false)

  React.useEffect(() => {
    setMounted(true)
  }, [])

  React.useEffect(() => {
    if (!mounted) return
    const meta = document.querySelector('meta[name="theme-color"]')
    if (meta) {
      meta.setAttribute(
        "content",
        resolvedTheme === "dark" ? "#050A30" : "#ffffff"
      )
    }
  }, [mounted, resolvedTheme])

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button
            variant="ghost"
            size="icon"
            className="size-9 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
            aria-label="Account menu"
          />
        }
      >
        <CircleUserRound className="size-5" />
      </DropdownMenuTrigger>
      <DropdownMenuContent>
        <DropdownMenuLabel>
          <p className="truncate text-sm font-medium text-foreground">{userName ?? "Signed in"}</p>
          {userEmail ? <p className="truncate text-xs font-normal text-muted-foreground">{userEmail}</p> : null}
        </DropdownMenuLabel>
        <DropdownMenuSeparator />
        <DropdownMenuItem render={<Link href="/account/change-password" />}>
          <KeyRound />
          Change password
        </DropdownMenuItem>
        <DropdownMenuSub>
          <DropdownMenuSubTrigger>
            <ThemeIcon mounted={mounted} theme={theme} resolvedTheme={resolvedTheme} />
            <span>Theme</span>
          </DropdownMenuSubTrigger>
          <DropdownMenuSubContent>
            <DropdownMenuRadioGroup value={mounted ? theme : undefined} onValueChange={(val) => setTheme(val)}>
              <DropdownMenuRadioItem value="light">
                <Sun className="size-4" />
                <span>Light</span>
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="dark">
                <Moon className="size-4" />
                <span>Dark</span>
              </DropdownMenuRadioItem>
              <DropdownMenuRadioItem value="system">
                <Monitor className="size-4" />
                <span>System</span>
              </DropdownMenuRadioItem>
            </DropdownMenuRadioGroup>
          </DropdownMenuSubContent>
        </DropdownMenuSub>
        <DropdownMenuSeparator />
        {/* REL-2 S1-T5: the console's own version, so a person can say
            which build they are looking at. Disabled — it is a fact, not
            an action. */}
        <DropdownMenuItem disabled>
          Version {CONSOLE_VERSION}
        </DropdownMenuItem>
        <DropdownMenuItem variant="destructive" onClick={() => void onLogout()}>
          <LogOut />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
