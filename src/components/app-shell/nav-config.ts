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
  Globe2,
  HardDrive,
  KeyRound,
  Layers,
  LayoutDashboard,
  Library,
  ListChecks,
  Plug,
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
  /**
   * true = halaman masih memakai data MOCK (belum tersambung engine nyata).
   * Disembunyikan dari sidebar kecuali NEXT_PUBLIC_SHOW_PREVIEW="1".
   * Hilangkan flag ini begitu service-nya sudah nyata.
   */
  preview?: boolean
}

export type NavGroup = {
  label: string
  /** Ikon section — untuk tombol flyout di sidebar. */
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
 * - Data: where data lives (explore, catalog, lifecycle, ingress).
 * - Build: author and operate data movement and queries.
 * - Intelligence: knowledge flow — sources → indexing → retrieval → agents.
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
      { title: "Storage Lifecycle", href: "/storage", icon: HardDrive },
      { title: "Connectors", href: "/connectors", icon: Plug, preview: true },
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
      { title: "Knowledge", href: "/knowledge", icon: Sparkles, preview: true },
      { title: "Vector Jobs", href: "/vector-jobs", icon: Layers, preview: true },
      { title: "Semantic Search", href: "/semantic-search", icon: ScanSearch, preview: true },
      { title: "Agent Workflows", href: "/agents/workflows", icon: Workflow, preview: true },
      { title: "Digital Employees", href: "/agents/employees", icon: Bot, preview: true },
      { title: "Approvals", href: "/agents/approvals", icon: ClipboardCheck, preview: true },
      { title: "Tool Registry", href: "/agents/tools", icon: Wrench, preview: true },
    ],
  },
  {
    label: "Governance",
    icon: ShieldCheck,
    items: [
      { title: "Policies", href: "/governance/policies", icon: ShieldCheck, preview: true },
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
      { title: "Usage & Budgets", href: "/usage", icon: Wallet },
    ],
  },
  {
    label: "Administration",
    icon: Settings,
    items: [
      { title: "Users", href: "/admin/users", icon: Users, preview: true },
      { title: "Teams & Roles", href: "/admin/roles", icon: Building2, preview: true },
      { title: "Tenants", href: "/admin/tenants", icon: Boxes, preview: true },
      {
        title: "Service Identities",
        href: "/admin/service-identities",
        icon: KeyRound,
        preview: true,
      },
      { title: "Settings", href: "/settings", icon: Settings },
    ],
  },
]

/**
 * Apakah item preview (mock) ditampilkan. Default TIDAK; set
 * NEXT_PUBLIC_SHOW_PREVIEW="1" untuk memunculkan lagi semua halaman mock.
 */
export const SHOW_PREVIEW = process.env.NEXT_PUBLIC_SHOW_PREVIEW === "1"

/**
 * Grup nav yang tampil di sidebar. Menyaring item `preview` (kecuali
 * SHOW_PREVIEW), lalu membuang grup yang jadi kosong.
 */
export function visibleNavGroups(): NavGroup[] {
  if (SHOW_PREVIEW) return NAV_GROUPS
  return NAV_GROUPS.map((g) => ({
    ...g,
    items: g.items.filter((it) => !it.preview),
  })).filter((g) => g.items.length > 0)
}

/** Flat list of every sidebar nav item, used for active-state and command search. */
export const ALL_NAV_ITEMS: NavItem[] = NAV_GROUPS.flatMap((g) => g.items)

/** Grup (section) yang memuat halaman aktif — untuk bottom-nav & sub-navigasi. */
export function activeNavGroup(pathname: string): NavGroup | undefined {
  const href = activeNavHref(pathname)
  return visibleNavGroups().find((g) => g.items.some((it) => it.href === href))
}

/** Sub-halaman section aktif (kosong bila hanya 1 item). */
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
