"use client"

import * as React from "react"
import Image from "next/image"
import Link from "next/link"
import { usePathname } from "next/navigation"
import { CircleUserRound, ChevronRight } from "lucide-react"
import {
  Sidebar,
  SidebarContent,
  SidebarFooter,
  SidebarGroup,
  SidebarGroupLabel,
  SidebarHeader,
  SidebarMenu,
  SidebarMenuButton,
  SidebarMenuItem,
  SidebarRail,
  useSidebar,
} from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"
import { visibleNavGroups, activeNavHref, type NavItem } from "./nav-config"

/** Rantai brand mark shown in the sidebar header (32px). */
function BrandLogo() {
  return (
    <div
      className="relative size-8 shrink-0 overflow-hidden rounded-md"
      aria-hidden
    >
      <Image
        src="/rantai.png"
        alt=""
        fill
        sizes="32px"
        className="object-cover"
        priority
      />
    </div>
  )
}

/**
 * Primary application sidebar rendered by the root layout.
 * Navigation groups come from `NAV_GROUPS` in `nav-config.ts` — edit that file
 * to add or reorder pages. Collapse behavior comes from `SidebarProvider`.
 */
export function AppSidebar() {
  const pathname = usePathname()
  const activeHref = activeNavHref(pathname)
  const { state } = useSidebar()
  const iconMode = state === "collapsed"

  const groups = visibleNavGroups()
  const activeGroup = groups.find((g) => g.items.some((it) => it.href === activeHref))?.label

  // Grup yang terbuka. Default: grup yang memuat halaman aktif. Saat pindah ke
  // halaman di grup lain, grup itu ikut terbuka (tanpa menutup yang lain).
  const [open, setOpen] = React.useState<Set<string>>(() => new Set(activeGroup ? [activeGroup] : []))
  React.useEffect(() => {
    if (activeGroup) setOpen((prev) => (prev.has(activeGroup) ? prev : new Set(prev).add(activeGroup)))
  }, [activeGroup])
  const toggle = (label: string) =>
    setOpen((prev) => {
      const n = new Set(prev)
      if (n.has(label)) n.delete(label)
      else n.add(label)
      return n
    })

  const renderItem = (item: NavItem) => {
    const active = item.href === activeHref
    const Icon = item.icon
    return (
      <SidebarMenuItem
        key={item.href}
        className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center"
      >
        <SidebarMenuButton
          isActive={active}
          tooltip={item.title}
          className={cn(
            "h-8 rounded-md px-2.5 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
            active &&
              "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border"
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

  return (
    <Sidebar
      collapsible="icon"
      side="left"
      className="border-r border-sidebar-border bg-sidebar shadow-sm"
    >
      <SidebarHeader className="flex flex-col border-b border-sidebar-border px-3 py-2.5 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-2 group-data-[collapsible=icon]:py-2">
        <div className="flex h-12 min-w-0 items-center justify-start gap-2 group-data-[collapsible=icon]:h-auto group-data-[collapsible=icon]:justify-center">
          <BrandLogo />
          <div className="grid min-w-0 flex-1 gap-0.5 leading-none group-data-[collapsible=icon]:hidden">
            <span className="text-sm font-semibold tracking-[-0.084px] text-sidebar-foreground">
              Rantai Lake
            </span>
            <span className="text-[11px] font-normal tracking-[-0.04px] text-sidebar-foreground/60">
              Enterprise Lakehouse Console
            </span>
          </div>
        </div>
      </SidebarHeader>
      <SidebarContent className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:flex-col group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-0">
        {groups.map((group) => {
          // Grup 1-item (mis. AI Copilot, Settings) → tampil datar, tanpa header.
          if (group.items.length === 1) {
            return (
              <SidebarGroup
                key={group.label}
                className="gap-0 px-2 py-1 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:px-2"
              >
                <SidebarMenu className="mt-0 flex flex-col gap-0.5 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-2">
                  {group.items.map(renderItem)}
                </SidebarMenu>
              </SidebarGroup>
            )
          }

          const isOpen = iconMode || open.has(group.label)
          const hasActive = group.items.some((it) => it.href === activeHref)
          return (
            <SidebarGroup
              key={group.label}
              className="gap-0 px-2 py-1.5 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:px-2"
            >
              {/* Header grup = tombol accordion (tersembunyi di mode ikon). */}
              <SidebarGroupLabel
                render={
                  <button type="button" onClick={() => toggle(group.label)} aria-expanded={isOpen}>
                    <span className="flex-1 text-left">{group.label}</span>
                    {!hasActive || isOpen ? (
                      <ChevronRight
                        className={cn(
                          "size-3.5 shrink-0 text-sidebar-foreground/50 transition-transform",
                          isOpen && "rotate-90",
                        )}
                      />
                    ) : (
                      <span className="size-1.5 shrink-0 rounded-full bg-sidebar-primary" aria-label="halaman aktif di grup ini" />
                    )}
                  </button>
                }
                className="flex h-7 w-full items-center gap-1 px-2 py-1 text-xs font-medium leading-4 tracking-[-0.072px] text-sidebar-foreground/70 hover:text-sidebar-foreground"
              />
              {isOpen ? (
                <SidebarMenu className="mt-0.5 flex flex-col gap-0.5 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-2">
                  {group.items.map(renderItem)}
                </SidebarMenu>
              ) : null}
            </SidebarGroup>
          )
        })}
      </SidebarContent>
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
                    <span className="truncate text-sm font-semibold text-sidebar-foreground">
                      Admin User
                    </span>
                    <span className="truncate text-xs text-sidebar-foreground/60">
                      admin@rantai.id
                    </span>
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
