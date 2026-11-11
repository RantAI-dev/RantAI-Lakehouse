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
  /** Permissions required to open this nav entry (any-of). Empty means any authenticated user. */
  requires: readonly string[]
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
    items: [
      {
        title: "Home",
        href: "/",
        icon: Home,
        requires: ["assistant:use", "assistant:ask"],
      },
    ],
  },
  {
    label: "History",
    icon: History,
    items: [
      {
        title: "History",
        href: "/copilot/history",
        icon: History,
        requires: ["assistant:use", "assistant:ask"],
      },
    ],
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
        requires: ["dashboard:read"],
      },
    ],
  },
  {
    label: "Data",
    icon: Database,
    defaultOpen: true,
    items: [
      { title: "Catalog", href: "/catalog", icon: Library, requires: ["catalog:read"] },
      { title: "Query Studio", href: "/query-studio", icon: SearchCode, requires: ["query:read"] },
      { title: "Sources", href: "/connectors", icon: Plug, requires: ["connector:manage"] },
    ],
  },
  {
    label: "Build",
    icon: GitBranch,
    items: [
      { title: "Pipelines", href: "/pipelines", icon: GitBranch, requires: ["pipeline:read"] },
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
      // ADM-13 review round 1, S4: policy:read only
      { title: "Access rules", href: "/governance/policies", icon: ShieldCheck, requires: ["policy:read"] },
      {
        title: "Data Quality",
        href: "/governance/data-quality",
        icon: ListChecks,
        requires: ["governance:read"],
      },
      // Copilot's destructive tools and catalog access requests queue here.
      { title: "Approvals", href: "/governance/approvals", icon: ClipboardCheck, requires: ["access:approve", "agent:approve"] },
    ],
  },
  {
    label: "Monitoring",
    icon: HeartPulse,
    items: [
      // WEB-3 S2-D2: Services, Capacity, Ingestion (CDC) and Table
      // Maintenance are sections of System status; their old addresses
      // redirect to `/health#<section>`.
      { title: "System status", href: "/health", icon: HeartPulse, requires: ["ops:read"] },
      { title: "Alerts", href: "/alerts", icon: BellRing, requires: ["alert:read", "alert:write"] },
      // ADM-13 review round 1, S4: governance:read only
      { title: "Audit Log", href: "/audit", icon: FileText, requires: ["governance:read"] },
    ],
  },
  {
    label: "Administration",
    icon: Settings,
    items: [
      { title: "Users", href: "/admin/users", icon: Users, requires: ["identity:read"] },
      // WEB-3 S3-D8: the page holds roles only, so it is named for them.
      { title: "Roles", href: "/admin/roles", icon: Building2, requires: ["identity:read"] },
      { title: "Tenants", href: "/admin/tenants", icon: Boxes, requires: ["identity:read"] },
      {
        title: "Service Identities",
        href: "/admin/service-identities",
        icon: KeyRound,
        requires: ["identity:read"],
      },
      // WEB-3 S3-D6, S3-D7: Sessions is a tab of Users and Maps a section
      // of Settings; their old addresses redirect.
      { title: "Settings", href: "/admin/settings", icon: SlidersHorizontal, requires: ["settings:write", "identity:read"] },
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

export type RouteAccessEntry = {
  pattern: string
  requires: readonly string[]
}

/**
 * Access map covering every non-public, non-redirect page in src/app.
 * Gated by requires (any-of permissions). An empty array means any signed-in user.
 * Verified against src/app/** /page.tsx and permissions.json by nav-config.test.ts.
 */
export const ROUTE_ACCESS: readonly RouteAccessEntry[] = [
  // Home and AI
  { pattern: "/", requires: ["assistant:use", "assistant:ask"] },
  { pattern: "/copilot", requires: ["assistant:use", "assistant:ask"] },
  { pattern: "/copilot/history", requires: ["assistant:use", "assistant:ask"] },

  // Dashboards
  { pattern: "/dashboards", requires: ["dashboard:read"] },
  { pattern: "/dashboards/browse", requires: ["dashboard:read"] },
  { pattern: "/dashboards/metrics", requires: ["dashboard:read"] },
  { pattern: "/dashboards/[id]", requires: ["dashboard:read"] },
  // WEB-7 D6: the chart builder's pages. `POST`/`PUT /api/dashboard/specs`
  // are `RequiresAuth` with the level resolved in the handler
  // (`policy.rs`), and a person can hold edit on one dashboard through a
  // rule without `dashboard:write`; so the entry opens to any signed-in
  // person and the page says "You don't have access" below edit.
  { pattern: "/dashboards/[id]/charts/new", requires: [] },
  { pattern: "/dashboards/[id]/charts/[chartId]/edit", requires: [] },

  // Data & Catalog
  { pattern: "/catalog", requires: ["catalog:read"] },
  { pattern: "/data/assets/[assetId]", requires: ["catalog:read"] },
  { pattern: "/lakehouse/tables/[namespace]/[table]", requires: ["catalog:read"] },
  // ADM-13 review round 1, S4: gold-exports requires [] (any signed-in user, RequiresAuth on GET)
  { pattern: "/gold-exports", requires: [] },

  // Query Studio
  { pattern: "/query-studio", requires: ["query:read"] },
  // WEB-7 D2/D5: the save forms left the editor. Both take the permission
  // their submit route takes: `POST /api/query/saved` is `query:read`
  // (`policy.rs:475`), and `POST /api/dashboard/sources` is `dashboard:sql`
  // (`policy.rs:680`), the same grant the editor's button is shown for.
  { pattern: "/query-studio/save", requires: ["query:read"] },
  { pattern: "/query-studio/save-source", requires: ["dashboard:sql"] },
  { pattern: "/query-studio/saved", requires: ["query:read"] },
  { pattern: "/query-studio/history", requires: ["query:read"] },

  // Sources / Connectors
  { pattern: "/connectors", requires: ["connector:manage"] },
  { pattern: "/connectors/create", requires: ["connector:manage"] },
  { pattern: "/connectors/upload", requires: ["connector:manage"] },
  { pattern: "/connectors/[id]", requires: ["connector:manage"] },
  { pattern: "/connectors/[id]/edit", requires: ["connector:manage"] },

  // Build / Pipelines
  { pattern: "/pipelines", requires: ["pipeline:read"] },
  // ADM-13 review round 1, S4: pipeline:write only
  { pattern: "/pipelines/create", requires: ["pipeline:write"] },
  { pattern: "/pipelines/[pipelineId]", requires: ["pipeline:read"] },
  { pattern: "/pipelines/[pipelineId]/edit", requires: ["pipeline:write"] },

  // Governance
  // ADM-13 review round 1, S4: policy:read only
  { pattern: "/governance/policies", requires: ["policy:read"] },
  { pattern: "/governance/data-quality", requires: ["governance:read"] },
  // WEB-7 D2: adding a rule is the `governance:write` the API takes
  // (`POST /api/governance/quality`).
  { pattern: "/governance/data-quality/rules/new", requires: ["governance:write"] },
  { pattern: "/governance/approvals", requires: ["access:approve", "agent:approve"] },
  // ADM-13 review round 1, S4: governance:read only
  { pattern: "/audit", requires: ["governance:read"] },
  { pattern: "/lineage", requires: ["lineage:read"] },

  // Monitoring
  { pattern: "/health", requires: ["ops:read"] },
  { pattern: "/alerts", requires: ["alert:read", "alert:write"] },

  // Intelligence / Agents
  // ADM-13 review round 1, S4: agent runs requires [] (any signed-in user, RequiresAuth on GET)
  { pattern: "/agents/runs/[runId]", requires: [] },

  // Administration
  { pattern: "/admin/users", requires: ["identity:read"] },
  // WEB-7 D2: the forms that left the side panel. Each takes the permission
  // its submit route takes (POST/PUT identity writes are `identity:write`,
  // `policy.rs`), so a person who may open the list but not write cannot
  // reach the form.
  { pattern: "/admin/users/invite", requires: ["identity:write"] },
  { pattern: "/admin/roles", requires: ["identity:read"] },
  { pattern: "/admin/roles/new", requires: ["identity:write"] },
  { pattern: "/admin/roles/[id]/edit", requires: ["identity:write"] },
  { pattern: "/admin/tenants", requires: ["identity:read"] },
  { pattern: "/admin/tenants/new", requires: ["identity:write"] },
  { pattern: "/admin/service-identities", requires: ["identity:read"] },
  { pattern: "/admin/service-identities/new", requires: ["identity:write"] },
  { pattern: "/admin/settings", requires: ["settings:write", "identity:read"] },
  // Tag keys are a Settings section; defining or changing one is the
  // governance write the API route takes (`POST`/`PUT /api/governance/tag-keys`).
  { pattern: "/admin/settings/tag-keys/new", requires: ["governance:write"] },
  { pattern: "/admin/settings/tag-keys/[key]/edit", requires: ["governance:write"] },
  { pattern: "/account/change-password", requires: [] },
]

/**
 * Checks if a route pattern matches a given runtime pathname.
 * Handles exact matches as well as Next.js dynamic segments like `[id]`.
 */
export function routeMatches(pattern: string, pathname: string): boolean {
  const normPath = pathname.split("?")[0].replace(/\/+$/, "") || "/"
  const normPattern = pattern.replace(/\/+$/, "") || "/"
  if (normPattern === normPath) return true
  const patternParts = normPattern.split("/")
  const pathParts = normPath.split("/")
  if (patternParts.length !== pathParts.length) return false
  return patternParts.every((part, i) => {
    if (part.startsWith("[") && part.endsWith("]")) {
      return pathParts[i].length > 0
    }
    return part === pathParts[i]
  })
}

/**
 * ADM-13 (D7): Fail-closed access check for pathnames against ROUTE_ACCESS.
 * Unlisted paths return false.
 * A path with empty `requires` is open to any authenticated user.
 * Otherwise, returns true if the user holds at least one required permission.
 */
export function canOpen(
  pathname: string,
  hasPermission: (perm: string) => boolean
): boolean {
  const normPath = pathname.split("?")[0].replace(/\/+$/, "") || "/"
  // Exact match wins over parameterized match
  const match =
    ROUTE_ACCESS.find((entry) => entry.pattern === normPath) ??
    ROUTE_ACCESS.find((entry) => routeMatches(entry.pattern, normPath))

  if (!match) {
    return false // Fail closed: unlisted routes return false
  }

  if (match.requires.length === 0) {
    return true // Any signed-in user
  }

  return match.requires.some((perm) => hasPermission(perm))
}

