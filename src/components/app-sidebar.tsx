"use client"

import Image from "next/image"
import Link from "next/link"
import { usePathname } from "next/navigation"
import {
  LayoutDashboard,
  GitBranch,
  SearchCode,
  Users,
  Building2,
  FileText,
  ScanSearch,
  Workflow,
  Plug,
  Library,
  Boxes,
  Brain,
  CircleUserRound,
} from "lucide-react"
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
} from "@/components/ui/sidebar"
import { cn } from "@/lib/utils"

const mainNav = [
  { title: "Dashboard", href: "/", icon: LayoutDashboard },
  { title: "Pipelines", href: "/pipelines", icon: GitBranch },
  { title: "Query Studio", href: "/query-studio", icon: SearchCode },
]

const discoverNav = [
  {
    title: "Search & similarity",
    href: "/semantic-search",
    icon: ScanSearch,
  },
  { title: "Lineage", href: "/lineage", icon: Workflow },
  { title: "Connectors", href: "/connectors", icon: Plug },
  { title: "Data Catalog", href: "/data-catalog", icon: Library },
  {
    title: "Intelligence Knowledge",
    href: "/intelligence-knowledge",
    icon: Brain,
  },
] as const

const adminNav = [
  { title: "User Management", href: "/user-management", icon: Users },
  { title: "Tenant Management", href: "/tenant-management", icon: Boxes },
  { title: "Data Governance", href: "/data-governance", icon: Building2 },
  { title: "Audit Logs", href: "/audit-logs", icon: FileText },
] as const

/**
 * Rantai brand mark shown in the sidebar header (32px).
 * Uses the official logo from `/public/rantai.png`.
 */
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
 * Primary application sidebar shown on every page (rendered by the root layout).
 *
 * Contains three navigation groups (Main, Discover, Administration) sourced from
 * the static `mainNav`, `discoverNav`, and `adminNav` arrays defined above.
 * Active-state matching:
 * - `mainNav` and `discoverNav`: exact path or any nested route (`startsWith(href + "/")`).
 *   Dashboard ("/") only matches when pathname === "/" to avoid lighting up on every page.
 * - `adminNav`: strict equality only.
 *
 * To add a new top-level page, append an entry to the appropriate nav array.
 * Sidebar collapse / icon-only behavior is provided by the parent `SidebarProvider`.
 */
export function AppSidebar() {
  const pathname = usePathname()

  return (
    <Sidebar
      collapsible="icon"
      side="left"
      className="border-r border-sidebar-border bg-sidebar font-[family-name:var(--font-montserrat)] shadow-sm"
    >
      <SidebarHeader className="flex flex-col border-b border-sidebar-border px-3 py-2.5 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-2 group-data-[collapsible=icon]:py-2">
        <div className="flex h-12 min-w-0 items-center justify-start gap-2 group-data-[collapsible=icon]:h-auto group-data-[collapsible=icon]:justify-center">
          <BrandLogo />
          <div className="grid min-w-0 flex-1 gap-0.5 leading-none group-data-[collapsible=icon]:hidden">
            <span className="text-sm font-semibold tracking-[-0.084px] text-sidebar-foreground">
              Rantai Lake
            </span>
            <span className="text-[11px] font-normal tracking-[-0.04px] text-sidebar-foreground/60">
              Data Intelligence Platform
            </span>
          </div>
        </div>
      </SidebarHeader>
      <SidebarContent className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:flex-col group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-0">
        <SidebarGroup className="gap-0 px-2 py-3 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:px-2 group-data-[collapsible=icon]:py-2">
          <SidebarGroupLabel className="h-8 px-2 py-1.5 text-xs font-medium leading-4 tracking-[-0.072px] text-sidebar-foreground/70">
            Main
          </SidebarGroupLabel>
          <SidebarMenu className="mt-0 flex flex-col gap-1 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-2">
            {mainNav.map((item) => {
              const isActive =
                item.href === "/"
                  ? pathname === "/"
                  : pathname === item.href ||
                    pathname.startsWith(`${item.href}/`)
              const Icon = item.icon
              return (
                <SidebarMenuItem key={item.href} className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center">
                  <SidebarMenuButton
                    isActive={isActive}
                    tooltip={item.title}
                    className={cn(
                      "h-9 rounded-md px-2.5 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                      isActive &&
                        "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border"
                    )}
                    render={
                      <Link href={item.href}>
                        <Icon className="size-4 shrink-0" />
                        <span className="leading-5">{item.title}</span>
                      </Link>
                    }
                  />
                </SidebarMenuItem>
              )
            })}
          </SidebarMenu>
        </SidebarGroup>
        <SidebarGroup className="gap-0 px-2 py-3 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:px-2 group-data-[collapsible=icon]:py-2">
          <SidebarGroupLabel className="h-8 px-2 py-1.5 text-xs font-medium leading-4 tracking-[-0.072px] text-sidebar-foreground/70">
            Discover
          </SidebarGroupLabel>
          <SidebarMenu className="mt-0 flex flex-col gap-1 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-2">
            {discoverNav.map((item) => {
              const isActive =
                pathname === item.href ||
                pathname.startsWith(`${item.href}/`)
              const Icon = item.icon
              return (
                <SidebarMenuItem key={item.href} className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center">
                  <SidebarMenuButton
                    isActive={isActive}
                    tooltip={item.title}
                    className={cn(
                      "h-9 rounded-md px-2.5 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                      isActive &&
                        "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border"
                    )}
                    render={
                      <Link href={item.href}>
                        <Icon className="size-4 shrink-0" />
                        <span className="leading-5">{item.title}</span>
                      </Link>
                    }
                  />
                </SidebarMenuItem>
              )
            })}
          </SidebarMenu>
        </SidebarGroup>
        <SidebarGroup className="gap-0 px-2 py-3 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:px-2 group-data-[collapsible=icon]:py-2">
          <SidebarGroupLabel className="h-8 px-2 py-1.5 text-xs font-medium leading-4 tracking-[-0.072px] text-sidebar-foreground/70">
            Administration
          </SidebarGroupLabel>
          <SidebarMenu className="mt-0 flex flex-col gap-1 group-data-[collapsible=icon]:w-full group-data-[collapsible=icon]:items-center group-data-[collapsible=icon]:gap-2">
            {adminNav.map((item) => {
              const isActive = pathname === item.href
              const Icon = item.icon
              return (
                <SidebarMenuItem key={item.href} className="group-data-[collapsible=icon]:flex group-data-[collapsible=icon]:justify-center">
                  <SidebarMenuButton
                    isActive={isActive}
                    tooltip={item.title}
                    className={cn(
                      "h-9 rounded-md px-2.5 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground/80 hover:bg-sidebar-accent hover:text-sidebar-accent-foreground",
                      isActive &&
                        "bg-sidebar-accent font-medium text-sidebar-primary shadow-sm ring-1 ring-sidebar-border"
                    )}
                    render={
                      <Link href={item.href}>
                        <Icon className="size-4 shrink-0" />
                        <span className="leading-5">{item.title}</span>
                      </Link>
                    }
                  />
                </SidebarMenuItem>
              )
            })}
          </SidebarMenu>
        </SidebarGroup>
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
