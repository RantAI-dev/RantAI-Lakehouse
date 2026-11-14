"use client"

import * as React from "react"
import { createPortal } from "react-dom"
import Image from "next/image"
import Link from "next/link"
import { usePathname, useRouter } from "next/navigation"
import { CircleUserRound, ChevronRight, ChevronsUpDown } from "lucide-react"
import { useAuth } from "@/features/auth/auth-provider"
import { rememberTableHref } from "@/hooks/use-table-memory"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuAction,
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
import { useSidebarGroups } from "@/hooks/use-sidebar-groups"
import { cn } from "@/lib/utils"
import { NAV_GROUPS, activeNavHref, canOpen, type NavGroup, type NavItem } from "./nav-config"
import { UserMenu } from "./user-menu"

/*
 * The sidebar's boxes, shared with the loading shell (`AppShellSkeleton`),
 * which draws the same frame with grey rows while the session is checked:
 * one set of classes, so the skeleton cannot drift from the real sidebar.
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

function BrandLogo() {
  // Logo follows the theme: navy for a light sidebar, white for a dark one.
  return (
    <div className="relative aspect-[196/167] h-8 shrink-0" aria-hidden>
      <Image src="/logo-light.svg" alt="RantAI Lakehouse" fill unoptimized sizes="32px" className="object-contain dark:hidden" priority />
      <Image src="/logo-dark.svg" alt="" fill unoptimized sizes="32px" className="hidden object-contain dark:block" priority />
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
          <span className="text-sm font-semibold tracking-[-0.084px] text-sidebar-foreground">Rantai Lake</span>
          <span className="text-[11px] font-normal tracking-[-0.04px] text-sidebar-foreground/60">Enterprise Lakehouse Console</span>
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

  const menuBtnClass = (active: boolean) =>
    cn(
      NAV_ROW_CLASS,
      active
        ? "bg-sidebar-accent font-medium text-sidebar-accent-foreground"
        : "font-normal text-sidebar-foreground/70 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
    )

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
          "flex items-center gap-2 rounded-md px-2 py-1.5 text-sm transition-colors duration-150",
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
    // more descriptive of the two ("History" over "History").
    const label = single ? first.title : group.label
    const active = group.items.some((it) => it.href === activeHref)

    // One page, or icon mode with one page: nothing to expand.
    if (single) {
      return (
        <SidebarMenuItem key={group.label}>
          <SidebarMenuButton
            isActive={active}
            tooltip={label}
            className={menuBtnClass(active)}
            render={
              <Link href={first.href} onClick={(e) => handleNavClick(e, first.href)}>
                <Icon className="size-4 shrink-0" />
                <span className={cn("truncate leading-5", SIDEBAR_FADE)}>{label}</span>
              </Link>
            }
          />
          {/* A second destination at the row's right end, where a section's
              chevron sits. `SidebarMenuAction` hides itself in icon mode. */}
          {first.action ? (
            // The app's tooltips open at once: the root `TooltipProvider`
            // has delay 0, and in base-ui a provider's zero wins over a
            // `delay` set on the trigger. This one sits on a navigation row
            // the pointer crosses all the time, so it gets its own provider
            // and waits for the pointer to rest.
            <TooltipProvider delay={500}>
            <Tooltip>
              <TooltipTrigger
                render={
                  <SidebarMenuAction
                    aria-label={first.action.title}
                    // Sits where a section's chevron sits: same size
                    // (3.5) and the same 18px from the row's right edge
                    // to its centre, measured against the Data chevron.
                    // Fades with the label instead of vanishing as the collapse starts.
                    className={cn("right-2 text-sidebar-foreground/50 [&>svg]:size-3.5 group-data-[collapsible=icon]:flex", SIDEBAR_FADE)}
                    render={
                      <Link
                        href={first.action.href}
                        onClick={(e) => handleNavClick(e, first.action?.href ?? first.href)}
                      />
                    }
                  />
                }
              >
                <first.action.icon />
              </TooltipTrigger>
              <TooltipContent side="right">{first.action.title}</TooltipContent>
            </Tooltip>
            </TooltipProvider>
          ) : null}
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
                  "size-3.5 shrink-0 text-sidebar-foreground/50 transition-[transform,opacity] duration-150",
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

  return (
    <Sidebar data-print-hide collapsible="icon" side="left" className={SIDEBAR_CLASS}>
      <SidebarBrand />

      <SidebarContent className={SIDEBAR_CONTENT_CLASS}>
        <SidebarGroup className={SIDEBAR_GROUP_CLASS}>
          <SidebarMenu className={SIDEBAR_MENU_CLASS}>
            {groups.map(renderEntry)}
          </SidebarMenu>
        </SidebarGroup>

        {/* A dashboard list and Copilot history used to live here. Both
            only ever appeared once you were already on their page, so
            they could never be used to NAVIGATE there — that was page
            content occupying navigation space, and it vanished the moment
            the sidebar collapsed to icons. Both now live in their own
            page's header instead (`BoardSwitcher`, `CopilotHistoryMenu`),
            and the sidebar purely reads `nav-config` with no per-route
            exception. */}
        {/* The active section's pages used to be repeated down here, in a
            separate block. They now sit inside their own group above, so
            the section that is open is the one you are reading. */}
      </SidebarContent>

      {/* Flyout (icon mode only) */}
      {mounted && flyout && openGroup
        ? createPortal(
            <div
              ref={flyoutRef}
              style={{ position: "fixed", top: Math.min(flyout.top, window.innerHeight - 40 - openGroup.items.length * 34), left: flyout.left }}
              className="z-[60] w-56 rounded-xl border border-sidebar-border bg-sidebar p-1.5 shadow-2xl"
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
              onLogout={logout}
              trigger={
                <SidebarMenuButton
                  size="lg"
                  tooltip={user?.name ?? "Signed in"}
                  aria-label={`Account menu, ${user?.name ?? "signed in"}`}
                  className={cn(ACCOUNT_CARD_CLASS, "hover:bg-sidebar-accent data-[popup-open]:bg-sidebar-accent")}
                />
              }
            >
              <div className={ACCOUNT_AVATAR_CLASS}>
                <CircleUserRound className="size-5" aria-hidden />
              </div>
              <div className={cn("grid min-w-0 flex-1 whitespace-nowrap text-left leading-tight", SIDEBAR_FADE)}>
                <span className="truncate text-sm font-semibold text-sidebar-foreground">{user?.name ?? "Signed in"}</span>
                <span className="truncate text-xs text-sidebar-foreground/60">{user?.email ?? ""}</span>
              </div>
              <ChevronsUpDown className={cn("ml-auto size-4 shrink-0 text-sidebar-foreground/60", SIDEBAR_FADE)} aria-hidden />
            </UserMenu>
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}
