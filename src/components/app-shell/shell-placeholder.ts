/**
 * Which page-shaped placeholder the loading shell (`AppShellSkeleton`) draws
 * in its main area while `AuthProvider` checks the session.
 *
 * The rule (owner QA 2026-11-14, "kenapa kita punya dua skeleton page?"): a
 * route that has its own loading placeholder gets that same component here,
 * so a refresh shows one continuous loading state: the page, once mounted,
 * keeps drawing the same shapes until its data lands. A route without one
 * gets the generic list skeleton, as before.
 *
 * - `dashboard-list`: `/dashboards/browse`, the Dashboards page
 *   (`DashboardListPageSkeleton`).
 * - `dashboard`: one dashboard, `/dashboards/<id>`, and `/dashboards` itself,
 *   which forwards to the board last opened and draws the dashboard's
 *   placeholder while it decides (`DashboardPageSkeleton`).
 * - `page`: everything else, including the dashboard routes that are not a
 *   board (`/dashboards/metrics`, `/dashboards/reports/…`, a chart's builder
 *   under `/dashboards/<id>/charts/…`).
 */
export type ShellPlaceholder = "dashboard-list" | "dashboard" | "page"

/** The `/dashboards/<segment>` routes that are pages of their own, not a board id. */
const DASHBOARD_PAGES: ReadonlySet<string> = new Set(["browse", "metrics", "reports"])

export function shellPlaceholderFor(pathname: string): ShellPlaceholder {
  const path = pathname.split(/[?#]/)[0].replace(/\/+$/, "") || "/"
  if (path === "/dashboards/browse") return "dashboard-list"
  if (path === "/dashboards") return "dashboard"
  const board = /^\/dashboards\/([^/]+)$/.exec(path)?.[1]
  if (board !== undefined && !DASHBOARD_PAGES.has(board)) return "dashboard"
  return "page"
}

/**
 * Which of the sidebar's two views the loading shell draws (`WEB-18` T5):
 * the chat list on a conversation (`/copilot`), where the real sidebar lists
 * the chats instead of the menu, and the menu everywhere else.
 */
export type SidebarPlaceholder = "menu" | "chats"

export function sidebarPlaceholderFor(pathname: string): SidebarPlaceholder {
  const path = pathname.split(/[?#]/)[0].replace(/\/+$/, "") || "/"
  return path === "/copilot" ? "chats" : "menu"
}
