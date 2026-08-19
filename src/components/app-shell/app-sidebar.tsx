"use client"

import * as React from "react"
import { createPortal } from "react-dom"
import Image from "next/image"
import Link from "next/link"
import { usePathname } from "next/navigation"
import { CircleUserRound, ChevronRight } from "lucide-react"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
} from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"
import { visibleNavGroups, activeNavHref, type NavGroup, type NavItem } from "./nav-config"

/** Rantai brand mark shown in the sidebar header (32px). */
function BrandLogo() {
  return (
    <div className="relative size-8 shrink-0 overflow-hidden rounded-md" aria-hidden>
      <Image src="/rantai.png" alt="" fill sizes="32px" className="object-cover" priority />
    </div>
  )
}

type FlyoutState = { label: string; top: number; left: number } | null

/**
 * Primary application sidebar. Tiap SECTION (grup) = satu tombol; klik → panel
 * submenu FLYOUT keluar ke kanan (via portal, jadi tak terpotong & tetap jalan
 * saat sidebar diciutkan ke ikon). Grup 1-item tampil sebagai link langsung.
 * Navigasi bersumber dari NAV_GROUPS di nav-config.ts.
 */
export function AppSidebar() {
  const pathname = usePathname()
  const activeHref = activeNavHref(pathname)
  const groups = visibleNavGroups()

  const [flyout, setFlyout] = React.useState<FlyoutState>(null)
  const flyoutRef = React.useRef<HTMLDivElement>(null)
  const [mounted, setMounted] = React.useState(false)
  React.useEffect(() => setMounted(true), [])

  // Tutup flyout saat pindah halaman.
  React.useEffect(() => setFlyout(null), [pathname])

  // Tutup saat klik di luar / tekan Escape.
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

  const openSection = (label: string, el: HTMLElement) => {
    if (flyout?.label === label) return setFlyout(null)
    const r = el.getBoundingClientRect()
    setFlyout({ label, top: r.top, left: r.right + 6 })
  }

  const linkClass = (active: boolean) =>
    cn(
      "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm font-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
      active && "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border",
    )

  const renderDirectItem = (item: NavItem) => {
    const Icon = item.icon
    return (
      <SidebarMenuItem key={item.href} className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center">
        <SidebarMenuButton
          isActive={item.href === activeHref}
          tooltip={item.title}
          className={cn(
            "h-8 rounded-md px-2.5 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
            item.href === activeHref && "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border",
          )}
          render={
            <Link href={item.href}>
              <Icon className="size-4 shrink-0" />
              <span className="truncate leading-5">{item.title}</span>
            </Link>
          }
        />
      </SidebarMenuItem>
    )
  }

  const renderSection = (group: NavGroup) => {
    const Icon = group.icon ?? group.items[0].icon
    const active = group.items.some((it) => it.href === activeHref)
    const isOpen = flyout?.label === group.label
    return (
      <SidebarMenuItem key={group.label} className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center">
        <button
          type="button"
          data-section-trigger
          title={group.label}
          aria-expanded={isOpen}
          onClick={(e) => openSection(group.label, e.currentTarget)}
          className={cn(
            "flex h-8 w-full items-center gap-2 rounded-md px-2.5 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
            "group-data-[collapsible=icon]:w-8 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-0",
            (active || isOpen) && "bg-sidebar-accent font-medium text-sidebar-primary",
            active && "shadow-sm ring-1 ring-sidebar-border",
          )}
        >
          <Icon className="size-4 shrink-0" />
          <span className="truncate leading-5 group-data-[collapsible=icon]:hidden">{group.label}</span>
          <ChevronRight
            className={cn(
              "ml-auto size-3.5 shrink-0 text-sidebar-foreground/50 transition-transform group-data-[collapsible=icon]:hidden",
              isOpen && "rotate-90",
            )}
          />
        </button>
      </SidebarMenuItem>
    )
  }

  const openGroup = groups.find((g) => g.label === flyout?.label)

  return (
    <Sidebar collapsible="icon" side="left" className="border-r border-sidebar-border bg-sidebar shadow-sm">
      <SidebarHeader className="flex flex-col border-b border-sidebar-border px-3 py-2.5 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-2 group-data-[collapsible=icon]:py-2">
        <div className="flex h-12 min-w-0 items-center justify-start gap-2 group-data-[collapsible=icon]:h-auto group-data-[collapsible=icon]:justify-center">
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
            {groups.map((group) =>
              group.items.length === 1 ? renderDirectItem(group.items[0]) : renderSection(group),
            )}
          </SidebarMenu>
        </SidebarGroup>
      </SidebarContent>

      {/* Flyout submenu (portal → tak terpotong oleh sidebar) */}
      {mounted && flyout && openGroup
        ? createPortal(
            <div
              ref={flyoutRef}
              style={{ position: "fixed", top: Math.min(flyout.top, window.innerHeight - 40 - openGroup.items.length * 34), left: flyout.left }}
              className="z-[60] w-56 rounded-xl border border-sidebar-border bg-sidebar p-1.5 shadow-2xl"
            >
              <p className="px-2 py-1 text-[11px] font-medium uppercase tracking-wide text-sidebar-foreground/60">{openGroup.label}</p>
              {openGroup.items.map((item) => {
                const Icon = item.icon
                return (
                  <Link key={item.href} href={item.href} onClick={() => setFlyout(null)} className={linkClass(item.href === activeHref)}>
                    <Icon className="size-4 shrink-0" />
                    <span className="truncate">{item.title}</span>
                  </Link>
                )
              })}
            </div>,
            document.body,
          )
        : null}

      <SidebarFooter className="border-t border-sidebar-border p-2">
        <SidebarMenu>
          <SidebarMenuItem>
            <SidebarMenuButton
              size="lg"
              tooltip="Admin User"
              className="h-14 rounded-lg px-2 hover:bg-sidebar-accent group-data-[collapsible=icon]:size-10! group-data-[collapsible=icon]:h-10!"
              render={
                <div role="group" aria-label="Current user">
                  <div className="relative flex size-9 shrink-0 items-center justify-center rounded-full bg-primary/12 text-primary ring-1 ring-primary/20">
                    <CircleUserRound className="size-5" aria-hidden />
                    <span className="absolute bottom-0 right-0 size-2.5 rounded-full border-2 border-sidebar bg-emerald-500" />
                  </div>
                  <div className="grid min-w-0 flex-1 text-left leading-tight group-data-[collapsible=icon]:hidden">
                    <span className="truncate text-sm font-semibold text-sidebar-foreground">Admin User</span>
                    <span className="truncate text-xs text-sidebar-foreground/60">admin@rantai.id</span>
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
