import { SIDEBAR_MOTION } from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"

/*
 * The sidebar's boxes, shared by the main menu (`AppSidebar`), the chat list
 * that replaces it on a conversation (`SidebarChats`, `WEB-18`) and the
 * loading shell (`AppShellSkeleton`), which draws the same frame with grey
 * rows while the session is checked: one set of classes, so neither view nor
 * the skeleton can drift from the others.
 */
export const SIDEBAR_CLASS = "border-r border-sidebar-border bg-sidebar shadow-sm"
//
// Collapsing is one move (owner 2026-11-14, `SIDEBAR_MOTION` in `ui/sidebar.tsx`): the rows keep the
// expanded layout in both states, left-aligned on one icon column (16 px from the edge, the centre
// of the 48 px rail), with the same vertical rhythm, so collapsing only narrows the row's box and
// fades the label; no icon moves and no row jumps.
export const SIDEBAR_CONTENT_CLASS = "min-w-0"
export const SIDEBAR_GROUP_CLASS = "gap-0 px-2 py-2"
export const SIDEBAR_MENU_CLASS = "mt-0 flex flex-col gap-0.5"
export const SIDEBAR_FOOTER_CLASS = "border-t border-sidebar-border p-2"
// Collapsed, the card is the avatar alone, 32 px on the icon column; the box narrows, its padding
// eases to none and the avatar with it, in the sidebar's time.
export const ACCOUNT_CARD_CLASS = "h-14 rounded-lg px-2 group-data-[collapsible=icon]:size-8! group-data-[collapsible=icon]:p-0!"
export const ACCOUNT_AVATAR_CLASS = cn(
  "flex size-9 shrink-0 items-center justify-center rounded-full bg-primary/12 text-primary ring-1 ring-primary/20 transition-[width,height]",
  SIDEBAR_MOTION,
  "group-data-[collapsible=icon]:size-8",
)
export const NAV_ROW_CLASS = "h-8 rounded-md px-2 py-1.5 text-sm tracking-normal"
export const SIDEBAR_SUB_CLASS = "mr-0 gap-0.5 border-sidebar-border pr-0 group-data-[collapsible=icon]:flex"

/** A row's colours: the active page lit, the others quiet until hovered. */
export function navRowClass(active: boolean): string {
  return cn(
    NAV_ROW_CLASS,
    active
      ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground"
      : "font-normal text-sidebar-foreground/70 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
  )
}

/*
 * WEB-18: the two views of the sidebar (the main menu, and the chat list on a
 * conversation) slide sideways inside it, both at once, so there is never an
 * empty frame: the menu leaves to the left as the chats come in from the
 * right, and the reverse for "Back to menu". The sidebar's own curve
 * (`SIDEBAR_MOTION`, a strong ease-out) at 240 ms, a touch longer than the
 * collapse because the distance is the sidebar's whole width. Reduced motion:
 * the views swap at once.
 */
export const SIDEBAR_VIEW_MS = 240
export const SIDEBAR_VIEW_PANE_CLASS =
  "absolute inset-0 flex min-h-0 flex-col transition-transform duration-[240ms] ease-[cubic-bezier(0.23,1,0.32,1)] motion-reduce:transition-none"
