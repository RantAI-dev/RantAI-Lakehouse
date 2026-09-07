"use client"

import * as React from "react"
import { createPortal } from "react-dom"
import Image from "next/image"
import Link from "next/link"
import { usePathname } from "next/navigation"
import { CircleUserRound, ChevronRight } from "lucide-react"
import { useAuth } from "@/features/auth/auth-provider"
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
  useSidebar,
} from "@/components/ui/sidebar"
import { useSidebarGroups } from "@/hooks/use-sidebar-groups"
import { cn } from "@/lib/utils"
import { visibleNavGroups, activeNavHref, type NavGroup, type NavItem } from "./nav-config"

function BrandLogo() {
  // Logo ikut tema: navy untuk sidebar terang, putih untuk sidebar gelap.
  return (
    <div className="relative size-8 shrink-0" aria-hidden>
      <Image src="/logo-light.png" alt="Rantai Lake" fill sizes="32px" className="object-contain dark:hidden" priority />
      <Image src="/logo-dark.png" alt="" fill sizes="32px" className="hidden object-contain dark:block" priority />
    </div>
  )
}

type FlyoutState = { label: string; top: number; left: number } | null

/**
 * Primary sidebar — daftar SECTION, dua mode:
 *
 *  · MELEBAR: klik header section = BUKA/TUTUP section itu di tempat;
 *    sub-halamannya bersarang di bawahnya. Beberapa section boleh terbuka
 *    sekaligus, pilihannya diingat (`useSidebarGroups`), dan section yang
 *    memuat halaman aktif selalu terbuka.
 *  · DICIUTKAN (ikon): klik ikon section → FLYOUT sub-halaman ke kanan,
 *    karena tidak ada ruang untuk menampilkannya di tempat.
 *
 * Grup 1-item selalu link langsung — tidak ada yang perlu dibuka.
 *
 * Section yang SELURUH halamannya masih mock tampil disabled dengan badge
 * "Soon" (lihat `comingSoon` di nav-config), bukan disembunyikan.
 */
export function AppSidebar() {
  const pathname = usePathname()
  const activeHref = activeNavHref(pathname)
  const groups = visibleNavGroups()
  const { state } = useSidebar()
  const iconMode = state === "collapsed"
  const { user } = useAuth()
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
      "h-8 rounded-md px-2.5 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
      active && "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border",
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
        onClick={() => setFlyout(null)}
        className={cn(
          "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
          active && "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border",
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
    // more descriptive of the two ("AI Copilot" over "AI"). A group that
    // was REDUCED to one page by the preview filter keeps its own label —
    // otherwise "Administration" silently renames itself to "Settings"
    // just because its other four pages are still mocks.
    const label = single && !group.partiallyHidden ? first.title : group.label
    const active = group.items.some((it) => it.href === activeHref)

    // Every page in this section is still a mock. Shown, but inert — the
    // alternative was the whole section vanishing with no explanation.
    if (group.comingSoon) {
      return (
        <SidebarMenuItem key={group.label}>
          <div
            title={`${group.label} — coming soon`}
            aria-disabled
            className={cn(
              "flex h-8 cursor-default items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-sidebar-foreground/40",
              iconMode && "justify-center px-0"
            )}
          >
            <Icon className="size-4 shrink-0" />
            {!iconMode ? (
              <>
                <span className="flex-1 truncate">{group.label}</span>
                <span className="rounded bg-sidebar-accent px-1.5 py-0.5 text-[10px] font-medium uppercase tracking-wide text-sidebar-foreground/50">
                  Soon
                </span>
              </>
            ) : null}
          </div>
        </SidebarMenuItem>
      )
    }

    // One page, or icon mode with one page: nothing to expand.
    if (single) {
      return (
        <SidebarMenuItem key={group.label} className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center">
          <SidebarMenuButton
            isActive={active}
            tooltip={label}
            className={menuBtnClass(active)}
            render={
              <Link href={first.href}>
                <Icon className="size-4 shrink-0" />
                <span className="truncate leading-5">{label}</span>
              </Link>
            }
          />
        </SidebarMenuItem>
      )
    }

    // Expanded mode, several pages → the header expands the section in
    // place instead of navigating. Previously it was a link to the first
    // page, so simply looking at what a section contained cost a page
    // load, and only the active section could ever be seen.
    if (!iconMode) {
      const expanded = isGroupOpen(group.label)
      return (
        <SidebarMenuItem key={group.label}>
          <SidebarMenuButton
            isActive={active && !expanded}
            className={menuBtnClass(active && !expanded)}
            render={
              <button
                type="button"
                aria-expanded={expanded}
                onClick={() => toggleGroup(group.label)}
              >
                <Icon className="size-4 shrink-0" />
                <span className="flex-1 truncate text-left leading-5">
                  {label}
                </span>
                <ChevronRight
                  className={cn(
                    "size-3.5 shrink-0 text-sidebar-foreground/50 transition-transform duration-150",
                    expanded && "rotate-90"
                  )}
                  aria-hidden
                />
              </button>
            }
          />
          {expanded ? (
            <SidebarMenuSub className="mr-0 gap-0.5 border-sidebar-border pr-0">
              {group.items.map((item) => (
                <SidebarMenuSubItem key={item.href}>
                  {linkRow(item, item.href === activeHref, false)}
                </SidebarMenuSubItem>
              ))}
            </SidebarMenuSub>
          ) : null}
        </SidebarMenuItem>
      )
    }

    // Mode ikon + multi-item → tombol flyout.
    const isOpen = flyout?.label === group.label
    return (
      <SidebarMenuItem key={group.label} className="flex justify-center">
        <button
          type="button"
          data-section-trigger
          title={label}
          aria-expanded={isOpen}
          onClick={(e) => openFlyout(group.label, e.currentTarget)}
          className={cn(
            "grid size-8 place-items-center rounded-md text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
            (active || isOpen) && "bg-sidebar-accent text-sidebar-primary",
            active && "shadow-sm ring-1 ring-sidebar-border",
          )}
        >
          <Icon className="size-4" />
        </button>
      </SidebarMenuItem>
    )
  }

  const openGroup = groups.find((g) => g.label === flyout?.label)

  return (
    <Sidebar data-print-hide collapsible="icon" side="left" className="border-r border-sidebar-border bg-sidebar shadow-sm">
      {/* `h-16` supaya persis setinggi navbar (`app-navbar.tsx` juga
          `h-16`), sehingga garis bawah keduanya menyambung jadi satu
          garis lurus di seluruh lebar layar. Sebelumnya tinggi header ini
          dihitung dari isinya (padding + baris logo) dan jatuh di 69px —
          5px lebih rendah dari navbar, dan patahannya terlihat tepat di
          pertemuan sidebar dengan konten. */}
      <SidebarHeader className="flex h-16 shrink-0 flex-col justify-center border-b border-sidebar-border px-3 py-0 group-data-[collapsible=icon]:px-2">
        <div className="flex min-w-0 items-center justify-start gap-2 group-data-[collapsible=icon]:justify-center">
          <BrandLogo />
          <div className="grid min-w-0 flex-1 gap-0.5 leading-none group-data-[collapsible=icon]:hidden">
            <span className="text-sm font-semibold tracking-[-0.084px] text-sidebar-foreground">Rantai Lake</span>
            <span className="text-[11px] font-normal tracking-[-0.04px] text-sidebar-foreground/60">Enterprise Lakehouse Console</span>
          </div>
        </div>
      </SidebarHeader>

      <SidebarContent className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:flex-col group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-0">
        <SidebarGroup className="gap-0 px-2 py-2 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:px-2">
          <SidebarMenu className="mt-0 flex flex-col gap-0.5 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-1.5">
            {groups.map(renderEntry)}
          </SidebarMenu>
        </SidebarGroup>

        {/* Daftar dashboard dan riwayat Copilot DULU ada di sini. Keduanya
            hanya muncul ketika sudah berada di halamannya masing-masing,
            jadi tak pernah bisa dipakai untuk MENUJU ke sana — itu isi
            halaman yang menempati ruang navigasi, dan ikut hilang begitu
            sidebar diciutkan jadi ikon. Sekarang keduanya ada di header
            halamannya sendiri (`BoardSwitcher`, `CopilotHistoryMenu`), dan
            sidebar murni membaca `nav-config` tanpa pengecualian per-rute. */}
        {/* The active section's pages used to be repeated down here, in a
            separate block. They now sit inside their own group above, so
            the section that is open is the one you are reading. */}
      </SidebarContent>

      {/* Flyout (hanya mode ikon) */}
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

      <SidebarFooter className="border-t border-sidebar-border p-2">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              tooltip={user?.name ?? "Signed in"}
              className="h-14 rounded-lg px-2 hover:bg-sidebar-accent group-data-[collapsible=icon]:size-10! group-data-[collapsible=icon]:h-10!"
              render={
                <div role="group" aria-label="Current user">
                  <div className="relative flex size-9 shrink-0 items-center justify-center rounded-full bg-primary/12 text-primary ring-1 ring-primary/20">
                    <CircleUserRound className="size-5" aria-hidden />
                    <span className="absolute bottom-0 right-0 size-2.5 rounded-full border-2 border-sidebar bg-emerald-500" />
                  </div>
                  <div className="grid min-w-0 flex-1 text-left leading-tight group-data-[collapsible=icon]:hidden">
                    <span className="truncate text-sm font-semibold text-sidebar-foreground">{user?.name ?? "Signed in"}</span>
                    <span className="truncate text-xs text-sidebar-foreground/60">{user?.email ?? ""}</span>
                  </div>
                </div>
              }
            />
          </SidebarMenuItem>
        </SidebarMenu>
      </SidebarFooter>
      <SidebarRail />
    </Sidebar>
  )
}
