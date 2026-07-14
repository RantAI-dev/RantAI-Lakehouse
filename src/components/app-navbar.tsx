"use client"

import { Search, Bell } from "lucide-react"
import { SidebarTrigger } from "@/components/ui/sidebar"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { cn } from "@/lib/utils"

/**
 * Navbar for the Rantai Lake identity (Organisms / NavBars)
 * - Left: Sidebar toggle + Search input
 * - Right: Notification bell
 */
export function AppNavbar() {
  return (
    <header
      className={cn(
        "flex h-14 shrink-0 items-center gap-4 border-b border-border bg-sidebar px-4 py-3"
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
              "size-8 shrink-0 rounded-md border-border bg-background"
            )}
          />
          <div className="relative max-w-[372px] flex-1">
            <Search
              className="pointer-events-none absolute left-3 top-1/2 size-4 shrink-0 -translate-y-1/2 text-primary"
              aria-hidden
            />
            <Input
              type="search"
              placeholder="Search..."
              className={cn(
                "h-8 w-full rounded-md border-border bg-background pl-9 pr-3 py-2 text-sm leading-5 placeholder:text-muted-foreground",
                "font-[family-name:var(--font-montserrat)]"
              )}
              aria-label="Search"
            />
          </div>
        </div>

        {/* Right: Notifications */}
        <div className="flex shrink-0 items-center gap-0">
          <Button
            variant="ghost"
            size="icon"
            className="size-9 rounded-md text-primary hover:bg-sidebar-accent hover:text-primary dark:hover:bg-sidebar-accent dark:hover:text-sidebar-primary"
            aria-label="Notifications"
          >
            <Bell className="size-4" />
          </Button>
        </div>
      </div>
    </header>
  )
}
