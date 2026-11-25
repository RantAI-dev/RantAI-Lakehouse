"use client"

import { ChevronLeft, Plus, Search } from "lucide-react"

import { SIDEBAR_FADE } from "@/components/ui/sidebar"
import { Skeleton } from "@/components/ui/skeleton"
import { cn } from "@/lib/utils"
import { NAV_ROW_CLASS, SIDEBAR_GROUP_CLASS, SIDEBAR_MENU_CLASS } from "./sidebar-classes"

/** Title bars' widths, in the order the rows take them: varied, like real titles, and fixed so the skeleton never shifts. */
const SKELETON_TITLES = ["78%", "62%", "85%", "54%", "70%", "66%", "80%", "58%"]

/**
 * The chat list while it loads: a day's label and title rows, the shape the
 * list lands in. The loading shell draws the same rows on `/copilot`
 * (`AppShellSkeleton`), so a refresh shows one loading look, not two.
 */
export function ChatListSkeleton({ rows = SKELETON_TITLES.length }: { readonly rows?: number }) {
  return (
    <div aria-hidden className="pt-1">
      <div className="flex h-5 items-center px-2 pb-1">
        <Skeleton className="h-2.5 w-12" />
      </div>
      <div className={SIDEBAR_MENU_CLASS}>
        {SKELETON_TITLES.slice(0, rows).map((w, i) => (
          <div key={i} className={cn(NAV_ROW_CLASS, "flex items-center")}>
            <Skeleton className="h-3" style={{ width: w }} />
          </div>
        ))}
      </div>
    </div>
  )
}

/**
 * The sidebar's chat view while the session is checked, on a conversation
 * (`AppShellSkeleton`, `WEB-18` T5): the same rows in the same boxes as
 * `SidebarChats`, so the shell lands on rows already in place. Back to menu,
 * New chat and Search do not depend on who is signed in and are drawn for
 * real (not interactive yet); AI settings does (it is for whoever may change
 * them), so it is a grey row, and the chats are grey titles.
 */
export function SidebarChatsSkeleton() {
  // The real rows' box: left-aligned on the icon column in both states, 32 px wide when collapsed.
  const row = cn(NAV_ROW_CLASS, "flex items-center gap-2 text-sidebar-foreground/70 group-data-[collapsible=icon]:w-8")
  const label = cn("truncate leading-5", SIDEBAR_FADE)
  return (
    <div aria-hidden className="flex min-h-0 flex-1 flex-col" data-sidebar-view="chats">
      <div className={cn("flex w-full min-w-0 shrink-0 flex-col", SIDEBAR_GROUP_CLASS)}>
        <div className={SIDEBAR_MENU_CLASS}>
          <div className={row}>
            <ChevronLeft className="size-4 shrink-0" />
            <span className={label}>Back to menu</span>
          </div>
          <div className={row}>
            <Plus className="size-4 shrink-0" />
            <span className={label}>New chat</span>
          </div>
          <div className={row}>
            <Search className="size-4 shrink-0" />
            <span className={label}>Search</span>
          </div>
          <div className={row}>
            <Skeleton className="size-4 shrink-0 rounded" />
            <Skeleton className={cn("h-3 w-20", SIDEBAR_FADE)} />
          </div>
        </div>
      </div>
      <div className={cn("min-h-0 flex-1 overflow-hidden px-2 pb-2", SIDEBAR_FADE)}>
        <ChatListSkeleton />
      </div>
    </div>
  )
}
