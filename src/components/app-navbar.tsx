"use client"

import { Search, Bell } from "lucide-react"
import { usePathname } from "next/navigation"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ThemeToggle } from "@/components/theme-toggle"
import { cn } from "@/lib/utils"

const pageTitles: Record<string, string> = {
  "/": "Dashboard",
  "/pipelines": "Pipelines",
  "/query-studio": "Query Studio",
  "/semantic-search": "Search & Similarity",
  "/lineage": "Data Lineage",
  "/connectors": "Connectors",
  "/data-catalog": "Data Catalog",
  "/intelligence-knowledge": "Intelligence Knowledge",
  "/user-management": "User Management",
  "/tenant-management": "Tenant Management",
  "/data-governance": "Data Governance",
  "/audit-logs": "Audit Logs",
}

/**
 * Navbar for the Rantai Lake identity (Organisms / NavBars)
 * - Left: Sidebar toggle + Search input
 * - Right: Notification bell
 */
export function AppNavbar() {
  const pathname = usePathname()
  const route = Object.keys(pageTitles)
    .filter((path) => path === "/" || pathname.startsWith(path))
    .sort((a, b) => b.length - a.length)[0]
  const pageTitle = pageTitles[route] ?? "Rantai Lake"

  return (
    <header
      className={cn(
        "sticky top-0 z-20 flex h-16 shrink-0 items-center gap-4 border-b border-border bg-background/90 px-4 backdrop-blur-md sm:px-5"
      )}
      data-name="Organisms / NavBars"
    >
      <div className="flex min-h-0 min-w-0 flex-1 items-center justify-between gap-3">
        {/* Left: Sidebar trigger + Search */}
        <div className="flex min-w-0 flex-1 items-center gap-3">
          <SidebarTrigger
            variant="outline"
            size="icon"
            className={cn(
              "size-9 shrink-0 rounded-lg border-border bg-background shadow-xs"
            )}
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
              placeholder="Search data, pipelines, users..."
              className={cn(
                "h-9 w-full rounded-lg border-border bg-muted/40 py-2 pl-9 pr-14 text-sm leading-5 shadow-none placeholder:text-muted-foreground focus-visible:bg-background",
                "font-[family-name:var(--font-montserrat)]"
              )}
              aria-label="Search"
            />
            <kbd className="pointer-events-none absolute right-2 top-1/2 hidden h-5 -translate-y-1/2 items-center rounded border border-border bg-background px-1.5 font-mono text-[10px] text-muted-foreground lg:flex">
              ⌘ K
            </kbd>
          </div>
        </div>

        {/* Right: Theme toggle + Notifications */}
        <div className="flex shrink-0 items-center gap-1">
          <ThemeToggle />
          <Button
            variant="ghost"
            size="icon"
            className="relative size-9 rounded-lg text-muted-foreground hover:bg-muted hover:text-foreground"
            aria-label="Notifications"
          >
            <Bell className="size-4" />
            <span className="absolute right-2 top-2 size-1.5 rounded-full bg-primary ring-2 ring-background" />
          </Button>
        </div>
      </div>
    </header>
  )
}
