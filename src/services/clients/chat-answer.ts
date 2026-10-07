import type { NlAnswer, NlAnswerStep, QueryCell } from "../contracts/queries"

type TraceEntry = { tool?: unknown; args?: unknown; ok?: unknown; result?: unknown }

type RunSqlResult = {
  columns?: { name?: unknown }[]
  rows?: Record<string, QueryCell>[]
  rowCount?: number
  error?: unknown
}

/** `args.sql` of a `run_sql` entry, or "" when the model sent none. */
function sqlOf(entry: TraceEntry): string {
  const sql = (entry.args as { sql?: unknown } | null | undefined)?.sql
  return typeof sql === "string" ? sql : ""
}

function stepOf(entry: TraceEntry): NlAnswerStep {
  const step = typeof entry.tool === "string" ? entry.tool : ""
  if (step !== "run_sql") return { step, detail: "" }
  if (entry.ok === true) return { step, detail: sqlOf(entry) }
  const error = (entry.result as RunSqlResult | null | undefined)?.error
  return { step, detail: typeof error === "string" ? error : "" }
}

/**
 * Reads a `POST /api/ai/chat` response (no `stream`) as the box's answer.
 *
 * The SQL and rows come from the LAST successful `run_sql`: when the model
 * corrects itself, the final query is the one that answers the question,
 * and it is the one the editor should receive.
 */
export function chatAnswerFromBody(body: unknown): NlAnswer {
  const { answer, toolTrace } = (body ?? {}) as { answer?: unknown; toolTrace?: unknown }
  const trace = (Array.isArray(toolTrace) ? toolTrace : []) as TraceEntry[]
  const lastOk = trace.findLast((e) => e.tool === "run_sql" && e.ok === true)
  const result = lastOk?.result as RunSqlResult | undefined
  const rows = result?.rows ?? []
  return {
    answer: typeof answer === "string" ? answer : "",
    sql: lastOk ? sqlOf(lastOk) : undefined,
    columns: (result?.columns ?? []).map((c) => String(c.name ?? "")),
    rows,
    rowCount: result?.rowCount ?? rows.length,
    steps: trace.map(stepOf),
  }
}
