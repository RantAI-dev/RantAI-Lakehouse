/**
 * What the report editor tells the assistant (`BI-62` PR B, decision D8):
 * the report as it stands, as Markdown, and, for "Describe", the chosen
 * chart with the rows the reader sees. Richer than the reader page's
 * context (`buildReportPageContext`), because the assistant writes into this
 * page.
 *
 * Nothing here fetches: the rows are the ones the page already drew the
 * chart with (`ReportChartsProvider`), summarised by `summarizeRows`; when
 * they are not loaded yet the context says so instead of guessing.
 *
 * The server keeps the first `PAGE_CONTEXT_MAX_CHARS` characters and drops
 * the rest, so the whole text is fitted here: the instructions and the
 * described chart come first and are never cut, and the report's Markdown
 * takes the room that is left, cut at the end with a note.
 */
import type { ReportBlock, ReportChartEntry } from "@/services/contracts/dashboard-reports"
import { MAX_REPORT_CHARTS, chartCount } from "./report-blocks"
import { blocksToMarkdown, type ChartLabel } from "./report-markdown"
import { PAGE_CONTEXT_MAX_CHARS, summarizeRows } from "./page-context-summary"

/** The rows of the chart being described, as far as the page has them. */
export type DescribedRows =
  | { kind: "rows"; columns: string[]; rows: Record<string, unknown>[] }
  /** The page has not drawn the chart yet. */
  | { kind: "not loaded" }
  /** The chart could not be read, or this person may not see it. */
  | { kind: "not readable" }

/**
 * The rows the page drew a chart with: its report read or its dashboard's
 * payload (`ReportChartsProvider`). No entry, or one without a result yet, is
 * "not loaded"; an entry the person may not see, or a failed tile, is "not
 * readable" (its error text is never sent).
 */
export function describedRows(entry: ReportChartEntry | undefined): DescribedRows {
  if (!entry) return { kind: "not loaded" }
  if (!("spec" in entry)) return { kind: "not readable" }
  if (!entry.cell) return { kind: "not loaded" }
  if ("error" in entry.cell) return { kind: "not readable" }
  return { kind: "rows", columns: entry.cell.columns, rows: entry.cell.rows }
}

export type DescribedChart = { id: string; title: string; dashboard: string; rows: DescribedRows }

/** How many of the chart's rows are sent; the rest are counted. */
export const DESCRIBE_MAX_ROWS = 50
/** The described chart's own room, so a wide table cannot crowd out the instructions. */
export const DESCRIBE_MAX_CHARS = 2500

/** The first words of a draft request from the page; the context tells the model what they mean. */
export const WRITE_PREFIX = "Write for this report:"

/** The request "Describe" sends for a chart. */
export function describeRequest(title: string): string {
  return `Describe the chart “${title.replace(/\s+/g, " ").trim() || "Untitled chart"}” for this report`
}

/** The request "Ask AI to write…" sends for what the person typed. */
export function writeRequest(prompt: string): string {
  return `${WRITE_PREFIX} ${prompt.replace(/\s+/g, " ").trim()}`
}

const REST_NOTE = "\n… (the rest of the report is not shown)"

function clip(text: string, max: number): string {
  return text.length > max ? `${text.slice(0, max - 13)}… (truncated)` : text
}

function chartPart(chart: DescribedChart): string {
  const name = `"${chart.title.slice(0, 200)}" (chart id ${chart.id})${chart.dashboard ? ` from the dashboard "${chart.dashboard.slice(0, 200)}"` : ""}`
  let rows: string
  if (chart.rows.kind === "rows") rows = `Its rows as the reader sees them: ${summarizeRows(chart.rows.columns, chart.rows.rows, DESCRIBE_MAX_ROWS)}.`
  else if (chart.rows.kind === "not loaded") rows = "Its rows are not loaded on the page yet (rows not loaded): read them with a tool before giving any number."
  else rows = "Its rows could not be read for this person: give no numbers for it."
  return clip(`The chart to describe: ${name}. ${rows}`, DESCRIBE_MAX_CHARS)
}

export type ReportEditorContextInput = {
  /** The report's id once it is saved; null for a new one. */
  id: string | null
  title: string
  blocks: readonly ReportBlock[]
  chartLabel: (id: string) => ChartLabel | undefined
  /** The chart "Describe" was asked for, if any. */
  chart?: DescribedChart | null
}

/** The editor's page context for the assistant, within `PAGE_CONTEXT_MAX_CHARS`. */
export function buildReportEditorContext(input: ReportEditorContextInput): string {
  const title = JSON.stringify(input.title.slice(0, 200) || "Untitled report")
  const head = [
    `The user is writing the report ${title} in the report editor (${input.id ? `report id ${input.id}` : "not saved yet"}, ${chartCount(input.blocks)} of ${MAX_REPORT_CHARTS} charts).`,
    "The editor holds this report while it is open and saves it itself: do not change it with update_report; answer with text instead.",
    `A message that starts "${WRITE_PREFIX}" or asks to describe a chart "for this report" is placed in the report as a draft the user accepts or discards:`,
    "answer it with only the text to add, in the Markdown a report holds (paragraphs, # and ## headings, - and 1. lists, > quotes, ---, **bold**, *italic*, [words](https://…) links),",
    "with no preamble, no closing question, no tables and no code. Use only numbers from the rows given here or from your tools.",
    ...(input.chart ? [chartPart(input.chart)] : []),
    "The report so far, as Markdown:",
  ].join(" ")
  const markdown = blocksToMarkdown(input.blocks, input.chartLabel) || "(empty)"
  const room = PAGE_CONTEXT_MAX_CHARS - head.length - 1
  const body = markdown.length <= room ? markdown : `${markdown.slice(0, Math.max(0, room - REST_NOTE.length))}${REST_NOTE}`
  return `${head}\n${body}`
}
