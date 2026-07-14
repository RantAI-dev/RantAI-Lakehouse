"use client"

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
  Waves,
} from "lucide-react"
import {
  Sidebar,
  SidebarContent,
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
    title: "Intellegance Knowladge",
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
 * Rantai Lake brand mark shown in the sidebar header (32px).
 *
 * Uses the design-system primary color with a wave glyph to evoke the data
 * "lake". When the sidebar is collapsed to icons, the shadow is removed for a
 * cleaner appearance via `group-data-[collapsible=icon]:shadow-none`.
 */
function BrandLogo() {
  return (
    <div
      className="flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-md bg-primary text-primary-foreground shadow-sm group-data-[collapsible=icon]:shadow-none"
      aria-hidden
    >
      <Waves className="size-4.5" strokeWidth={2.5} />
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
      className="border-r border-sidebar-border bg-sidebar font-[family-name:var(--font-montserrat)] shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]"
    >
      <SidebarHeader className="flex flex-col border-b border-sidebar-border px-4 py-3 group-data-[collapsible=icon]:justify-center group-data-[collapsible=icon]:px-2 group-data-[collapsible=icon]:py-2">
        <div className="flex h-12 min-w-0 items-center justify-start gap-2 group-data-[collapsible=icon]:h-auto group-data-[collapsible=icon]:justify-center">
          <BrandLogo />
          <div className="grid min-w-0 flex-1 gap-0.5 leading-none group-data-[collapsible=icon]:hidden">
            <span className="text-sm font-medium tracking-[-0.084px] text-sidebar-foreground">
              Rantai
            </span>
            <span className="text-xs font-normal tracking-[-0.072px] text-sidebar-foreground">
              Lake
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
                      "h-8 rounded-md px-2 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground",
                      isActive &&
                        "border border-sidebar-border bg-sidebar text-primary dark:border-sidebar-border dark:bg-sidebar-accent dark:text-sidebar-primary"
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
                      "h-8 rounded-md px-2 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground",
                      isActive &&
                        "border border-sidebar-border bg-sidebar text-primary dark:border-sidebar-border dark:bg-sidebar-accent dark:text-sidebar-primary"
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
                      "h-8 rounded-md px-2 py-1.5 text-sm font-normal tracking-normal text-sidebar-foreground",
                      isActive &&
                        "border border-sidebar-border bg-sidebar text-primary dark:border-sidebar-border dark:bg-sidebar-accent dark:text-sidebar-primary"
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
      <SidebarRail />
    </Sidebar>
  )
}
