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
  HeartPulse,
  History,
  Home,
  KeyRound,
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
  /**
   * `true` when the page still runs on mock data (not wired to a real
   * service yet). Hidden from the sidebar unless
   * `NEXT_PUBLIC_SHOW_PREVIEW="1"`. Remove this flag once the page's
   * service is real.
   */
  preview?: boolean
}

export type NavGroup = {
  label: string
  /** Section icon — for the flyout button in the sidebar. */
  icon?: LucideIcon
  items: NavItem[]
  /**
   * This group starts open on first visit (before any saved user choice).
   * A group containing the active page is always open regardless, so this
   * flag only affects the first impression.
   */
  defaultOpen?: boolean
  /**
   * DERIVED, never hand-written. Set by `visibleNavGroups()` when EVERY
   * item in a group is still `preview` — the group still renders
   * (disabled, "Soon" badge) instead of vanishing without a trace.
   */
  comingSoon?: boolean
  /**
   * DERIVED. `true` when some of a group's items are hidden as preview.
   *
   * Used by the sidebar to choose a label. A group genuinely declared
   * with 1 item (e.g. "AI" → "AI Copilot") uses that item's title, since
   * it is more descriptive. But a group that BECAME 1 item through
   * filtering must keep its group label — otherwise "Administration"
   * would rename itself to "Settings" just because its other four pages
   * are still mock.
   */
  partiallyHidden?: boolean
}

/**
 * Single source of truth for the product information architecture.
 * The sidebar and navbar page titles are both derived from this config.
 * To add a top-level page, add an entry to the appropriate group.
 *
 * Grouping rationale (the console is agentic-first, so it opens on the
 * AI-first Home and "Ask AI" sits right under it, not in a section):
 * - Home / Ask AI: where work starts — ask or instruct, see what needs you.
 * - Data: where data lives and how it is read (catalog, queries, boards).
 * - Build: author and operate data movement and outbound exports.
 * - Governance: the rules applied to the data (policies, quality, lineage).
 * - Monitoring: whether the platform is healthy, and what happened — the
 *   old "Overview" dashboard lives here as Health, next to alerts and the
 *   audit trail, instead of being the landing page.
 * - Intelligence: digital employees and their runs. Still `preview`: they
 *   have not been proven end-to-end, so the group renders as "Soon" rather
 *   than as working pages. Their approvals inbox is under Governance.
 * - Administration: identity and workspace configuration.
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    label: "Home",
    icon: Home,
    items: [{ title: "Home", href: "/", icon: Home }],
  },
  {
    label: "Ask AI",
    icon: Sparkles,
    items: [{ title: "Ask AI", href: "/copilot", icon: Sparkles }],
  },
  {
    label: "Data",
    icon: Database,
    defaultOpen: true,
    items: [
      { title: "Catalog", href: "/catalog", icon: Library },
      { title: "Data Explorer", href: "/data", icon: Database },
      { title: "Tables", href: "/lakehouse/tables", icon: Layers },
      { title: "Query Studio", href: "/query-studio", icon: SearchCode },
      { title: "Dashboards", href: "/dashboards", icon: BarChart3 },
      { title: "Sources", href: "/connectors", icon: Plug },
    ],
  },
  {
    label: "Build",
    icon: GitBranch,
    items: [
      { title: "Pipelines", href: "/pipelines", icon: GitBranch },
      { title: "Exports", href: "/gold-exports", icon: PackageCheck },
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
      // Not under Intelligence: Copilot's destructive tools and data access
      // requests queue here, so it must stay reachable while the digital
      // employee pages are hidden.
      { title: "Approvals", href: "/agents/approvals", icon: ClipboardCheck },
    ],
  },
  {
    label: "Monitoring",
    icon: HeartPulse,
    items: [
      { title: "Health", href: "/health", icon: HeartPulse },
      { title: "Alerts", href: "/alerts", icon: BellRing },
      { title: "Audit Log", href: "/audit", icon: FileText },
      { title: "Activity", href: "/activity", icon: Activity },
      { title: "Workloads", href: "/workloads", icon: CircleGauge },
      { title: "Observability", href: "/observability", icon: FileSearch },
      { title: "Services", href: "/services", icon: Server },
      { title: "Capacity", href: "/lakehouse/capacity", icon: BarChart3 },
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
    label: "Intelligence",
    icon: Bot,
    items: [
      {
        title: "Digital Employees",
        href: "/agents/employees",
        icon: Bot,
        preview: true,
      },
      { title: "Agent Runs", href: "/agents/runs", icon: History, preview: true },
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

/**
 * Whether preview (mock) items are shown. Default is NO; set
 * `NEXT_PUBLIC_SHOW_PREVIEW="1"` to bring back every mock page.
 */
export const SHOW_PREVIEW = process.env.NEXT_PUBLIC_SHOW_PREVIEW === "1"

/**
 * Nav groups shown in the sidebar, with `preview` items filtered out
 * unless SHOW_PREVIEW is on.
 *
 * A group whose items are ALL preview is not dropped, only flagged
 * `comingSoon` and still rendered (disabled, "Soon" badge). It used to
 * vanish entirely — a section with several pages and none of them ever
 * shown left the product map in this config out of sync with what the
 * user actually saw, with no hint anything was missing.
 */
export function visibleNavGroups(): NavGroup[] {
  if (SHOW_PREVIEW) return NAV_GROUPS
  return NAV_GROUPS.map((g) => {
    const items = g.items.filter((it) => !it.preview)
    return items.length > 0
      ? { ...g, items, partiallyHidden: items.length < g.items.length }
      : // The preview items are kept so the flyout/label still has
        // content to show; `comingSoon` is what prevents navigating to them.
        { ...g, comingSoon: true }
  })
}

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
