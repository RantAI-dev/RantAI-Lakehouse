"use client"

import Link from "next/link"
import { Search, Bell, LogOut, KeyRound, CircleUserRound } from "lucide-react"
import { usePathname } from "next/navigation"
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
} from "@/components/ui/dropdown-menu"
import { ThemeToggle } from "@/components/theme-toggle"
import { cn } from "@/lib/utils"
import { pageTitleFor } from "./nav-config"
import { openCommandPalette } from "@/components/command-palette"
import { useAuth } from "@/features/auth/auth-provider"

/**
 * Sticky top navbar rendered on every page.
 * Left: sidebar toggle + current page title (derived from nav config) + search.
 * Right: theme toggle + notifications entry point.
 */
export function AppNavbar() {
  const pathname = usePathname()
  const pageTitle = pageTitleFor(pathname)
  const { user, logout } = useAuth()

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
          {/* Judul halaman saja. Dulu ada baris kedua bertuliskan "Rantai
              Lake workspace" — teks tetap yang tidak pernah berubah, jadi
              tidak menyampaikan apa pun, sekaligus mengulang merek yang
              sudah tertera di header sidebar tepat di sebelahnya. Posisi
              itu biasanya dipakai untuk breadcrumb atau nama workspace
              aktif, sehingga label statis di sana justru menjanjikan
              multi-workspace yang belum ada (`workspaceName` baru dipakai
              di Settings, dan Tenants masih preview). */}
          <div className="hidden min-w-0 sm:block">
            <p className="truncate text-sm font-semibold text-foreground">
              {pageTitle}
            </p>
          </div>
          <div className="relative ml-auto hidden w-full max-w-[360px] md:block">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 size-4 shrink-0 -translate-y-1/2 text-muted-foreground"
              aria-hidden
            />
            <Input
              type="search"
              readOnly
              onClick={() => openCommandPalette()}
              onFocus={() => openCommandPalette()}
              placeholder="Search pages, actions… (⌘K)"
              className="h-9 w-full cursor-pointer rounded-lg border-border bg-muted/40 py-2 pl-9 pr-14 text-sm leading-5 shadow-none placeholder:text-muted-foreground focus-visible:bg-background"
              aria-label="Open command palette"
            />
            <kbd className="pointer-events-none absolute right-2 top-1/2 hidden h-5 -translate-y-1/2 items-center rounded border border-border bg-background px-1.5 font-mono text-[10px] text-muted-foreground lg:flex">
              ⌘ K
            </kbd>
          </div>
        </div>

        <div className="flex shrink-0 items-center gap-1">
          <ThemeToggle />
          <Button
            variant="ghost"
            size="icon"
            className="relative size-9 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
            aria-label="Notifications"
            render={
              <Link href="/alerts">
                <Bell className="size-4" />
                <span className="absolute right-2 top-2 size-1.5 rounded-full bg-primary ring-2 ring-background" />
              </Link>
            }
          />
          <UserMenu userName={user?.name} userEmail={user?.email} onLogout={logout} />
        </div>
      </div>
    </header>
  )
}

/** Who's signed in, with a logout affordance. Renders once `AuthProvider` has resolved a user (see `AppFrame`). */
function UserMenu({
  userName,
  userEmail,
  onLogout,
}: {
  userName: string | undefined
  userEmail: string | null | undefined
  onLogout: () => void | Promise<void>
}) {
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
        <DropdownMenuSeparator />
        <DropdownMenuItem variant="destructive" onClick={() => void onLogout()}>
          <LogOut />
          Sign out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  )
}
