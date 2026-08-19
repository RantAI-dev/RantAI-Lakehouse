"use client"

import Link from "next/link"
import { Search, Bell } from "lucide-react"
import { usePathname } from "next/navigation"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ThemeToggle } from "@/components/theme-toggle"
import { cn } from "@/lib/utils"
import { pageTitleFor, visibleNavGroups, activeNavHref } from "./nav-config"
import { openCommandPalette } from "@/components/command-palette"

/**
 * Sticky top navbar rendered on every page.
 * Left: sidebar toggle + current page title (derived from nav config) + search.
 * Right: theme toggle + notifications entry point.
 */
export function AppNavbar() {
  const pathname = usePathname()
  const pageTitle = pageTitleFor(pathname)
  const activeHref = activeNavHref(pathname)
  const activeGroup = visibleNavGroups().find((g) => g.items.some((it) => it.href === activeHref))
  // Sub-nav muncul saat section aktif punya >1 halaman.
  const subItems = activeGroup && activeGroup.items.length > 1 ? activeGroup.items : []

  return (
    <div className="sticky top-0 z-20 border-b border-border bg-background/90 backdrop-blur-md">
    <header className={cn("flex h-16 shrink-0 items-center gap-4 px-4 sm:px-5")}>
      <div className="flex min-h-0 min-w-0 flex-1 items-center justify-between gap-3">
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <SidebarTrigger
            variant="outline"
            size="icon"
            className="size-9 shrink-0 rounded-lg border-border bg-background shadow-xs"
          />
          <div className="hidden min-w-0 sm:block">
            <p className="truncate text-sm font-semibold text-foreground">
              {pageTitle}
            </p>
            <p className="truncate text-xs text-muted-foreground">
              Rantai Lake workspace
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
              placeholder="Cari halaman, aksi… (⌘K)"
              className="h-9 w-full cursor-pointer rounded-lg border-border bg-muted/40 py-2 pl-9 pr-14 text-sm leading-5 shadow-none placeholder:text-muted-foreground focus-visible:bg-background"
              aria-label="Buka command palette"
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
        </div>
      </div>
    </header>

    {subItems.length ? (
      <nav
        aria-label={`${activeGroup?.label} sub-navigasi`}
        className="flex items-center gap-1 overflow-x-auto border-t border-border px-4 sm:px-5"
      >
        {subItems.map((it) => {
          const active = it.href === activeHref
          const Icon = it.icon
          return (
            <Link
              key={it.href}
              href={it.href}
              className={cn(
                "flex shrink-0 items-center gap-1.5 border-b-2 border-transparent px-2.5 py-2 text-sm text-muted-foreground transition-colors hover:text-foreground",
                active && "border-primary font-medium text-foreground"
              )}
            >
              <Icon className="size-4 shrink-0" />
              {it.title}
            </Link>
          )
        })}
      </nav>
    ) : null}
    </div>
  )
}
