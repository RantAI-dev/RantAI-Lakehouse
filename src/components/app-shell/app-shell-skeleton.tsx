"use client";

import { Bell, ChevronDown } from "lucide-react";
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarInset,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubItem,
  SidebarTrigger,
  SIDEBAR_FADE,
  useSidebar,
} from "@/components/ui/sidebar";
import { Skeleton } from "@/components/ui/skeleton";
import { useSidebarGroups } from "@/hooks/use-sidebar-groups";
import { cn } from "@/lib/utils";
import { SidebarBrand } from "./app-sidebar";
import {
  ACCOUNT_CARD_CLASS,
  NAV_ROW_CLASS,
  SIDEBAR_CLASS,
  SIDEBAR_CONTENT_CLASS,
  SIDEBAR_FOOTER_CLASS,
  SIDEBAR_GROUP_CLASS,
  SIDEBAR_MENU_CLASS,
  SIDEBAR_SUB_CLASS,
} from "./sidebar-classes";
import { NAVBAR_CLASS, NAVBAR_GRID_CLASS, NAVBAR_TRIGGER_CLASS, NavbarSearch, NavbarTitle } from "./app-navbar";
import { NAV_GROUPS, activeNavHref, navbarTitleFor } from "./nav-config";
import { SidebarChatsSkeleton } from "./chat-list-skeleton";
import { sidebarPlaceholderFor } from "./shell-placeholder";
import { ShellMainPlaceholder } from "./shell-main-placeholder";

/**
 * What the console shows while `AuthProvider` is still checking the session.
 *
 * Owner QA 2026-11-14 ("kenapa kita punya dua skeleton page?"): from a refresh
 * to the data there is one loading state. So this is not a drawing of the
 * shell but the shell itself, built from the same parts:
 * - the real `Sidebar` and `SidebarInset` inside the app's one
 *   `SidebarProvider` (`app-frame.tsx`), so the width, the collapsed state and
 *   the phone sheet are the real ones and carry over when the shell mounts;
 * - everything that does not depend on who is signed in is drawn for real:
 *   the logo and product name (`SidebarBrand`), the sidebar toggle, the page
 *   title (from the nav config), the ⌘K search and the notifications bell;
 * - only what does depend on the person is grey: which sections the sidebar
 *   lists (permissions), the account card's name and email, and the Copilot
 *   button. The sidebar rows follow the nav config's sections and the open
 *   state the person keeps (`useSidebarGroups`), with bars as long as the
 *   labels, so a full sidebar lands on rows already in place;
 * - on a conversation (`/copilot`) the sidebar draws the chat list's shape
 *   instead of the menu's (`SidebarChatsSkeleton`, `WEB-18`), as the real
 *   sidebar lists the chats there;
 * - the main area draws the page's own placeholder (`shellPlaceholderFor`),
 *   the same component the page shows while its data loads, so the handoff
 *   to the page is real content replacing grey shapes, never a second
 *   placeholder.
 */
export function AppShellSkeleton({ pathname }: { pathname: string }) {
  return (
    <>
      <Sidebar collapsible="icon" side="left" className={SIDEBAR_CLASS}>
        <SidebarBrand />
        <SidebarContent className={SIDEBAR_CONTENT_CLASS}>
          {/* WEB-18 T5: on a conversation the sidebar is the chat list, so the shell draws that. */}
          {sidebarPlaceholderFor(pathname) === "chats" ? (
            <SidebarChatsSkeleton />
          ) : (
            <SidebarGroup className={SIDEBAR_GROUP_CLASS}>
              <SkeletonNav pathname={pathname} />
            </SidebarGroup>
          )}
        </SidebarContent>
        <SidebarFooter className={SIDEBAR_FOOTER_CLASS} aria-hidden>
          <SidebarMenu>
            <SidebarMenuItem>
              {/* The account row's box: an avatar circle and one name bar, as tall as the real row. */}
              <SidebarMenuButton render={<div />} className={cn(ACCOUNT_CARD_CLASS, "hover:bg-transparent active:bg-transparent")}>
                <Skeleton className="size-6 shrink-0 rounded-full" />
                <div className={cn("min-w-0 flex-1", SIDEBAR_FADE)}>
                  <Skeleton className="h-3.5 w-24" />
                </div>
                <ChevronDown className={cn("ml-auto size-4 shrink-0 text-sidebar-foreground/60", SIDEBAR_FADE)} aria-hidden />
              </SidebarMenuButton>
            </SidebarMenuItem>
          </SidebarMenu>
        </SidebarFooter>
      </Sidebar>
      <SidebarInset className="min-w-0 bg-muted/25">
        <header className={NAVBAR_CLASS}>
          <div className={NAVBAR_GRID_CLASS}>
            <div className="flex min-w-0 items-center gap-2">
              <SidebarTrigger variant="ghost" size="icon" className={NAVBAR_TRIGGER_CLASS} />
              <NavbarTitle title={navbarTitleFor(pathname)} />
            </div>
            <NavbarSearch />
            <div className="flex min-w-0 items-center justify-end gap-1" aria-hidden>
              <span className="grid size-8 place-items-center rounded-lg text-muted-foreground">
                <Bell className="size-4" />
              </span>
              {/* The AI Copilot's button, last, is there only for a holder of the assistant
                  permission: its box, labelled from `lg` as the real one is. */}
              <Skeleton className="h-8 w-8 rounded-lg lg:w-[6.75rem]" />
            </div>
          </div>
        </header>
        <div className="flex flex-1 min-h-0 min-w-0">
          <div className="flex-1 min-w-0 p-4 sm:p-5 lg:p-6" aria-busy="true">
            <ShellMainPlaceholder pathname={pathname} />
          </div>
        </div>
      </SidebarInset>
    </>
  );
}

/** The width of a label's bar: about 7 px a letter, as `text-sm` sets them. */
function barWidth(label: string): string {
  return `${Math.round(label.length * 7)}px`;
}

/**
 * The sidebar's rows as grey bars, one per section of the nav config, with
 * the open sections' pages under them: the rows of a person who may open
 * everything. Someone who may open less gets fewer rows when the shell
 * mounts; the rows they keep are where they were.
 */
function SkeletonNav({ pathname }: { pathname: string }) {
  const activeHref = activeNavHref(pathname);
  const activeLabel = NAV_GROUPS.find((g) => g.items.some((it) => it.href === activeHref))?.label;
  const defaultOpenLabels = NAV_GROUPS.filter((g) => g.defaultOpen).map((g) => g.label);
  const { isOpen } = useSidebarGroups({ activeLabel, defaultOpenLabels });
  // Collapsed, a section's pages are folded away, as in the real sidebar.
  const collapsed = useSidebar().state === "collapsed";
  // The real rows' box: left-aligned on the icon column in both states, 32 px wide when collapsed.
  const row = "flex items-center gap-2 group-data-[collapsible=icon]:w-8";
  return (
    <SidebarMenu className={SIDEBAR_MENU_CLASS} aria-hidden>
      {NAV_GROUPS.map((group) => {
        const single = group.items.length === 1;
        const label = single ? group.items[0].title : group.label;
        return (
          <SidebarMenuItem key={group.label}>
            <div className={cn(NAV_ROW_CLASS, row)}>
              <Skeleton className="size-4 shrink-0 rounded" />
              <Skeleton className={cn("h-3", SIDEBAR_FADE)} style={{ width: barWidth(label) }} />
            </div>
            {single && group.items[0].action ? (
              // The split row's right part: a separator and a 32 px square, as the real row draws them.
              <div className="absolute inset-y-0 right-0 flex group-data-[collapsible=icon]:hidden">
                <span className="my-2 w-px bg-sidebar-border" />
                <span className="grid size-8 place-items-center">
                  <Skeleton className="size-3.5 rounded" />
                </span>
              </div>
            ) : null}
            {!single && !collapsed && isOpen(group.label) ? (
              <SidebarMenuSub className={SIDEBAR_SUB_CLASS}>
                {group.items.map((item) => (
                  <SidebarMenuSubItem key={item.href}>
                    <div className="flex items-center rounded-md px-2.5 py-1.5">
                      <span className="flex h-5 items-center">
                        <Skeleton className="h-3" style={{ width: barWidth(item.title) }} />
                      </span>
                    </div>
                  </SidebarMenuSubItem>
                ))}
              </SidebarMenuSub>
            ) : null}
          </SidebarMenuItem>
        );
      })}
    </SidebarMenu>
  );
}
