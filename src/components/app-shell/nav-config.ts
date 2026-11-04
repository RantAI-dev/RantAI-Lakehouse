import {
  BarChart3,
  BellRing,
  Boxes,
  Building2,
  ClipboardCheck,
  Database,
  FileText,
  GitBranch,
  HeartPulse,
  History,
  LayoutGrid,
  Home,
  KeyRound,
  Library,
  ListChecks,
  Plug,
  SearchCode,
  Settings,
  SlidersHorizontal,
  ShieldCheck,
  Users,
  type LucideIcon,
} from "lucide-react"

export type NavItem = {
  title: string
  href: string
  icon: LucideIcon
  /**
   * A second destination for this entry, shown as an icon button at the
   * right end of its row (where a section's chevron sits). The row itself
   * still goes to `href`. Only rendered for a single-page entry in the
   * expanded sidebar.
   */
  action?: { title: string; href: string; icon: LucideIcon }
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
}

/**
 * Single source of truth for the product information architecture.
 * The sidebar and navbar page titles are both derived from this config.
 * To add a top-level page, add an entry to the appropriate group.
 *
 * Grouping rationale (the console is agentic-first, so it opens on the
 * AI-first Home and the conversations sit right under it, not in a section):
 * - Home: where work starts — the Copilot composer, and what needs you.
 *   It is the one place a conversation is started; there is no separate
 *   "Ask AI" entry, which used to open a second, near-identical composer.
 * - History: the conversations already had — find, reopen, rename, delete.
 *   A conversation itself is `/copilot?id=…`, reached from Home or here.
 * - Dashboards: always top level, right under History — it is where most
 *   people spend their time, not a sub-page of Data.
 * - Data: where data lives and how it is read (catalog, queries).
 * - Build: author and operate data movement and outbound exports.
 * - Governance: the rules applied to the data (policies, quality) and the
 *   human approval queue. Lineage is still a page, but it is reached from a
 *   table, chart, pipeline or run rather than the sidebar (`WEB-3` D8).
 * - Monitoring: whether the platform is healthy, and what happened — the
 *   old "Overview" dashboard lives here as Health, next to alerts and the
 *   audit trail, instead of being the landing page.
 * - Administration: identity and workspace configuration. Sign-in (SSO)
 *   lives inside Settings (`WEB-3` D6).
 */
export const NAV_GROUPS: NavGroup[] = [
  {
    label: "Home",
    icon: Home,
    items: [{ title: "Home", href: "/", icon: Home }],
  },
  {
    label: "History",
    icon: History,
    items: [{ title: "History", href: "/copilot/history", icon: History }],
  },
  {
    label: "Dashboards",
    icon: BarChart3,
    items: [
      {
        title: "Dashboards",
        href: "/dashboards",
        icon: BarChart3,
        // The row opens the dashboard you last had open (`/dashboards` is
        // the resolver); the icon opens the list, where dashboards and
        // their folders are managed.
        action: { title: "Browse all dashboards", href: "/dashboards/browse", icon: LayoutGrid },
      },
    ],
  },
  {
    label: "Data",
    icon: Database,
    defaultOpen: true,
    items: [
      { title: "Catalog", href: "/catalog", icon: Library },
      { title: "Query Studio", href: "/query-studio", icon: SearchCode },
      { title: "Sources", href: "/connectors", icon: Plug },
    ],
  },
  {
    label: "Build",
    icon: GitBranch,
    items: [
      { title: "Pipelines", href: "/pipelines", icon: GitBranch },
    ],
  },
  {
    label: "Governance",
    icon: ShieldCheck,
    items: [
      // WEB-3 S3-D3, S3-D6: a column's sensitivity and masks are set on the
      // column (a table's Schema tab), and Tag keys is a section of Settings;
      // their old addresses redirect. S3-D4: Policies is "Access rules", at
      // the same address.
      { title: "Access rules", href: "/governance/policies", icon: ShieldCheck },
      {
        title: "Data Quality",
        href: "/governance/data-quality",
        icon: ListChecks,
      },
      // Copilot's destructive tools and catalog access requests queue here.
      { title: "Approvals", href: "/governance/approvals", icon: ClipboardCheck },
    ],
  },
  {
    label: "Monitoring",
    icon: HeartPulse,
    items: [
      // WEB-3 S2-D2: Services, Capacity, Ingestion (CDC) and Table
      // Maintenance are sections of System status; their old addresses
      // redirect to `/health#<section>`.
      { title: "System status", href: "/health", icon: HeartPulse },
      { title: "Alerts", href: "/alerts", icon: BellRing },
      { title: "Audit Log", href: "/audit", icon: FileText },
    ],
  },
  {
    label: "Administration",
    icon: Settings,
    items: [
      { title: "Users", href: "/admin/users", icon: Users },
      // WEB-3 S3-D8: the page holds roles only, so it is named for them.
      { title: "Roles", href: "/admin/roles", icon: Building2 },
      { title: "Tenants", href: "/admin/tenants", icon: Boxes },
      {
        title: "Service Identities",
        href: "/admin/service-identities",
        icon: KeyRound,
      },
      // WEB-3 S3-D6, S3-D7: Sessions is a tab of Users and Maps a section
      // of Settings; their old addresses redirect.
      { title: "Settings", href: "/admin/settings", icon: SlidersHorizontal },
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
  { title: "All Dashboards", href: "/dashboards/browse" },
  // One conversation with Copilot. Not a sidebar entry: it is opened from
  // Home (a new one) or from History (an old one).
  { title: "Conversation", href: "/copilot" },
  // Table detail pages, opened from System status's maintenance runs or
  // an asset's Snapshots tab. Their nav entry was removed, so without this the
  // title would fall back to the product name.
  { title: "Iceberg Table", href: "/lakehouse/tables" },
]

/**
 * Routes whose sidebar parent is not a prefix of their path. An open
 * conversation (`/copilot`) belongs under History (`/copilot/history`),
 * which the longest-prefix rule alone cannot see.
 */
const HIGHLIGHT_UNDER: { href: string; item: string }[] = [
  { href: "/copilot", item: "/copilot/history" },
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
  return (
    bestMatch(pathname, ALL_NAV_ITEMS)?.href ??
    bestMatch(pathname, HIGHLIGHT_UNDER)?.item
  )
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
