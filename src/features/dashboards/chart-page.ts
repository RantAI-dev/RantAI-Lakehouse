/**
 * Where a chart is built and edited, and the one handoff a draft needs
 * (`WEB-7` D6).
 *
 * Chart building is a page now (`/dashboards/<board>/charts/new` and
 * `.../<chartId>/edit`), so two places have to agree on its address: the
 * dashboard's own buttons and the assistant's draft card. The address also
 * carries the dashboard's parameter values, which the builder draws its
 * preview with (BI-11); those are short, and they belong to the dashboard
 * the person is looking at, not to the chart, so the query string is the
 * right place for them. They are never saved with the chart: the builder
 * only puts them in the preview request.
 *
 * A chart the assistant drafted travels the other way: the card writes the
 * draft to `sessionStorage` under `lakehouse.chartDraft` and navigates to
 * the New chart page, which takes it once (`takeChartDraft`) and then
 * deletes it, so a draft is never applied twice. A draft is a chart
 * definition, not a credential. A draft the chat made also names the
 * conversation it came from, so the page can mark that conversation's step
 * saved and no other (`ChartDraftStep`).
 */

import type { ParamValue } from "@/lib/params"

export const CHART_DRAFT_KEY = "lakehouse.chartDraft"

/**
 * Whether the caller may edit the built-in dashboard (`WEB-7` D6).
 *
 * `GET /api/dashboard/access` refuses the built-in board: it is not a stored
 * row (`managed_subject`, `BUILT_IN`). The server judges it at the root,
 * where only the starting point can give a level (`AccessMap::level_at_root`):
 * `identity:write` is manage everywhere, `dashboard:write` is manage, and
 * `dashboard:read` alone is view. Both grants are read through the console's
 * wildcard-aware `hasPermission`, which mirrors the server's `PermissionSet`.
 */
export function builtInBoardEditable(hasPermission: (permission: string) => boolean): boolean {
  return hasPermission("identity:write") || hasPermission("dashboard:write");
}

/** The built-in dashboard's id: `default`, or blank (as `/api/dashboard` reads it). */
export function isBuiltInBoard(boardId: string): boolean {
  const id = boardId.trim();
  return id === "" || id === "default";
}

/** The query string the builder reads its preview parameters from. */
const PARAMS_KEY = "params"

function encodedParams(params: Record<string, ParamValue> | undefined): string {
  if (!params || Object.keys(params).length === 0) return "";
  return `?${PARAMS_KEY}=${encodeURIComponent(JSON.stringify(params))}`;
}

/** `/dashboards/<board>/charts/new`. */
export function chartNewHref(
  board: string,
  params?: Record<string, ParamValue>
): string {
  return `/dashboards/${encodeURIComponent(board)}/charts/new${encodedParams(params)}`;
}

/** `/dashboards/<board>/charts/<chartId>/edit`. */
export function chartEditHref(
  board: string,
  chartId: string,
  params?: Record<string, ParamValue>
): string {
  return `/dashboards/${encodeURIComponent(board)}/charts/${encodeURIComponent(chartId)}/edit${encodedParams(params)}`;
}

/**
 * The parameter values a chart page's address carries, or `undefined` when
 * it carries none. A malformed entry is ignored rather than thrown: the
 * address is the person's, and a half-copied one must still open the form.
 */
export function chartParamValues(search: string): Record<string, ParamValue> | undefined {
  const raw = new URLSearchParams(search).get(PARAMS_KEY)
  if (!raw) return undefined
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return undefined
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return undefined
  const values: Record<string, ParamValue> = {}
  for (const [name, value] of Object.entries(parsed)) {
    if (typeof value === "string" || typeof value === "number") values[name] = value
  }
  return Object.keys(values).length > 0 ? values : undefined
}

/**
 * Where the assistant's tool step sat, and which conversation it sat in
 * (`WEB-7` S2 review round 1, S1).
 *
 * A draft outlives the chat that made it by one navigation, and the dock may
 * show another conversation by the time the chart is saved. Two indexes alone
 * would then name whatever sits there in the conversation that is open now,
 * so the draft carries the conversation too and the page checks it first.
 */
export type ChartDraftStep = {
  messageIndex: number
  stepIndex: number
  /**
   * The conversation the draft came from (`useCopilot().sessionId`), or
   * `null` for one that was not saved yet. A null id identifies nothing, so
   * the page falls back to recognising the step itself (`tool`).
   */
  sessionId: string | null
  /** The tool the step ran (`create_chart`), so an unsaved conversation's step can be recognised. */
  tool: string
}

/** A chart definition the card leaves for the New chart page. */
export type ChartDraft = {
  /** The dashboard the chart is being built for: the page that takes it. */
  board: string
  /** The definition, in the builder's own shape (`ChartDef`), as JSON. */
  def: Record<string, unknown>
  /**
   * The assistant's tool step the draft came from, when it came from the
   * chat: the page marks it saved once the chart is written, so the chat
   * does not offer to add the same chart again. Absent for a draft a person
   * started themselves.
   */
  complete?: ChartDraftStep
}

/** Store a draft for the New chart page, or do nothing when storage refuses. */
export function writeChartDraft(draft: ChartDraft): void {
  try {
    window.sessionStorage.setItem(CHART_DRAFT_KEY, JSON.stringify(draft))
  } catch {
    // A draft that cannot be stored is a lost convenience: the page opens
    // empty, which is honest, and the chat still holds the card.
  }
}

/**
 * Take the draft left for `board`, once.
 *
 * The key is deleted whatever it held: a draft for another board is not
 * this page's to fill in, and keeping it would surprise the next visit.
 * `null` when there was nothing, or nothing this page can use.
 */
export function takeChartDraft(board: string): ChartDraft | null {
  let raw: string | null = null
  try {
    raw = window.sessionStorage.getItem(CHART_DRAFT_KEY)
    window.sessionStorage.removeItem(CHART_DRAFT_KEY)
  } catch {
    return null
  }
  if (!raw) return null
  let parsed: unknown
  try {
    parsed = JSON.parse(raw)
  } catch {
    return null
  }
  if (typeof parsed !== "object" || parsed === null) return null
  const draft = parsed as Partial<ChartDraft>
  if (draft.board !== board) return null
  if (typeof draft.def !== "object" || draft.def === null || Array.isArray(draft.def)) return null
  const complete = draft.complete
  return {
    board,
    def: draft.def as Record<string, unknown>,
    ...(isStepAddress(complete) && { complete }),
  }
}

function isStepAddress(value: unknown): value is ChartDraftStep {
  if (typeof value !== "object" || value === null) return false
  const address = value as Partial<ChartDraftStep>
  // `WEB-7` S2 review round 1, S1: a draft with no conversation recorded (one
  // an older build wrote) cannot say where its step is, so it carries no
  // address and the page marks nothing. `null` is a conversation that was not
  // saved yet, and is answered for by the step's own tool.
  const namedConversation = address.sessionId === null || typeof address.sessionId === "string"
  return (
    Number.isInteger(address.messageIndex) &&
    Number.isInteger(address.stepIndex) &&
    (address.messageIndex ?? -1) >= 0 &&
    (address.stepIndex ?? -1) >= 0 &&
    namedConversation &&
    typeof address.tool === "string" &&
    address.tool.trim() !== ""
  )
}
