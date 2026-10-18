/**
 * Scheduled reports by email (`RPT-1`, with `AI-10`): a dashboard sent on a
 * schedule as a PDF and chosen charts' data as Excel or CSV. Mirrors
 * `rust/crates/lakehouse-api/src/routes/reports.rs` and the store module
 * `lakehouse_store::report_subscriptions`.
 *
 * A report is made as the person who set it up: their masking and row filters,
 * and every recipient sees what that person sees (feature decision 1).
 */

export type ReportSchedule =
  | { kind: "hourly"; minute: number }
  | { kind: "daily"; hour: number; minute: number }
  | { kind: "weekly"; weekday: number; hour: number; minute: number }
  /** `monthDay` is 1 to 28, or `"last"`. */
  | { kind: "monthly"; monthDay: number | "last"; hour: number; minute: number }

export type ReportPdf = { paper: "a4" | "letter" | string; orientation: "portrait" | "landscape" | string }
export type ReportChartFile = { chart: string; format: "xlsx" | "csv" | string; values?: "formatted" | "raw" | string }
export type ReportAttachments = { pdf?: ReportPdf | null; charts: ReportChartFile[] }

/** A recipient on the wire: a console user by id, or an outside address. */
export type ReportRecipientInput = { userId: string } | { email: string }

/** What a create or an update sends (an update leaves `board` out or repeats it). */
export type ReportSubscriptionInput = {
  board?: string
  schedule: ReportSchedule
  timeZone?: string
  attachments: ReportAttachments
  /** The dashboard's filters (a list), as the page sends them. */
  filters?: unknown[] | null
  /** Parameter values (an object). */
  params?: Record<string, unknown> | null
  grain?: string | null
  skipWhenEmpty: boolean
  recipients: ReportRecipientInput[]
  /** An update may pause or resume at the same time. */
  paused?: boolean
}

export type ReportRecipient = {
  userId?: string
  name?: string | null
  email: string | null
  /** For a console user: whether they are active now. */
  active?: boolean
}

export type ReportDelivery = {
  id: string
  startedAt: string
  finishedAt: string
  status: "sent" | "failed" | "skipped" | string
  origin: "schedule" | "manual" | string
  recipientCount: number
  /** A plain sentence the product wrote; never the mail server's or the PDF service's text. */
  reason: string | null
}

export type ReportSubscription = {
  id: string
  board: string
  ownerId: string
  schedule: ReportSchedule | null
  timeZone: string
  attachments: ReportAttachments
  filters: unknown[] | null
  params: Record<string, unknown> | null
  grain: string | null
  skipWhenEmpty: boolean
  paused: boolean
  pausedReason: string | null
  consecutiveFailures: number
  nextRunAt: string
  createdAt: string
  recipients: ReportRecipient[]
  lastDelivery?: ReportDelivery | null
  /** The caller owns it or is an administrator. */
  canEdit?: boolean
}

export type ReportList = {
  items: ReportSubscription[]
  /** False without `SMTP_HOST`: a run records "Email is not set up on this deployment". */
  emailSetUp: boolean
  /** False without the PDF service: a PDF cannot be attached. */
  pdfSupported: boolean
}

export type ReportPerson = { id: string; name: string; email: string }

export interface ReportService {
  list(board: string, signal?: AbortSignal): Promise<ReportList>
  /** A refusal (an outside domain not listed, 51 recipients) arrives as `invalid_request` with the server's sentence. */
  create(input: ReportSubscriptionInput & { board: string }, signal?: AbortSignal): Promise<ReportSubscription>
  update(id: string, input: ReportSubscriptionInput, signal?: AbortSignal): Promise<ReportSubscription>
  remove(id: string, signal?: AbortSignal): Promise<void>
  /** Sends the report now, as a test; the delivery says `failed` and why when it could not. */
  sendNow(id: string, signal?: AbortSignal): Promise<ReportDelivery>
  deliveries(id: string, signal?: AbortSignal): Promise<ReportDelivery[]>
  /** Active people for the recipient picker: at most 20. */
  people(query: string, signal?: AbortSignal): Promise<ReportPerson[]>
}
