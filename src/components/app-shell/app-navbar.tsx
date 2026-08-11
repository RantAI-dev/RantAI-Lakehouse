"use client"

import Link from "next/link"
import { Search, Bell } from "lucide-react"
import { usePathname } from "next/navigation"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ThemeToggle } from "@/components/theme-toggle"
import { cn } from "@/lib/utils"
import { pageTitleFor } from "./nav-config"

/**
 * Sticky top navbar rendered on every page.
 * Left: sidebar toggle + current page title (derived from nav config) + search.
 * Right: theme toggle + notifications entry point.
 */
export function AppNavbar() {
  const pathname = usePathname()
  const pageTitle = pageTitleFor(pathname)

  return (
    <header
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
              placeholder="Search assets, pipelines, agents..."
              className="h-9 w-full rounded-lg border-border bg-muted/40 py-2 pl-9 pr-14 text-sm leading-5 shadow-none placeholder:text-muted-foreground focus-visible:bg-background"
              aria-label="Search"
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
  )
}
