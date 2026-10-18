/**
 * Which confirmed assistant tools change a dashboard the open page may be
 * showing (BI-18 part C review fix (SHOULD-FIX) R23). The page reloads on
 * `notifyDashboardsChanged`, but the assistant only raised it for
 * `create_chart`, so a tab, layout, format or theme change showed after leaving
 * the dashboard and coming back.
 */
const BOARD_TOOLS: ReadonlySet<string> = new Set([
  "create_tab", "rename_tab", "move_charts_to_tab", "move_tab", "delete_tab",
  "update_chart", "format_chart", "delete_chart", "move_chart",
  "arrange_board", "rename_board", "delete_board", "save_board_filters", "set_dashboard_theme",
  "create_board", "create_chart",
])

/** Whether a tool step that ran changed a board: it succeeded and its result is not an error or a refusal. */
export function changedABoard(step: { tool?: string; ok?: boolean; result?: unknown }): boolean {
  if (!step.tool || !BOARD_TOOLS.has(step.tool) || step.ok === false) return false
  const result = step.result
  if (result && typeof result === "object") {
    const r = result as Record<string, unknown>
    if (r.error || r.cancelled || r.needsApproval || r.pending) return false
  }
  return true
}
