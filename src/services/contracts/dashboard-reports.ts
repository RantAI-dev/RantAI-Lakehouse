/**
 * Reports: a written page of text blocks with live charts between the
 * paragraphs (`BI-18` part D, feature decisions R1 to R4). Mirrors
 * `rust/crates/lakehouse-api/src/routes/dashboard_reports.rs`, the block
 * shapes of `lakehouse_bi::reports::Block` and the store module
 * `lakehouse_store::dashboard_reports`.
 *
 * Text is plain text: no block carries HTML, and the console renders every
 * string as text. A chart block is a reference to a chart on a dashboard; the
 * server reads it as the person reading the report (their masks and row
 * filters), and a chart they may not see comes back as `unavailable` with a
 * fixed sentence and nothing else.
 */
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { ReportingContext } from "@/lib/time-grain"
import type { AccessLevel, TileFailure } from "./dashboards"

/** One block of a report (`lakehouse_bi::reports::Block`). */
export type ReportBlock =
  | { type: "heading"; text: string; level: 1 | 2 | 3 }
  | { type: "paragraph"; text: string }
  | { type: "list"; items: string[]; ordered?: boolean }
  | { type: "link"; text: string; url: string }
  /** `board` is filled by the server from the chart's own row. */
  | { type: "chart"; chart: string; board?: string }

export type ReportBlockType = ReportBlock["type"]

/** A report row in the list. */
export type ReportSummary = {
  id: string
  title: string
  /** The dashboard folder; `""` is the root. */
  folderId: string
  createdBy: string
  updatedBy: string
  /** RFC 3339. */
  updatedAt: string
  blockCount: number
  chartCount: number
}

export type ReportList = {
  reports: ReportSummary[]
  /** More reports exist than the list holds (it reads at most 1,000). */
  truncated: boolean
  /** The most chart blocks a report may hold (decision R4). */
  maxCharts: number
}

/** A report, blocks and all. */
export type Report = {
  id: string
  title: string
  folderId: string
  blocks: ReportBlock[]
  createdBy: string
  createdAt: string
  updatedBy: string
  updatedAt: string
}

/** One chart block as read for this person. */
export type ReportChartEntry =
  | {
      spec: ChartRenderSpec & { text?: string; caption?: string }
      cell: { columns: string[]; rows: Record<string, unknown>[] } | TileFailure | null
      board: string
      boardName: string
      unavailable?: undefined
    }
  /** The fixed sentence: the chart is not one this person may see, or could not be read. */
  | { unavailable: string }

/** What `GET /api/dashboard/reports/{id}` answers. */
export type ReportView = {
  report: Report
  /** Keyed by chart id. */
  charts: Record<string, ReportChartEntry>
  reporting?: ReportingContext | null
  access: { level: AccessLevel | string }
  maxCharts: number
  pdfSupported: boolean
  canDownload: boolean
}

export type SaveReportInput = {
  title: string
  folderId?: string
  blocks: ReportBlock[]
}

export type UpdateReportInput = {
  title?: string
  folderId?: string
  blocks?: ReportBlock[]
}

export interface DashboardReportService {
  listReports(signal?: AbortSignal): Promise<ReportList>
  getReport(id: string, signal?: AbortSignal): Promise<ReportView>
  createReport(input: SaveReportInput, signal?: AbortSignal): Promise<Report>
  updateReport(id: string, input: UpdateReportInput, signal?: AbortSignal): Promise<Report>
  /** To the trash, restorable for 30 days. */
  deleteReport(id: string, signal?: AbortSignal): Promise<void>
}
