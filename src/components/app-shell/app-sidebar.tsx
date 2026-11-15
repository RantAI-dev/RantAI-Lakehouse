"use client"

import * as React from "react"
import { createPortal } from "react-dom"
import Image from "next/image"
import Link from "next/link"
import { usePathname, useRouter } from "next/navigation"
import { ChevronDown, ChevronRight, FileText } from "lucide-react"
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
import { useDashboardsRow } from "@/features/dashboards/dashboards-row"
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
// One compact row (Claude's footer): a 24 px avatar, the name and role, a chevron, 32 px tall.
// Collapsed, it is the avatar alone, centred in the 32 px icon column by 4 px of padding: the faded name and chevron
// still take room, so centring with justify-center would push the avatar out of the button's overflow-hidden box.
export const ACCOUNT_CARD_CLASS = "h-8 gap-2 rounded-lg px-1.5 group-data-[collapsible=icon]:size-8! group-data-[collapsible=icon]:justify-start group-data-[collapsible=icon]:p-1!"
export const ACCOUNT_AVATAR_CLASS =
  "flex size-6 shrink-0 items-center justify-center rounded-full bg-primary/12 text-[10px] font-semibold text-primary"

/** Up to two initials of a display name, for the avatar (the app has no pictures for people). */
export function initialsOf(name: string | undefined): string {
  const words = (name ?? "").trim().split(/\s+/).filter(Boolean)
  if (words.length === 0) return "?"
  const letters = words.length === 1 ? [...words[0]].slice(0, 2) : [[...words[0]][0], [...words[words.length - 1]][0]]
  return letters.join("").toUpperCase()
}
export const NAV_ROW_CLASS = "h-8 rounded-md px-2 py-1.5 text-sm tracking-normal"
export const SIDEBAR_SUB_CLASS = "mr-0 gap-0.5 border-sidebar-border pr-0 group-data-[collapsible=icon]:flex"

function BrandLogo() {
  // Logo follows the theme: navy for a light sidebar, white for a dark one.
  return (
    // A 32 px slot (the rail's icon column) holds the 24 px mark, centred in it.
    <div className="grid size-8 shrink-0 place-items-center" aria-hidden>
      {/* Explicit width too: a grid item centred in its slot would otherwise size to its (absolutely placed) image, i.e. 0, and the mark would not show. */}
      <span className="relative block h-6 w-[28.17px] shrink-0">
        <Image src="/logo-light.png" alt="RantAI Lakehouse" fill sizes="28px" className="object-contain dark:hidden" priority />
        <Image src="/logo-dark.png" alt="" fill sizes="28px" className="hidden object-contain dark:block" priority />
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
  const dashboardsRow = useDashboardsRow()
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
    // more descriptive of the two ("History" over "History").
    // Only the Dashboards row is named after what it opens: the last dashboard or report
    // opened (its own memory, cleared by Browse). Nothing remembered keeps "Dashboards".
    const remembered = first.href === "/dashboards" ? dashboardsRow : null
    const label = remembered?.name ?? (single ? first.title : group.label)
    const labelHref = remembered
      ? remembered.kind === "report"
        ? `/dashboards/reports/${encodeURIComponent(remembered.id)}`
        : `/dashboards/${encodeURIComponent(remembered.id)}`
      : first.href === "/dashboards"
        ? (first.action?.href ?? first.href)
        : first.href
    // The icon follows the kind. BI-61 T11 review: a dashboard keeps the row's own chart icon
    // (the Browse part beside it already uses the grid); a report gets the document icon.
    const RowIcon = remembered?.kind === "report" ? FileText : Icon
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
            const actionActive = !!action && (pathname === action.href || pathname.startsWith(`${action.href}/`))
            const labelActive = active && !actionActive
            return (
              <>
                <SidebarMenuButton
                  isActive={labelActive}
                  tooltip={label}
                  aria-label={remembered ? `${remembered.kind === "report" ? "Report" : "Dashboard"}: ${remembered.name}` : undefined}
                  className={cn(
                    menuBtnClass(labelActive),
                    action && "min-w-0 flex-1 rounded-r-none group-data-[collapsible=icon]:flex-none group-data-[collapsible=icon]:rounded-md",
                  )}
                  render={
                    <Link href={labelHref} onClick={(e) => handleNavClick(e, labelHref)}>
                      <RowIcon className="size-4 shrink-0" />
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
                          <Link
                            href={action.href}
                            aria-label={action.title}
                            aria-current={actionActive ? "page" : undefined}
                            onClick={(e) => handleNavClick(e, action.href)}
                            className={cn(
                              "grid size-8 shrink-0 place-items-center rounded-r-md outline-hidden ring-sidebar-ring transition-colors duration-150 focus-visible:ring-2 group-data-[collapsible=icon]:hidden [&>svg]:size-3.5",
                              actionActive
                                ? "bg-sidebar-accent text-sidebar-accent-foreground"
                                : "text-sidebar-foreground/70 hover:bg-sidebar-accent/60 hover:text-sidebar-accent-foreground",
                            )}
                          />
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
