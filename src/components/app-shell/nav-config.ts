import {
  Activity,
  BarChart3,
  BellRing,
  Boxes,
  Bot,
  Building2,
  CircleGauge,
  ClipboardCheck,
  Database,
  DatabaseZap,
  FileSearch,
  FileText,
  GitBranch,
  History,
  KeyRound,
  LayoutDashboard,
  Layers,
  Library,
  ListChecks,
  LogIn,
  MonitorSmartphone,
  PackageCheck,
  Plug,
  SearchCode,
  Server,
  Settings,
  ShieldCheck,
  Sparkles,
  Tags,
  Users,
  Waypoints,
  Wrench,
  type LucideIcon,
} from "lucide-react"

export type NavItem = {
  title: string
  href: string
  icon: LucideIcon
}

export type NavGroup = {
  label: string
  /** Section icon — for the flyout button in the sidebar. */
  icon?: LucideIcon
  items: NavItem[]
}

/**
 * Single source of truth for the product information architecture.
 * The sidebar and navbar page titles are both derived from this config.
 * To add a top-level page, add an entry to the appropriate group.
 *
 * Grouping rationale:
 * - Overview: monitor the whole platform (dashboard, feed, alerts).
 * - Data: where data lives (explore, catalog, ingress).
 * - Build: author and operate data movement and queries.
 * - Intelligence: digital employees, their runs and approvals.
 * - Governance: control and evidence.
 * - Operations: platform runtime health and spend.
 * - Administration: identity and workspace configuration.
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    label: "AI",
    icon: Sparkles,
    items: [
      { title: "AI Copilot", href: "/copilot", icon: Sparkles },
    ],
  },
  {
    label: "Dashboards",
    icon: BarChart3,
    items: [
      { title: "Dashboards", href: "/dashboards", icon: BarChart3 },
    ],
  },
  {
    label: "Overview",
    icon: LayoutDashboard,
    items: [
      { title: "Overview", href: "/", icon: LayoutDashboard },
      { title: "Activity", href: "/activity", icon: Activity },
      { title: "Alerts", href: "/alerts", icon: BellRing },
    ],
  },
  {
    label: "Data",
    icon: Database,
    items: [
      { title: "Data Explorer", href: "/data", icon: Database },
      { title: "Catalog", href: "/catalog", icon: Library },
      { title: "Connectors", href: "/connectors", icon: Plug },
      { title: "Gold Exports", href: "/gold-exports", icon: PackageCheck },
    ],
  },
  {
    label: "Lakehouse",
    icon: Layers,
    items: [
      { title: "Tables", href: "/lakehouse/tables", icon: Layers },
      { title: "Capacity", href: "/lakehouse/capacity", icon: BarChart3 },
    ],
  },
  {
    label: "Build",
    icon: GitBranch,
    items: [
      { title: "Pipelines", href: "/pipelines", icon: GitBranch },
      { title: "Query Studio", href: "/query-studio", icon: SearchCode },
    ],
  },
  {
    label: "Intelligence",
    icon: Bot,
    items: [
      { title: "Digital Employees", href: "/agents/employees", icon: Bot },
      { title: "Agent Runs", href: "/agents/runs", icon: History },
      { title: "Approvals", href: "/agents/approvals", icon: ClipboardCheck },
    ],
  },
  {
    label: "Governance",
    icon: ShieldCheck,
    items: [
      { title: "Policies", href: "/governance/policies", icon: ShieldCheck },
      {
        title: "Classification & Masking",
        href: "/governance/classification",
        icon: Tags,
      },
      {
        title: "Data Quality",
        href: "/governance/data-quality",
        icon: ListChecks,
      },
      { title: "Lineage", href: "/lineage", icon: Waypoints },
      { title: "Audit", href: "/audit", icon: FileText },
      {
        title: "Bronze Maintenance",
        href: "/governance/maintenance",
        icon: Wrench,
      },
      {
        title: "Ingestion (CDC)",
        href: "/governance/ingestion",
        icon: DatabaseZap,
      },
    ],
  },
  {
    label: "Operations",
    icon: Server,
    items: [
      { title: "Workloads", href: "/workloads", icon: CircleGauge },
      { title: "Observability", href: "/observability", icon: FileSearch },
      { title: "Services", href: "/services", icon: Server },
    ],
  },
  {
    label: "Administration",
    icon: Settings,
    items: [
      { title: "Users", href: "/admin/users", icon: Users },
      { title: "Teams & Roles", href: "/admin/roles", icon: Building2 },
      { title: "Tenants", href: "/admin/tenants", icon: Boxes },
      {
        title: "Service Identities",
        href: "/admin/service-identities",
        icon: KeyRound,
      },
      { title: "SSO", href: "/admin/sso", icon: LogIn },
      { title: "Sessions", href: "/admin/sessions", icon: MonitorSmartphone },
    ],
  },
]

/** Flat list of every sidebar nav item, used for active-state and command search. */
export const ALL_NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((g) => g.items)

/** The group (section) that contains the active page — for bottom-nav & sub-navigation. */
export function activeNavGroup(pathname: string): NavGroup | undefined {
  const href = activeNavHref(pathname)
  return NAV_GROUPS.find((g) => g.items.some((it) => it.href === href))
}

/** Sub-pages of the active section (empty when there's only 1 item). */
export function subNavItems(pathname: string): NavItem[] {
  const g = activeNavGroup(pathname)
  return g && g.items.length > 1 ? g.items : []
}

/**
 * Routes that live under a sidebar entry but have their own page title
 * (for example Query Studio workspace tabs). They never appear in the sidebar;
 * the parent nav item stays highlighted while these titles win the lookup.
 */
const SECONDARY_ROUTES: { title: string; href: string }[] = [
  { title: "Saved Queries", href: "/query-studio/saved" },
]

function bestMatch<T extends { href: string }>(
  pathname: string,
  candidates: T[]
): T | undefined {
  return candidates
    .filter((item) =>
      item.href === "/"
        ? pathname === "/"
        : pathname === item.href || pathname.startsWith(`${item.href}/`)
    )
    .sort((a, b) => b.href.length - a.href.length)[0]
}

/**
 * Resolves which sidebar item should be highlighted for a pathname.
 * Longest-match semantics so nested routes highlight their closest parent.
 */
export function activeNavHref(pathname: string): string | undefined {
  return bestMatch(pathname, ALL_NAV_ITEMS)?.href
}

/**
 * Resolves the product page title for a pathname by longest matching href.
 * Secondary routes win over their parent nav item. Falls back to "Rantai Lake".
 */
export function pageTitleFor(pathname: string): string {
  const match = bestMatch(pathname, [
    ...SECONDARY_ROUTES,
    ...ALL_NAV_ITEMS.map(({ title, href }) => ({ title, href })),
  ])
  return match?.title ?? "Rantai Lake"
}
