"use client"

import * as React from "react"
import { createPortal } from "react-dom"
import Image from "next/image"
import Link from "next/link"
import { usePathname, useRouter } from "next/navigation"
import { ChevronDown, ChevronRight } from "lucide-react"
import { useAuth } from "@/features/auth/auth-provider"
import { rememberTableHref } from "@/hooks/use-table-memory"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarMenuSub,
  SidebarMenuSubItem,
  SidebarRail,
  SIDEBAR_FADE,
  SIDEBAR_MOTION,
  useSidebar,
} from "@/components/ui/sidebar"
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip"
import { useLastBoardId, useLastBoardName } from "@/features/dashboards/last-board"
import { useSidebarGroups } from "@/hooks/use-sidebar-groups"
import { cn } from "@/lib/utils"
import { NAV_GROUPS, activeNavHref, canOpen, type NavGroup, type NavItem } from "./nav-config"
import {
  ACCOUNT_AVATAR_CLASS,
  ACCOUNT_CARD_CLASS,
  SIDEBAR_CLASS,
  SIDEBAR_CONTENT_CLASS,
  SIDEBAR_FOOTER_CLASS,
  SIDEBAR_GROUP_CLASS,
  SIDEBAR_MENU_CLASS,
  SIDEBAR_SUB_CLASS,
  SIDEBAR_VIEW_PANE_CLASS,
  initialsOf,
  navRowClass,
} from "./sidebar-classes"
import { SidebarChats } from "./sidebar-chats"
import { UserMenu } from "./user-menu"

/** The conversation page, where the sidebar lists the chats instead of the menu (`WEB-18`). */
export const CONVERSATION_PATH = "/copilot"

function BrandLogo() {
  // Logo follows the theme: navy for a light sidebar, white for a dark one.
  return (
    // A 32 px slot (the rail's icon column) holds the 24 px mark, centred in it.
    <div className="grid size-8 shrink-0 place-items-center" aria-hidden>
      {/* Explicit width too: a grid item centred in its slot would otherwise size to its (absolutely placed) image, i.e. 0, and the mark would not show. */}
      <span className="relative block h-6 w-[28.17px] shrink-0">
        <Image src="/logo-light.svg" alt="RantAI Lakehouse" fill unoptimized sizes="28px" className="object-contain dark:hidden" priority />
        <Image src="/logo-dark.svg" alt="" fill unoptimized sizes="28px" className="hidden object-contain dark:block" priority />
      </span>
    </div>
  )
}

/**
 * The sidebar's header: the logo and the product's name. Nothing in it
 * depends on who is signed in, so the loading shell (`AppShellSkeleton`)
 * draws this same component while the session is checked, and the header
 * does not change at all when the real sidebar mounts.
 */
export function SidebarBrand() {
  // `--app-navbar-h` supaya persis setinggi navbar (`app-navbar.tsx` membaca variabel yang sama),
  // sehingga garis bawah keduanya menyambung jadi satu garis lurus di seluruh
  // lebar layar. Sebelumnya tinggi header ini dihitung dari isinya (padding +
  // baris logo) dan jatuh di 69px — 5px lebih rendah dari navbar, dan
  // patahannya terlihat tepat di pertemuan sidebar dengan konten.
  return (
    // The logo sits 8 px in, centred on the 48 px rail, in both states; the name fades.
    <SidebarHeader className="flex h-(--app-navbar-h) shrink-0 flex-col justify-center border-b border-sidebar-border px-2 py-0">
      <div className="flex min-w-0 items-center justify-start gap-2">
        <BrandLogo />
        <div className={cn("grid min-w-0 flex-1 gap-0.5 whitespace-nowrap leading-none", SIDEBAR_FADE)}>
          <span className="truncate text-sm font-semibold tracking-[-0.084px] text-sidebar-foreground">RantAI Lakehouse</span>
        </div>
      </div>
    </SidebarHeader>
  )
}

type FlyoutState = { label: string; top: number; left: number } | null

/**
 * Primary sidebar — a list of SECTIONs, two modes:
 *
 *  · EXPANDED: clicking a section header OPENS/CLOSES that section in
 *    place; its sub-pages nest below it. Several sections may be open at
 *    once, the choice is remembered (`useSidebarGroups`), and a section
 *    containing the active page is always open.
 *  · COLLAPSED (icons): clicking a section icon opens a FLYOUT of
 *    sub-pages to the right, since there is no room to show them in place.
 *
 * A 1-item group always links directly — nothing to open.
 */
export function AppSidebar() {
  const router = useRouter()
  const pathname = usePathname()
  const activeHref = activeNavHref(pathname)
  const { state } = useSidebar()
  const iconMode = state === "collapsed"
  const lastBoardName = useLastBoardName()
  const lastBoardId = useLastBoardId()
  const { user, hasPermission, logout } = useAuth()
  const groups = React.useMemo(() => {
    return NAV_GROUPS.map((g) => ({
      ...g,
      items: g.items.filter((it) => canOpen(it.href, hasPermission)),
    })).filter((g) => g.items.length > 0)
  }, [hasPermission])
  const activeGroup = groups.find((g) => g.items.some((it) => it.href === activeHref))
  const defaultOpenLabels = React.useMemo(
    () => groups.filter((g) => g.defaultOpen).map((g) => g.label),
    [groups]
  )
  const { isOpen: isGroupOpen, toggle: toggleGroup } = useSidebarGroups({
    activeLabel: activeGroup?.label,
    defaultOpenLabels,
  })
  // WEB-18: on a conversation the sidebar lists the chats instead of the menu. "Back to menu"
  // brings the menu back for this visit only: any navigation (another page, the browser's Back)
  // resets it, so the next conversation opens on its chats again.
  const hasAssistant = hasPermission("assistant:use") || hasPermission("assistant:ask")
  const onConversation = pathname === CONVERSATION_PATH && hasAssistant
  // Owner QA 2026-11-14: Home's split row opens the list on any page, so the view is the route's
  // default (the list on a conversation, the menu elsewhere) unless the person chose one for this
  // visit; any pathname change drops the choice.
  const [chatsChoice, setChatsChoice] = React.useState<boolean | null>(null)
  React.useEffect(() => setChatsChoice(null), [pathname])
  const showChats = hasAssistant && (chatsChoice ?? onConversation)

  // The slide between the two views. The chat view mounts the first time it is wanted, off to the
  // right, and only then moves in, so the browser has a place to move it from; after that it stays
  // mounted, out of view and inert, and the two views just trade places.
  const [chatsMounted, setChatsMounted] = React.useState(showChats)
  const [chatsIn, setChatsIn] = React.useState(showChats)
  const chatsPaneRef = React.useRef<HTMLDivElement>(null)
  const contentRef = React.useRef<HTMLDivElement>(null)
  // The view that gets the focus once it is in: the control that leads back to the other one.
  const focusOnArrival = React.useRef<"chats" | "menu" | null>(null)
  React.useEffect(() => {
    if (showChats && !chatsMounted) {
      setChatsMounted(true)
      return
    }
    if (showChats === chatsIn) return
    // Lay the pane out at its starting place before it moves (a fresh pane has no "from" yet).
    void chatsPaneRef.current?.offsetWidth
    setChatsIn(showChats)
  }, [showChats, chatsMounted, chatsIn])
  React.useEffect(() => {
    const target = focusOnArrival.current
    if (!target) return
    focusOnArrival.current = null
    contentRef.current?.querySelector<HTMLElement>(`[data-view-focus="${target}"]`)?.focus({ preventScroll: true })
  }, [chatsIn])
  const backToMenu = () => {
    focusOnArrival.current = "menu"
    setChatsChoice(false)
  }
  const openChats = () => {
    focusOnArrival.current = "chats"
    setChatsChoice(true)
  }
  const [flyout, setFlyout] = React.useState<FlyoutState>(null)
  const flyoutRef = React.useRef<HTMLDivElement>(null)
  const [mounted, setMounted] = React.useState(false)
  React.useEffect(() => setMounted(true), [])

  const handleNavClick = React.useCallback(
    (e: React.MouseEvent<HTMLAnchorElement>, href: string) => {
      setFlyout(null)
      const target = rememberTableHref(href)
      if (target !== href) {
        e.preventDefault()
        router.push(target)
      }
    },
    [router]
  )
  React.useEffect(() => setFlyout(null), [pathname])
  React.useEffect(() => { if (!iconMode) setFlyout(null) }, [iconMode])

  React.useEffect(() => {
    if (!flyout) return
    const onDown = (e: MouseEvent) => {
      const t = e.target as Node
      if (flyoutRef.current?.contains(t)) return
      if ((t as HTMLElement)?.closest?.("[data-section-trigger]")) return
      setFlyout(null)
    }
    const onKey = (e: KeyboardEvent) => { if (e.key === "Escape") setFlyout(null) }
    document.addEventListener("mousedown", onDown)
    document.addEventListener("keydown", onKey)
    return () => {
      document.removeEventListener("mousedown", onDown)
      document.removeEventListener("keydown", onKey)
    }
  }, [flyout])

  const openFlyout = (label: string, el: HTMLElement) => {
    if (flyout?.label === label) return setFlyout(null)
    const r = el.getBoundingClientRect()
    setFlyout({ label, top: r.top, left: r.right + 8 })
  }

  const menuBtnClass = navRowClass

  /**
   * Satu baris halaman.
   *
   * `showIcon` mati di sub-menu: ikon section sudah ada di header tepat di
   * atasnya, dan `SidebarMenuSub` menandai kedalaman dengan garis kiri —
   * ikon per item hanya mengulang keduanya. Flyout mode ikon tetap
   * memakainya, karena panel itu melayang lepas dari sidebar dan headernya
   * cuma label teks kecil, jadi tanpa ikon isinya kehilangan jangkar.
   */
  const linkRow = (item: NavItem, active: boolean, showIcon = true) => {
    const Icon = item.icon
    return (
      <Link
        href={item.href}
        onClick={(e) => handleNavClick(e, item.href)}
        className={cn(
          "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors duration-150 ease-ui",
          active
            ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground"
            : "text-sidebar-foreground/70 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
        )}
      >
        {showIcon ? <Icon className="size-4 shrink-0" /> : null}
        <span className="truncate">{item.title}</span>
      </Link>
    )
  }

  const renderEntry = (group: NavGroup) => {
    const single = group.items.length === 1
    const first = group.items[0]
    const Icon = group.icon ?? first.icon
    // A group declared with one page shows that page's title, which is the
    // more descriptive of the two.
    // Only the Dashboards row is named after what it opens: `/dashboards` goes to the board
    // last opened, so the row says which one. Nothing remembered keeps "Dashboards".
    const boardName = first.href === "/dashboards" ? lastBoardName : null
    const label = boardName ?? (single ? first.title : group.label)
    // The row goes straight to the board it names; with nothing remembered (or after Browse)
    // it goes to the list, so it never depends on the `/dashboards` resolver's guess.
    const labelHref = first.href === "/dashboards" ? (lastBoardId ? `/dashboards/${encodeURIComponent(lastBoardId)}` : first.action?.href ?? first.href) : first.href
    const active = group.items.some((it) => it.href === activeHref)

    // One page, or icon mode with one page: nothing to expand.
    if (single) {
      return (
        <SidebarMenuItem key={group.label} className={first.action ? "flex items-stretch" : undefined}>
          {(() => {
            // A split row (shadcn's button group): the label part and, at the right end, a square
            // part that is the whole touch target for a second destination, one thin separator
            // between them. Each part lights up on its own; the active route lights only its own part.
            const action = first.action
            const href = action?.href
            const actionActive = !!action && (href ? pathname === href || pathname.startsWith(`${href}/`) : onConversation)
            const labelActive = active && !actionActive
            const actionClass = cn(
              "grid size-8 shrink-0 place-items-center rounded-r-md outline-hidden ring-sidebar-ring transition-colors duration-150 focus-visible:ring-2 group-data-[collapsible=icon]:hidden [&>svg]:size-3.5",
              actionActive
                ? "bg-sidebar-accent text-sidebar-accent-foreground"
                : "text-sidebar-foreground/70 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
            )
            return (
              <>
                <SidebarMenuButton
                  isActive={labelActive}
                  tooltip={label}
                  aria-label={boardName ? `Dashboard: ${boardName}` : undefined}
                  className={cn(
                    menuBtnClass(labelActive),
                    action && "min-w-0 flex-1 rounded-r-none group-data-[collapsible=icon]:flex-none group-data-[collapsible=icon]:rounded-md",
                  )}
                  render={
                    <Link href={labelHref} onClick={(e) => handleNavClick(e, labelHref)}>
                      <Icon className="size-4 shrink-0" />
                      <span className={cn("truncate leading-5", SIDEBAR_FADE)}>{label}</span>
                    </Link>
                  }
                />
                {action ? (
                  // The app's tooltips open at once: the root `TooltipProvider` has delay 0, and in
                  // base-ui a provider's zero wins over a `delay` set on the trigger. This sits on
                  // a navigation row the pointer crosses all the time, so it gets its own provider
                  // and waits for the pointer to rest. Hidden with the rail's collapse: Browse stays
                  // reachable from the page itself.
                  <TooltipProvider delay={500}>
                    <span aria-hidden className="my-2 w-px shrink-0 bg-sidebar-border group-data-[collapsible=icon]:hidden" />
                    <Tooltip>
                      <TooltipTrigger
                        render={
                          href ? (
                            <Link
                              href={href}
                              aria-label={action.title}
                              aria-current={actionActive ? "page" : undefined}
                              onClick={(e) => handleNavClick(e, href)}
                              className={actionClass}
                            />
                          ) : (
                            <button
                              type="button"
                              aria-label={action.title}
                              aria-current={actionActive ? "page" : undefined}
                              data-view-focus="menu"
                              onClick={openChats}
                              className={actionClass}
                            />
                          )
                        }
                      >
                        <action.icon />
                      </TooltipTrigger>
                      <TooltipContent side="right">{action.title}</TooltipContent>
                    </Tooltip>
                  </TooltipProvider>
                ) : null}
              </>
            )
          })()}
        </SidebarMenuItem>
      )
    }

    // Several pages. Expanded, the header opens and closes the section in
    // place instead of navigating (it used to link to the first page, so
    // looking at what a section held cost a page load). Collapsed, the same
    // row opens a FLYOUT of its pages, as there is no room to show them. One
    // element in both states, so the row does not swap as the sidebar moves:
    // its box narrows, its label fades and its pages fold away with it.
    const expanded = !iconMode && isGroupOpen(group.label)
    const flyoutOpen = iconMode && flyout?.label === group.label
    const lit = iconMode ? active || flyoutOpen : active && !expanded
    return (
      <SidebarMenuItem key={group.label}>
        <SidebarMenuButton
          isActive={lit}
          className={menuBtnClass(lit)}
          render={
            <button
              type="button"
              data-section-trigger
              title={iconMode ? label : undefined}
              aria-expanded={iconMode ? flyoutOpen : expanded}
              onClick={(e) => (iconMode ? openFlyout(group.label, e.currentTarget) : toggleGroup(group.label))}
            >
              <Icon className="size-4 shrink-0" />
              <span className={cn("flex-1 truncate text-left leading-5", SIDEBAR_FADE)}>
                {label}
              </span>
              <ChevronRight
                className={cn(
                  // Micro-interaction audit 2026-11-14: `rotate-90` sets `rotate`, not `transform`, in
                  // Tailwind v4, so the chevron used to snap; it now turns with the section's fold.
                  "size-3.5 shrink-0 text-sidebar-foreground/50 transition-[rotate,opacity] duration-200 ease-ui",
                  expanded && "rotate-90",
                  "group-data-[collapsible=icon]:opacity-0",
                )}
                aria-hidden
              />
            </button>
          }
        />
        {/* The pages fold (0fr) and unfold (1fr) in the sidebar's time; folded they are inert,
            so they take no focus. */}
        <div
          className={cn("grid transition-[grid-template-rows]", SIDEBAR_MOTION, expanded ? "grid-rows-[1fr]" : "grid-rows-[0fr]")}
          inert={!expanded}
        >
          <div className="min-h-0 overflow-hidden">
            <SidebarMenuSub className={SIDEBAR_SUB_CLASS}>
              {group.items.map((item) => (
                <SidebarMenuSubItem key={item.href}>
                  {linkRow(item, item.href === activeHref, false)}
                </SidebarMenuSubItem>
              ))}
            </SidebarMenuSub>
          </div>
        </div>
      </SidebarMenuItem>
    )
  }

  const openGroup = groups.find((g) => g.label === flyout?.label)

  const menuEntries = groups.map(renderEntry)

  return (
    <Sidebar data-print-hide collapsible="icon" side="left" className={SIDEBAR_CLASS}>
      <SidebarBrand />

      <SidebarContent ref={contentRef} className={cn(SIDEBAR_CONTENT_CLASS, "relative overflow-hidden")}>
        {/* WEB-18 (owner 2026-11-14): on a conversation the chat list takes the menu's place,
            as in a chat app, so the history is beside the chat it belongs to instead of on a
            page of its own. The account card and the logo row stay. */}
        {/* The two views sit side by side and slide together (`SIDEBAR_VIEW_PANE_CLASS`): the
            menu leaves to the left as the chats come in from the right, so the sidebar is never
            empty mid-move, and its width does not change. The one out of view is inert. */}
        <div
          className={cn(
            SIDEBAR_VIEW_PANE_CLASS,
            "no-scrollbar overflow-y-auto group-data-[collapsible=icon]:overflow-hidden",
            chatsIn && "-translate-x-full",
          )}
          inert={chatsIn}
          data-sidebar-view="menu"
        >
          <SidebarGroup className={SIDEBAR_GROUP_CLASS}>
            <SidebarMenu className={SIDEBAR_MENU_CLASS}>
              {menuEntries}
            </SidebarMenu>
          </SidebarGroup>
        </div>
        {chatsMounted ? (
          <div
            ref={chatsPaneRef}
            className={cn(SIDEBAR_VIEW_PANE_CLASS, !chatsIn && "translate-x-full")}
            inert={!chatsIn}
            data-sidebar-view="chats"
          >
            <SidebarChats shown={chatsIn} onBack={backToMenu} />
          </div>
        ) : null}
      </SidebarContent>

      {/* Flyout (icon mode only) */}
      {mounted && flyout && openGroup
        ? createPortal(
            <div
              ref={flyoutRef}
              style={{ position: "fixed", top: Math.min(flyout.top, window.innerHeight - 40 - openGroup.items.length * 34), left: flyout.left }}
              // Micro-interaction audit 2026-11-14: the flyout grows out of the rail it belongs to, like the menus.
              className="z-[60] w-56 origin-left rounded-xl border border-sidebar-border bg-sidebar p-1.5 shadow-2xl transition-[opacity,scale] duration-150 ease-ui starting:scale-[0.97] starting:opacity-0"
            >
              <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-sidebar-foreground/60">{openGroup.label}</p>
              {openGroup.items.map((item) => (
                <React.Fragment key={item.href}>{linkRow(item, item.href === activeHref)}</React.Fragment>
              ))}
            </div>,
            document.body,
          )
        : null}

      <SidebarFooter className={SIDEBAR_FOOTER_CLASS}>
        <SidebarMenu>
          <SidebarMenuItem>
            {/* BI-61 review round 1, SHOULD-FIX 7 (owner decision 2026-11-14): the card is the
                account menu's one trigger. It looked clickable and did nothing; the navbar's
                profile button that held the menu is gone. Collapsed, the avatar alone opens it. */}
            <UserMenu
              userName={user?.name}
              userEmail={user?.email}
              roles={user?.roles}
              onLogout={logout}
              side={state === "collapsed" ? "right" : "top"}
              align={state === "collapsed" ? "end" : "start"}
              trigger={
                <SidebarMenuButton
                  tooltip={user?.name ?? "Signed in"}
                  aria-label={`Account menu, ${user?.name ?? "signed in"}`}
                  className={cn(ACCOUNT_CARD_CLASS, "hover:bg-sidebar-accent data-[popup-open]:bg-sidebar-accent")}
                />
              }
            >
              <div className={ACCOUNT_AVATAR_CLASS} aria-hidden>{initialsOf(user?.name)}</div>
              {/* Avatar, name, chevron; the email and role are in the menu's header. */}
              <span className={cn("min-w-0 flex-1 truncate whitespace-nowrap text-left text-sm font-medium text-sidebar-foreground", SIDEBAR_FADE)}>
                {user?.name ?? "Signed in"}
              </span>
              <ChevronDown className={cn("ml-auto size-4 shrink-0 text-sidebar-foreground/60", SIDEBAR_FADE)} aria-hidden />
            </UserMenu>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}
