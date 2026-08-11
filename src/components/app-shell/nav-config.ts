import {
  Activity,
  BellRing,
  Boxes,
  Bot,
  Building2,
  CircleGauge,
  ClipboardCheck,
  Database,
  FileSearch,
  FileText,
  GitBranch,
  Globe2,
  HardDrive,
  KeyRound,
  Layers,
  LayoutDashboard,
  Library,
  ListChecks,
  Plug,
  Radio,
  ScanSearch,
  SearchCode,
  Server,
  Settings,
  ShieldCheck,
  Sparkles,
  Tags,
  Users,
  Wallet,
  Waypoints,
  Workflow,
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
  items: NavItem[]
}

/**
 * Single source of truth for the product information architecture.
 * The sidebar and navbar page titles are both derived from this config.
 * To add a top-level page, add an entry to the appropriate group.
 *
 * Grouping rationale:
 * - Overview: monitor the whole platform (dashboard, feed, alerts).
 * - Data: where data lives (explore, catalog, lifecycle, ingress).
 * - Build: author and operate data movement and queries.
 * - Intelligence: knowledge flow — sources → indexing → retrieval → agents.
 * - Governance: control and evidence.
 * - Operations: platform runtime health and spend.
 * - Administration: identity and workspace configuration.
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    label: "Overview",
    items: [
      { title: "Overview", href: "/", icon: LayoutDashboard },
      { title: "Activity", href: "/activity", icon: Activity },
      { title: "Alerts", href: "/alerts", icon: BellRing },
    ],
  },
  {
    label: "Data",
    items: [
      { title: "Data Explorer", href: "/data", icon: Database },
      { title: "Catalog", href: "/catalog", icon: Library },
      { title: "Storage Lifecycle", href: "/storage", icon: HardDrive },
      { title: "Connectors", href: "/connectors", icon: Plug },
    ],
  },
  {
    label: "Build",
    items: [
      { title: "Pipelines", href: "/pipelines", icon: GitBranch },
      { title: "Streaming Jobs", href: "/streaming", icon: Radio },
      { title: "Query Studio", href: "/query-studio", icon: SearchCode },
    ],
  },
  {
    label: "Intelligence",
    items: [
      { title: "Knowledge", href: "/knowledge", icon: Sparkles },
      { title: "Vector Jobs", href: "/vector-jobs", icon: Layers },
      { title: "Semantic Search", href: "/semantic-search", icon: ScanSearch },
      { title: "Agent Workflows", href: "/agents/workflows", icon: Workflow },
      { title: "Digital Employees", href: "/agents/employees", icon: Bot },
      { title: "Approvals", href: "/agents/approvals", icon: ClipboardCheck },
      { title: "Tool Registry", href: "/agents/tools", icon: Wrench },
    ],
  },
  {
    label: "Governance",
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
      { title: "Residency", href: "/residency", icon: Globe2 },
    ],
  },
  {
    label: "Operations",
    items: [
      { title: "Workloads", href: "/workloads", icon: CircleGauge },
      { title: "Observability", href: "/observability", icon: FileSearch },
      { title: "Services", href: "/services", icon: Server },
      { title: "Usage & Budgets", href: "/usage", icon: Wallet },
    ],
  },
  {
    label: "Administration",
    items: [
      { title: "Users", href: "/admin/users", icon: Users },
      { title: "Teams & Roles", href: "/admin/roles", icon: Building2 },
      { title: "Tenants", href: "/admin/tenants", icon: Boxes },
      {
        title: "Service Identities",
        href: "/admin/service-identities",
        icon: KeyRound,
      },
      { title: "Settings", href: "/settings", icon: Settings },
    ],
  },
]

/** Flat list of every sidebar nav item, used for active-state and command search. */
export const ALL_NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((g) => g.items)

/**
 * Routes that live under a sidebar entry but have their own page title
 * (for example Query Studio workspace tabs). They never appear in the sidebar;
 * the parent nav item stays highlighted while these titles win the lookup.
 */
const SECONDARY_ROUTES: { title: string; href: string }[] = [
  { title: "Saved Queries", href: "/query-studio/saved" },
  { title: "Collaboration", href: "/query-studio/collaboration" },
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
