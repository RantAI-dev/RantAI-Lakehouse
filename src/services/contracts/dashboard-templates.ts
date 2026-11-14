/**
 * Dashboard templates (`BI-18` part D, feature decisions T1 to T4): a saved
 * copy of a dashboard's definition (layout, charts, filters, text tiles; no
 * data, no snapshots) to start new dashboards from. Mirrors
 * `rust/crates/lakehouse-api/src/routes/dashboard_templates.rs` and the store
 * module `lakehouse_store::dashboard_templates`. Every route needs
 * `dashboard:write`.
 */
import type { Board } from "./dashboards"

export type DashboardTemplate = {
  id: string
  name: string
  /** The dashboard it was saved from; it may be gone since. */
  sourceBoard: string
  sourceBoardName: string
  chartCount: number
  createdBy: string
  /** RFC 3339. */
  createdAt: string
}

export type TemplateList = { templates: DashboardTemplate[]; truncated: boolean }

/** A chart the new dashboard does not have, with the server's reason. */
export type LeftOutChart = { title: string; reason: string }

export type StartedBoard = { board: Board; leftOut: LeftOutChart[] }

export interface DashboardTemplateService {
  listTemplates(signal?: AbortSignal): Promise<TemplateList>
  saveTemplate(input: { board: string; name: string }, signal?: AbortSignal): Promise<DashboardTemplate>
  /** A new, independent dashboard made from the template, in `folderId` (`""` = no folder). */
  useTemplate(id: string, input: { name: string; folderId: string }, signal?: AbortSignal): Promise<StartedBoard>
  deleteTemplate(id: string, signal?: AbortSignal): Promise<void>
}
