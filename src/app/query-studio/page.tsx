"use client"

import type { ReactNode } from "react"
import { useCallback, useEffect, useRef, useState } from "react"
import Link from "next/link"
import {
  AlignLeft,
  ArrowLeft,
  ArrowUp,
  Bookmark,
  ChevronLeft,
  ChevronRight,
  CircleArrowUp,
  Coins,
  History,
  Loader2,
  Mic,
  Play,
  Sparkles,
  Table2,
  Terminal,
  Timer,
  Users2,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import {
  Sheet,
  SheetContent,
  SheetDescription,
  SheetHeader,
  SheetTitle,
  SheetTrigger,
} from "@/components/ui/sheet"
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs"
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table"
import { SqlEditor } from "@/components/sql-editor"
import { PromptActionsMenu } from "@/components/query-studio/prompt-actions-menu"
import {
  AGENT_MODEL_OPTIONS,
  type AgentModelId,
} from "@/lib/knowledge-library-fixtures"
import { cn } from "@/lib/utils"

const TAB_NL = "natural-language"
const TAB_SQL = "sql-editor"

const NL_OUTPUT_TAB_RESULT = "nl-output-result"
const NL_OUTPUT_TAB_SUMMARY = "nl-output-summary"

const NL_GENERATE_MS = 2000

const SQL_PLACEHOLDER = `-- Example: query the lakehouse
SELECT
  order_id,
  customer_id,
  amount
FROM gold.orders_fact
WHERE order_date >= CURRENT_DATE - INTERVAL '7' DAY
LIMIT 100;`

type Topic = { id: string; title: string; description: string }

const TOPIC_PAGES: Topic[][] = [
  [
    {
      id: "1",
      title: "Loan Trends",
      description:
        "Show approved loan trends (last 2 fiscal years) as an interactive line chart.",
    },
    {
      id: "2",
      title: "Customer Performance",
      description:
        "List top 5 customers (largest portfolio) who haven't borrowed in the last year. Include contact and portfolio value.",
    },
    {
      id: "3",
      title: "Account Balance",
      description:
        "What's the average priority savings balance by region last quarter?",
    },
    {
      id: "4",
      title: "Approval Efficiency",
      description:
        "Show average loan approval time per product type for the last 6 months.",
    },
  ],
  [
    {
      id: "5",
      title: "Branch throughput",
      description:
        "Compare monthly new accounts opened per branch for this year vs last year.",
    },
    {
      id: "6",
      title: "Risk flags",
      description:
        "Summarize high-risk accounts flagged in the last 30 days by product line.",
    },
    {
      id: "7",
      title: "Campaign ROI",
      description:
        "Which acquisition campaigns had the best funded-loan conversion in Q1?",
    },
    {
      id: "8",
      title: "Support load",
      description:
        "Average ticket resolution time for priority 1 cases, broken down by region.",
    },
  ],
]

type NlRunState = "idle" | "generating" | "ready"

type NlResultRow = { colA: string; colB: string; colC: string }

type SqlStreamRow = {
  order_id: string
  customer_id: string
  amount: string
  order_date: string
}

const SAMPLE_SQL_STREAM_ROWS: SqlStreamRow[] = [
  {
    order_id: "O-90012",
    customer_id: "C-1042",
    amount: "128.40",
    order_date: "2026-04-10",
  },
  {
    order_id: "O-90013",
    customer_id: "C-881",
    amount: "94.12",
    order_date: "2026-04-10",
  },
  {
    order_id: "O-90014",
    customer_id: "C-2201",
    amount: "210.00",
    order_date: "2026-04-09",
  },
  {
    order_id: "O-90015",
    customer_id: "C-1042",
    amount: "56.00",
    order_date: "2026-04-09",
  },
  {
    order_id: "O-90016",
    customer_id: "C-3309",
    amount: "412.75",
    order_date: "2026-04-08",
  },
  {
    order_id: "O-90017",
    customer_id: "C-441",
    amount: "18.99",
    order_date: "2026-04-08",
  },
  {
    order_id: "O-90018",
    customer_id: "C-552",
    amount: "89.50",
    order_date: "2026-04-07",
  },
  {
    order_id: "O-90019",
    customer_id: "C-1042",
    amount: "301.20",
    order_date: "2026-04-07",
  },
]

/**
 * Builds a fake "generated SQL" string from a natural-language question.
 *
 * Escapes single quotes by doubling them and truncates to 80 chars before
 * embedding into a `plainto_tsquery` predicate against `gold.analytics_v`.
 * No real LLM call — purely a UI preview.
 */
function buildNlSqlPreview(question: string): string {
  const safe = question.replace(/'/g, "''").slice(0, 80)
  return `SELECT *
FROM gold.analytics_v
WHERE search_vector @@ plainto_tsquery('english', '${safe}')
ORDER BY relevance DESC
LIMIT 500;`
}

/**
 * Returns a fixed three-row sample shown beneath the generated SQL.
 *
 * Currently ignores the question (kept as a parameter so a real implementation
 * can swap in question-aware sample data without changing call sites).
 */
function sampleNlResultRows(_question: string): NlResultRow[] {
  return [
    { colA: "Row 1", colB: "128.4", colC: "2026-04-01" },
    { colA: "Row 2", colB: "94.12", colC: "2026-04-02" },
    { colA: "Row 3", colB: "210.0", colC: "2026-04-03" },
  ]
}

/** Mock plain-language summary shown below the result grid. */
function buildNlNaturalExplanation(
  question: string,
  rows: NlResultRow[]
): string {
  const q =
    question.length > 160 ? `${question.slice(0, 157).trim()}…` : question.trim()
  if (rows.length === 0) {
    return (
      "This preview returned no rows. That often means filters were too strict, the date range had no " +
      "data, or the requested tables or metrics are not available. Try widening dates or confirming " +
      "the data you need exists in the lakehouse."
    )
  }
  const [a, b, c] = [rows[0], rows[1], rows[2]].filter(Boolean) as NlResultRow[]
  const parts: string[] = [
    `Here is a simple reading of the first ${rows.length} preview row${
      rows.length === 1 ? "" : "s"
    } for your question.`,
  ]
  if (a) {
    parts.push(
      `The top row shows “${a.colA}” with value ${a.colB} as of ${a.colC}.`
    )
  }
  if (b) {
    parts.push(
      `The next row is “${b.colA}” at ${b.colB} (${b.colC}), which helps compare scale within this sample.`
    )
  }
  if (c) {
    parts.push(
      `A third row—“${c.colA}” (${c.colB}, ${c.colC})—gives a sense of how values move in this slice.`
    )
  }
  parts.push(
    `This is only a **preview**; for full totals, breakdowns, and filters that fully match “${q}”, run against the warehouse or export the complete result set.`
  )
  return parts.join(" ")
}

type SqlPreRunEstimate = {
  estimatedBytes: number
  estimatedBytesDisplay: string
  partitionsTotal: number
  partitionsAssigned: number
  compileMsEstimate: number
  creditsPerHourWarehouse: number
  estimatedCreditsMin: number
  estimatedCreditsMax: number
  warehouseName: string
}

type SqlPostRunMetrics = {
  wallClockMs: number
  bytesScanned: number
  bytesScannedDisplay: string
  creditsAttributed: number
  rowsReturned: number
  percentOfExplainUpperBound: number
}

/** Formats a byte count with a single appropriate unit (B / KB / MB / GB). */
function formatBytesShort(n: number): string {
  if (n < 1024) return `${n} B`
  const kb = n / 1024
  if (kb < 1024) return `${kb < 10 ? kb.toFixed(1) : Math.round(kb)} KB`
  const mb = kb / 1024
  if (mb < 1024) return `${mb < 10 ? mb.toFixed(1) : Math.round(mb)} MB`
  const gb = mb / 1024
  return `${gb < 10 ? gb.toFixed(2) : gb.toFixed(1)} GB`
}

/**
 * Tiny deterministic string hash (variant of djb2). Used so identical SQL
 * always yields the same fake estimate, which keeps the explain card stable
 * between renders without storing real engine telemetry.
 */
function stringHash(s: string): number {
  let h = 0
  for (let i = 0; i < s.length; i++) {
    h = (h << 5) - h + s.charCodeAt(i)
    h |= 0
  }
  return Math.abs(h)
}

/**
 * Produces a mock "explain"-style pre-run estimate for a SQL string.
 *
 * Heuristics (no real planner is involved):
 * - Base bytes seeded from `stringHash(sql)`.
 * - Each `JOIN` increases the estimate by ~28%.
 * - A `LIMIT` clamps to the square-root of the limit relative to 5,000 rows.
 * - Presence of `WHERE` reduces the estimate by 28%.
 * - Partition counts and credit bands are derived deterministically from the hash.
 */
function mockSqlPreRunEstimate(sql: string): SqlPreRunEstimate {
  const h = stringHash(sql.trim())
  const s = sql.toLowerCase()
  let bytes = 48 * 1024 * 1024 + (h % 200) * (1024 * 1024)
  const joins = (s.match(/\bjoin\b/g) ?? []).length
  bytes *= 1 + joins * 0.28
  const limitMatch = /\blimit\s+(\d+)/i.exec(s)
  if (limitMatch) {
    const lim = Math.max(1, parseInt(limitMatch[1], 10))
    bytes *= Math.sqrt(Math.min(lim, 100_000) / 5000)
  }
  if (/\bwhere\b/.test(s)) bytes *= 0.72

  const partitionsTotal = 64 + (h % 120)
  const partitionsAssigned = Math.max(
    1,
    Math.min(
      partitionsTotal,
      Math.round(partitionsTotal * (0.35 + (h % 40) / 100))
    )
  )

  const compileMsEstimate = 80 + (h % 220)
  const creditsPerHourWarehouse = 4
  const scanGb = bytes / (1024 * 1024 * 1024)
  const estMin = scanGb * 0.0008 + 0.0002
  const estMax = scanGb * 0.006 + 0.0012

  return {
    estimatedBytes: Math.round(bytes),
    estimatedBytesDisplay: formatBytesShort(Math.round(bytes)),
    partitionsTotal,
    partitionsAssigned,
    compileMsEstimate,
    creditsPerHourWarehouse,
    estimatedCreditsMin: Number(estMin.toFixed(4)),
    estimatedCreditsMax: Number(estMax.toFixed(4)),
    warehouseName: "Lakehouse SQL · Standard (4 credits / hr)",
  }
}

/** Formats a duration in milliseconds as `123 ms`, `4.5 s`, or `2m 06s`. */
function formatDurationMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`
  const sec = ms / 1000
  if (sec < 60) return `${sec.toFixed(sec < 10 ? 1 : 0)} s`
  const m = Math.floor(sec / 60)
  const r = sec - m * 60
  return `${m}m ${r < 10 ? "0" : ""}${Math.floor(r)}s`
}

/**
 * Produces mock "post-run" metrics (wall time, bytes scanned, attributed credits)
 * by combining the pre-run estimate with the actual elapsed wall time and a
 * stable hash. These metrics drive the "Last run" half of `SqlCostInlineDock`.
 */
function buildPostRunMetrics(
  pre: SqlPreRunEstimate,
  wallMs: number,
  rowsReturned: number
): SqlPostRunMetrics {
  const h = stringHash(`${pre.estimatedBytes}-${wallMs}-${rowsReturned}`)
  const ratio = 0.52 + (h % 35) / 100
  const bytesScanned = Math.max(1, Math.round(pre.estimatedBytes * ratio))
  const wallHours = wallMs / 3_600_000
  const creditsFromWall = wallHours * pre.creditsPerHourWarehouse * 0.08
  const creditsFromScan = (bytesScanned / 1024 ** 3) * 0.02
  const creditsAttributed = Number(
    Math.max(0.0001, creditsFromWall + creditsFromScan).toFixed(4)
  )
  return {
    wallClockMs: wallMs,
    bytesScanned,
    bytesScannedDisplay: formatBytesShort(bytesScanned),
    creditsAttributed,
    rowsReturned,
    percentOfExplainUpperBound: Math.min(
      100,
      Math.round((bytesScanned / pre.estimatedBytes) * 100)
    ),
  }
}

/** Single-line label/value row used inside `SqlCostInlineDock`'s details panel. */
function SqlMetricRow({
  label,
  value,
}: {
  label: string
  value: ReactNode
}) {
  return (
    <div className="flex justify-between gap-4 py-1.5 text-sm">
      <span className="shrink-0 text-muted-foreground">{label}</span>
      <span className="min-w-0 text-right font-mono text-xs tabular-nums text-foreground sm:text-sm">
        {value}
      </span>
    </div>
  )
}

/**
 * Inline cost / explain dock shown beneath the SQL editor and the NL output.
 *
 * Three states:
 * 1. No `estimate` → renders the `emptyHint` only.
 * 2. `estimate` only → "Explain" line with bytes / partitions / credit band.
 * 3. `estimate` + `post` → also renders the "Last run" line with wall time,
 *    bytes scanned, rows, attributed credits, and explain-bound utilization.
 *
 * `streaming = true` shows a spinner instead of the "Last run" details.
 * The `<details>` block underneath provides a fuller pre-run / post-run breakdown.
 */
function SqlCostInlineDock({
  estimate,
  post,
  streaming,
  emptyHint = "Type SQL for an explain-style scan and credit band preview.",
}: {
  estimate: SqlPreRunEstimate | null
  post: SqlPostRunMetrics | null
  streaming: boolean
  emptyHint?: string
}) {
  return (
    <div className="rounded-md border border-border/70 bg-muted/20 px-3 py-2">
      {!estimate ? (
        <p className="text-[11px] text-muted-foreground sm:text-xs">
          {emptyHint}
        </p>
      ) : (
        <>
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] leading-relaxed text-muted-foreground sm:text-xs">
            <span className="font-semibold uppercase tracking-wide text-foreground/85">
              Explain
            </span>
            <span className="font-mono tabular-nums text-foreground">
              {estimate.estimatedBytesDisplay}
            </span>
            <span aria-hidden className="text-border">
              ·
            </span>
            <span>
              <span className="text-muted-foreground">parts </span>
              <span className="font-mono tabular-nums text-foreground">
                {estimate.partitionsAssigned}/{estimate.partitionsTotal}
              </span>
            </span>
            <span aria-hidden className="text-border">
              ·
            </span>
            <span>
              <span className="text-muted-foreground">cr est. </span>
              <span className="font-mono tabular-nums text-foreground">
                {estimate.estimatedCreditsMin.toFixed(4)}–
                {estimate.estimatedCreditsMax.toFixed(4)}
              </span>
            </span>

            <span
              aria-hidden
              className="hidden text-border min-[520px]:inline"
            >
              |
            </span>
            {streaming ? (
              <span className="inline-flex items-center gap-1.5">
                <Loader2
                  className="size-3.5 shrink-0 animate-spin text-primary"
                  aria-hidden
                />
                <span className="text-muted-foreground">Measuring usage…</span>
              </span>
            ) : post ? (
              <>
                <span className="font-semibold uppercase tracking-wide text-foreground/85">
                  Last run
                </span>
                <span className="inline-flex items-center gap-0.5 font-mono tabular-nums text-foreground">
                  <Timer className="size-3 opacity-60" aria-hidden />
                  {formatDurationMs(post.wallClockMs)}
                </span>
                <span aria-hidden className="text-border">
                  ·
                </span>
                <span className="font-mono tabular-nums text-foreground">
                  {post.bytesScannedDisplay}
                </span>
                <span aria-hidden className="text-border">
                  ·
                </span>
                <span className="font-mono tabular-nums text-foreground">
                  {post.rowsReturned}
                  <span className="font-sans text-muted-foreground"> rows</span>
                </span>
                <span aria-hidden className="text-border">
                  ·
                </span>
                <span className="inline-flex items-center gap-0.5 font-mono tabular-nums text-foreground">
                  <Coins className="size-3 opacity-60" aria-hidden />
                  {post.creditsAttributed.toFixed(4)}
                </span>
                <span aria-hidden className="text-border">
                  ·
                </span>
                <span>
                  <span className="font-mono tabular-nums text-foreground">
                    {post.percentOfExplainUpperBound}%
                  </span>
                  <span className="text-muted-foreground"> of explain bytes</span>
                </span>
              </>
            ) : (
              <span className="text-muted-foreground">
                Run for wall time, bytes scanned, and attributed credits.
              </span>
            )}
          </div>

          <details className="mt-2">
            <summary className="cursor-pointer text-[11px] font-medium text-primary hover:underline">
              {"Full explain & usage breakdown"}
            </summary>
            <div className="mt-3 space-y-4 border-t border-border/60 pt-3">
              <div className="grid gap-6 sm:grid-cols-2">
                <div className="space-y-1">
                  <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                    Pre-run (explain)
                  </p>
                  <p className="mb-1 text-xs text-muted-foreground">
                    {estimate.warehouseName}
                  </p>
                  <SqlMetricRow
                    label="Est. bytes scanned (upper bound)"
                    value={estimate.estimatedBytesDisplay}
                  />
                  <SqlMetricRow
                    label="Partitions (assigned / total)"
                    value={`${estimate.partitionsAssigned} / ${estimate.partitionsTotal}`}
                  />
                  <SqlMetricRow
                    label="Compile (est.)"
                    value={`~${estimate.compileMsEstimate} ms`}
                  />
                  <SqlMetricRow
                    label="Credit band (est.)"
                    value={
                      <span className="inline-flex items-center gap-1">
                        <Coins className="size-3.5 opacity-70" aria-hidden />
                        {estimate.estimatedCreditsMin.toFixed(4)} –{" "}
                        {estimate.estimatedCreditsMax.toFixed(4)}
                      </span>
                    }
                  />
                  <p className="pt-1 text-[11px] leading-snug text-muted-foreground">
                    Runtime pruning can reduce actual bytes below this explain
                    upper bound.
                  </p>
                </div>
                <div className="sm:border-l sm:border-border sm:pl-6">
                  <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                    Post-run (last completed)
                  </p>
                  {streaming ? (
                    <div className="mt-2 flex items-center gap-2 text-sm text-muted-foreground">
                      <Loader2
                        className="size-4 shrink-0 animate-spin"
                        aria-hidden
                      />
                      Collecting wall time and attributed usage…
                    </div>
                  ) : post ? (
                    <div className="mt-1 space-y-1">
                      <SqlMetricRow
                        label="Wall time"
                        value={
                          <span className="inline-flex items-center gap-1">
                            <Timer
                              className="size-3.5 opacity-70"
                              aria-hidden
                            />
                            {formatDurationMs(post.wallClockMs)}
                          </span>
                        }
                      />
                      <SqlMetricRow
                        label="Bytes scanned"
                        value={post.bytesScannedDisplay}
                      />
                      <SqlMetricRow
                        label="Rows returned"
                        value={post.rowsReturned}
                      />
                      <SqlMetricRow
                        label="Credits attributed"
                        value={
                          <span className="inline-flex items-center gap-1">
                            <Coins
                              className="size-3.5 opacity-70"
                              aria-hidden
                            />
                            {post.creditsAttributed.toFixed(4)}
                          </span>
                        }
                      />
                      <SqlMetricRow
                        label="vs explain upper bound"
                        value={`${post.percentOfExplainUpperBound}% of est. bytes`}
                      />
                    </div>
                  ) : (
                    <p className="mt-2 text-sm text-muted-foreground">
                      Run the query to populate wall time, bytes scanned, row
                      count, and attributed credits.
                    </p>
                  )}
                </div>
              </div>
            </div>
          </details>
        </>
      )}
    </div>
  )
}

type StudioHistoryKind = "natural_language" | "sql"

type HistoryMessage = {
  id: string
  role: "user" | "assistant"
  content: string
}

type StudioHistoryEntry = {
  id: string
  kind: StudioHistoryKind
  preview: string
  createdAt: number
  messages: HistoryMessage[]
}

const STUDIO_HISTORY_CAP = 80

/** Generates a unique id for a history entry or single message. */
function newHistoryId(): string {
  return `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
}

/** Formats a past timestamp as a coarse relative string (Just now / Xm ago / Xh ago / Xd ago). */
function formatHistoryWhen(ts: number): string {
  const sec = Math.floor((Date.now() - ts) / 1000)
  if (sec < 45) return "Just now"
  const min = Math.floor(sec / 60)
  if (min < 60) return `${min}m ago`
  const hr = Math.floor(min / 60)
  if (hr < 48) return `${hr}h ago`
  const day = Math.floor(hr / 24)
  return `${day}d ago`
}

/** Convenience constructor for a single chat message with a fresh id. */
function msg(role: HistoryMessage["role"], content: string): HistoryMessage {
  return { id: newHistoryId(), role, content }
}

/** Sample threads so History is never empty on first visit */
const DUMMY_STUDIO_HISTORY: StudioHistoryEntry[] = [
  {
    id: "dummy-loan-trends",
    kind: "natural_language",
    createdAt: Date.now() - 1000 * 60 * 60 * 26,
    preview: "Approved loan trends — last 2 fiscal years",
    messages: [
      msg(
        "user",
        "Show approved loan trends for the last two fiscal years, ready for a line chart."
      ),
      msg(
        "assistant",
        "Here is SQL for monthly aggregation:\n\n```sql\nSELECT fiscal_year, fiscal_month,\n       SUM(approved_amount) AS total_approved,\n       COUNT(*) AS loan_count\nFROM gold.loan_fact\nWHERE status = 'approved'\n  AND fiscal_year >= EXTRACT(YEAR FROM CURRENT_DATE) - 2\nGROUP BY 1, 2\nORDER BY 1, 2;\n```\n\nYou can plot `total_approved` by month in your BI tool."
      ),
      msg("user", "Also break it down by region."),
      msg(
        "assistant",
        "`region` has been added to the GROUP BY. Filter by region using parameters or a dashboard dropdown."
      ),
    ],
  },
  {
    id: "dummy-customer-top",
    kind: "natural_language",
    createdAt: Date.now() - 1000 * 60 * 60 * 3,
    preview: "Top 5 customers — no loan in 12 months",
    messages: [
      msg(
        "user",
        "List the top 5 customers by portfolio who have not taken a loan in the last 12 months. Include contact and portfolio value."
      ),
      msg(
        "assistant",
        "Result summary:\n\n| customer_id | name | portfolio_value | last_loan_date |\n|-------------|------|-----------------|----------------|\n| C-1042 | PT Maju | 4.2B | 2023-11-02 |\n| C-881 | Ana Wijaya | 3.1B | 2024-01-15 |\n\n_The query completed successfully against the warehouse._"
      ),
    ],
  },
  {
    id: "dummy-sql-bronze",
    kind: "sql",
    createdAt: Date.now() - 1000 * 60 * 45,
    preview: "SELECT … bronze.ingest_errors",
    messages: [
      msg(
        "user",
        `SELECT ingest_id, error_code, occurred_at\nFROM bronze.ingest_errors\nWHERE occurred_at >= CURRENT_DATE - INTERVAL '7' DAY\nORDER BY occurred_at DESC\nLIMIT 200;`
      ),
      msg(
        "assistant",
        "Execution returned 12 rows. Columns available: ingest_id, error_code, occurred_at, payload_ref. Full results are shown in the grid."
      ),
    ],
  },
]

/**
 * Adds (or moves) `entry` to the top of the history list and trims the result
 * to `STUDIO_HISTORY_CAP` items. Existing entries with the same id are deduped.
 */
function prependHistoryEntry(
  prev: StudioHistoryEntry[],
  entry: StudioHistoryEntry
): StudioHistoryEntry[] {
  return [entry, ...prev.filter((e) => e.id !== entry.id)].slice(
    0,
    STUDIO_HISTORY_CAP
  )
}

/** Returns the most recent user message content from a history entry, or null. */
function lastUserMessage(entry: StudioHistoryEntry): string | null {
  const users = entry.messages.filter((m) => m.role === "user")
  const last = users[users.length - 1]
  return last?.content ?? null
}

/** Returns a new history list with an assistant message appended to the entry with the given id. */
function appendAssistantMessage(
  entries: StudioHistoryEntry[],
  entryId: string,
  content: string
): StudioHistoryEntry[] {
  return entries.map((e) =>
    e.id === entryId
      ? {
          ...e,
          messages: [...e.messages, msg("assistant", content)],
        }
      : e
  )
}

/**
 * Right-side panel inside the History sheet that shows the full conversation
 * for a selected history entry, with a "Load into editor" action that drops the
 * last user message back into the NL prompt or the SQL editor.
 */
function HistoryDetailPanel({
  entry,
  onBack,
  onLoadInEditor,
}: {
  entry: StudioHistoryEntry | undefined
  onBack: () => void
  onLoadInEditor: (e: StudioHistoryEntry) => void
}) {
  if (!entry) {
    return (
      <div className="flex flex-col gap-4 px-4 pb-6 pt-2">
        <Button
          type="button"
          variant="ghost"
          className="h-9 w-fit gap-2 px-2"
          onClick={onBack}
        >
          <ArrowLeft className="size-4" />
          Back to list
        </Button>
        <p className="text-sm text-muted-foreground">
          This conversation was not found.
        </p>
      </div>
    )
  }

  const lastUser = lastUserMessage(entry)

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <SheetHeader className="shrink-0 space-y-3 border-b border-border px-4 pb-4 text-left">
        <div className="flex items-start gap-2">
          <Button
            type="button"
            variant="outline"
            size="icon-sm"
            className="shrink-0"
            aria-label="Back to history list"
            onClick={onBack}
          >
            <ArrowLeft className="size-4" />
          </Button>
          <div className="min-w-0 flex-1 space-y-1">
            <SheetTitle className="text-left">Conversation</SheetTitle>
            <SheetDescription className="text-left">
              {entry.kind === "natural_language"
                ? "Natural language"
                : "SQL"}{" "}
              · {formatHistoryWhen(entry.createdAt)} · {entry.messages.length}{" "}
              messages
            </SheetDescription>
          </div>
        </div>
      </SheetHeader>

      <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-4 py-4">
        {entry.messages.map((m) => (
          <div
            key={m.id}
            className={cn(
              "flex w-full",
              m.role === "user" ? "justify-end" : "justify-start"
            )}
          >
            <div
              className={cn(
                "max-w-[min(100%,28rem)] break-words rounded-xl px-3 py-2.5 text-sm leading-relaxed whitespace-pre-wrap",
                m.role === "user"
                  ? "bg-primary/15 text-foreground"
                  : "bg-muted text-muted-foreground"
              )}
            >
              <span className="mb-1 block text-[10px] font-semibold uppercase tracking-wide opacity-70">
                {m.role === "user" ? "You" : "Assistant"}
              </span>
              {m.content}
            </div>
          </div>
        ))}
      </div>

      <div className="shrink-0 border-t border-border bg-background p-4">
        <Button
          type="button"
          className="w-full"
          disabled={!lastUser}
          onClick={() => onLoadInEditor(entry)}
        >
          Load into editor
        </Button>
      </div>
    </div>
  )
}

type WebSpeechRec = {
  lang: string
  interimResults: boolean
  maxAlternatives: number
  onresult: ((ev: { results: { 0: { 0: { transcript: string } } } }) => void) | null
  onerror: (() => void) | null
  onend: (() => void) | null
  start: () => void
  stop: () => void
}

/**
 * Returns the browser's SpeechRecognition constructor if available, else `null`.
 * Handles the WebKit-prefixed variant and the SSR case where `window` is undefined.
 */
function getSpeechRecognitionCtor(): (new () => WebSpeechRec) | null {
  if (typeof window === "undefined") return null
  const w = window as unknown as {
    SpeechRecognition?: new () => WebSpeechRec
    webkitSpeechRecognition?: new () => WebSpeechRec
  }
  return w.SpeechRecognition ?? w.webkitSpeechRecognition ?? null
}

/**
 * Hook that wraps the Web Speech API for the dictation button in the prompt bar.
 *
 * Returns `{ supported, listening, start, stop }`:
 * - `supported`: whether the browser exposes SpeechRecognition (computed after mount).
 * - `listening`: whether dictation is currently running.
 * - `start()` / `stop()`: toggle dictation. `start` is idempotent — calling it
 *   while listening will stop instead. `onTranscript` is invoked once with the
 *   final transcript, and `onEnd` fires when the recognition session ends.
 */
function useSpeechRecognition(
  onTranscript: (text: string) => void,
  onEnd: () => void
) {
  const [supported, setSupported] = useState(false)
  const [listening, setListening] = useState(false)
  const recRef = useRef<WebSpeechRec | null>(null)

  useEffect(() => {
    setSupported(!!getSpeechRecognitionCtor())
  }, [])

  const stop = useCallback(() => {
    try {
      recRef.current?.stop()
    } catch {
      /* ignore */
    }
    recRef.current = null
    setListening(false)
  }, [])

  const start = useCallback(() => {
    const SR = getSpeechRecognitionCtor()
    if (!SR) return

    if (listening) {
      stop()
      return
    }

    const rec = new SR()
    rec.lang = "en-US"
    rec.interimResults = false
    rec.maxAlternatives = 1
    rec.onresult = (event) => {
      const text = event.results[0]?.[0]?.transcript?.trim()
      if (text) onTranscript(text)
    }
    rec.onerror = () => {
      setListening(false)
      recRef.current = null
    }
    rec.onend = () => {
      setListening(false)
      recRef.current = null
      onEnd()
    }
    recRef.current = rec
    setListening(true)
    rec.start()
  }, [listening, onEnd, onTranscript, stop])

  return { supported, listening, start, stop }
}

/**
 * Query Studio page at `/query-studio`.
 *
 * Two top-level tabs:
 * - **Natural language** — the user types or dictates a question. After ~2s the page
 *   fakes "generation" by displaying a generated SQL preview, a 3-row sample grid,
 *   and an AI-generated plain-language explanation. Cost / explain metrics are
 *   shown via `SqlCostInlineDock` for both states.
 * - **SQL editor** — full SQL editor (CodeMirror via `SqlEditor`) with a
 *   simulated streaming result grid, a pre-run explain estimate, and post-run
 *   metrics. Streaming is mocked with `setInterval` over `SAMPLE_SQL_STREAM_ROWS`.
 *
 * Side surfaces:
 * - **Saved**: a `Sheet` listing saved NL prompts; clicking one drops it into the prompt input.
 * - **History**: a `Sheet` listing prior NL/SQL conversations (capped via
 *   `STUDIO_HISTORY_CAP`); clicking opens `HistoryDetailPanel`, which can load
 *   the last user message back into the appropriate editor.
 *
 * All data is in-memory; nothing is persisted across reloads.
 */
export default function QueryStudioPage() {
  const [tab, setTab] = useState(TAB_NL)
  const [topicPage, setTopicPage] = useState(0)
  const [nlPrompt, setNlPrompt] = useState("")
  const [sqlQuery, setSqlQuery] = useState(SQL_PLACEHOLDER)
  const [nlRunState, setNlRunState] = useState<NlRunState>("idle")
  const [nlActiveRequest, setNlActiveRequest] = useState<string | null>(null)
  const [nlGeneratedSql, setNlGeneratedSql] = useState<string | null>(null)
  const [nlResultRows, setNlResultRows] = useState<NlResultRow[] | null>(null)
  const [nlResultExplanation, setNlResultExplanation] = useState<string | null>(
    null
  )
  const [sqlStreamRows, setSqlStreamRows] = useState<SqlStreamRow[]>([])
  const [sqlStreaming, setSqlStreaming] = useState(false)
  const [sqlRan, setSqlRan] = useState(false)
  const [sqlExplainSteps, setSqlExplainSteps] = useState<string[]>([])
  const sqlStreamIntervalRef = useRef<ReturnType<typeof setInterval> | null>(
    null
  )
  const sqlRunStartedAtRef = useRef(0)
  const sqlPreAtRunRef = useRef<SqlPreRunEstimate | null>(null)
  const [sqlPreRunEstimate, setSqlPreRunEstimate] =
    useState<SqlPreRunEstimate | null>(null)
  const [sqlPostRunMetrics, setSqlPostRunMetrics] =
    useState<SqlPostRunMetrics | null>(null)
  const [promptHistory, setPromptHistory] = useState<StudioHistoryEntry[]>(
    () => [...DUMMY_STUDIO_HISTORY]
  )
  const [historyDetailId, setHistoryDetailId] = useState<string | null>(null)
  const [saved, setSaved] = useState<string[]>([])
  const fileRef = useRef<HTMLInputElement>(null)
  const nlGenerateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null
  )
  const nlHistoryPendingIdRef = useRef<string | null>(null)
  const nlCostRunStartedAtRef = useRef(0)
  const nlCostPreAtRunRef = useRef<SqlPreRunEstimate | null>(null)
  const [nlSqlPreRunEstimate, setNlSqlPreRunEstimate] =
    useState<SqlPreRunEstimate | null>(null)
  const [nlSqlPostRunMetrics, setNlSqlPostRunMetrics] =
    useState<SqlPostRunMetrics | null>(null)

  const [nlAgentModel, setNlAgentModel] = useState<AgentModelId>("default")
  const [nlUseIntelligenceKnowledge, setNlUseIntelligenceKnowledge] =
    useState(false)
  const [nlKnowledgeSourceIds, setNlKnowledgeSourceIds] = useState<string[]>(
    []
  )

  /** Appends `text` to the NL prompt, inserting a single space when needed. */
  const appendPrompt = useCallback((text: string) => {
    setNlPrompt((prev) => (prev ? `${prev.trimEnd()} ${text}` : text))
  }, [])

  const { supported: micSupported, listening, start: toggleMic } =
    useSpeechRecognition(appendPrompt, () => {})

  /** Toggles "Use Intelligence Knowledge" and clears the selected sources when turning it off. */
  const handleUseIntelligenceKnowledge = useCallback((v: boolean) => {
    setNlUseIntelligenceKnowledge(v)
    if (!v) setNlKnowledgeSourceIds([])
  }, [])

  /** Adds or removes a knowledge-library entry id from the selection. */
  const toggleNlKnowledgeId = useCallback((id: string, selected: boolean) => {
    setNlKnowledgeSourceIds((prev) =>
      selected
        ? prev.includes(id)
          ? prev
          : [...prev, id]
        : prev.filter((x) => x !== id)
    )
  }, [])

  /** Appends an `@token` reference (table or schema mention) to the NL prompt. */
  const appendSchemaMention = useCallback(
    (token: string) => {
      appendPrompt(` @${token}`)
    },
    [appendPrompt]
  )

  useEffect(() => {
    return () => {
      if (nlGenerateTimeoutRef.current)
        clearTimeout(nlGenerateTimeoutRef.current)
      if (sqlStreamIntervalRef.current) {
        clearInterval(sqlStreamIntervalRef.current)
        sqlStreamIntervalRef.current = null
      }
    }
  }, [])

  useEffect(() => {
    if (tab !== TAB_SQL) return
    const handle = setTimeout(() => {
      const q = sqlQuery.trim()
      setSqlPreRunEstimate(q ? mockSqlPreRunEstimate(q) : null)
    }, 280)
    return () => clearTimeout(handle)
  }, [sqlQuery, tab])

  /**
   * Starts the natural-language "run" simulation.
   *
   * 1. Records a new history entry with the user's question.
   * 2. Stores a pre-run explain estimate so the cost dock has data immediately.
   * 3. After `NL_GENERATE_MS` ms, builds the fake generated SQL, sample rows,
   *    and the AI-style explanation, then appends the assistant message to
   *    that history entry and computes post-run metrics.
   */
  const runNl = useCallback(() => {
    const q = nlPrompt.trim()
    if (!q || nlRunState === "generating") return

    if (nlGenerateTimeoutRef.current) {
      clearTimeout(nlGenerateTimeoutRef.current)
      nlGenerateTimeoutRef.current = null
    }

    const entryId = newHistoryId()
    nlHistoryPendingIdRef.current = entryId
    const preview =
      q.length > 90 ? `${q.slice(0, 90).trim()}…` : q
    setPromptHistory((h) =>
      prependHistoryEntry(h, {
        id: entryId,
        kind: "natural_language",
        preview,
        createdAt: Date.now(),
        messages: [msg("user", q)],
      })
    )
    setNlActiveRequest(q)
    const previewSql = buildNlSqlPreview(q)
    const preNl = mockSqlPreRunEstimate(previewSql)
    nlCostPreAtRunRef.current = preNl
    nlCostRunStartedAtRef.current = Date.now()
    setNlSqlPreRunEstimate(preNl)
    setNlSqlPostRunMetrics(null)
    setNlRunState("generating")
    setNlGeneratedSql(null)
    setNlResultRows(null)
    setNlResultExplanation(null)

    nlGenerateTimeoutRef.current = setTimeout(() => {
      nlGenerateTimeoutRef.current = null
      const sql = buildNlSqlPreview(q)
      const hid = nlHistoryPendingIdRef.current
      nlHistoryPendingIdRef.current = null
      const assistantBody = `Here is the generated SQL:\n\n\`\`\`sql\n${sql}\n\`\`\`\n\nA three-row sample is shown below. Run against the warehouse for the full result set.`
      if (hid) {
        setPromptHistory((h) => appendAssistantMessage(h, hid, assistantBody))
      }
      const rows = sampleNlResultRows(q)
      setNlGeneratedSql(sql)
      setNlResultRows(rows)
      setNlResultExplanation(buildNlNaturalExplanation(q, rows))
      setNlRunState("ready")
      const preCost = nlCostPreAtRunRef.current
      if (preCost) {
        const wall = Date.now() - nlCostRunStartedAtRef.current
        setNlSqlPostRunMetrics(
          buildPostRunMetrics(preCost, wall, rows.length)
        )
      }
    }, NL_GENERATE_MS)
  }, [nlPrompt, nlRunState])

  const showFloatingNlPrompt =
    tab === TAB_NL && (nlRunState === "generating" || nlRunState === "ready")

  /**
   * Starts the SQL "run" simulation.
   *
   * Streams the static `SAMPLE_SQL_STREAM_ROWS` into the result grid one row at
   * a time (every 320ms), records the run in prompt history, and produces
   * pre-run / post-run metrics for the cost dock when the stream completes.
   */
  const runSql = useCallback(() => {
    const q = sqlQuery.trim()
    if (!q) return
    if (sqlStreamIntervalRef.current) {
      clearInterval(sqlStreamIntervalRef.current)
      sqlStreamIntervalRef.current = null
    }
    const preview =
      q.length > 72 ? `${q.slice(0, 72).trim()}…` : q
    setPromptHistory((h) =>
      prependHistoryEntry(h, {
        id: newHistoryId(),
        kind: "sql",
        preview,
        createdAt: Date.now(),
        messages: [
          msg("user", q),
          msg(
            "assistant",
            "Query accepted. Results are streaming below with an AI query trace."
          ),
        ],
      })
    )
    const pre = mockSqlPreRunEstimate(q)
    sqlPreAtRunRef.current = pre
    setSqlPreRunEstimate(pre)
    sqlRunStartedAtRef.current = Date.now()
    setSqlPostRunMetrics(null)
    setSqlRan(true)
    setSqlStreamRows([])
    setSqlStreaming(true)
    setSqlExplainSteps([
      "Resolved referenced relations from the SQL text (catalog and schema inferred).",
      "Row-level security applied: predicate `tenant_id = current_tenant()` on fact tables.",
      "Predicate pushdown on `order_date` for the date range in your WHERE clause.",
      "Result set ordered; rows stream to the grid in batches.",
    ])
    let i = 0
    sqlStreamIntervalRef.current = setInterval(() => {
      const row = SAMPLE_SQL_STREAM_ROWS[i]
      if (row === undefined) {
        if (sqlStreamIntervalRef.current) {
          clearInterval(sqlStreamIntervalRef.current)
          sqlStreamIntervalRef.current = null
        }
        setSqlStreaming(false)
        const pre = sqlPreAtRunRef.current
        if (pre) {
          const wall = Date.now() - sqlRunStartedAtRef.current
          setSqlPostRunMetrics(
            buildPostRunMetrics(pre, wall, SAMPLE_SQL_STREAM_ROWS.length)
          )
        }
        return
      }
      i += 1
      setSqlStreamRows((prev) => [...prev, row])
    }, 320)
  }, [sqlQuery])

  /** Saves the current NL prompt to the in-memory "Saved" list (deduped, capped at 30). */
  const saveCurrentPrompt = useCallback(() => {
    const q = nlPrompt.trim()
    if (!q) return
    setSaved((s) => (s.includes(q) ? s : [q, ...s].slice(0, 30)))
  }, [nlPrompt])

  const topics = TOPIC_PAGES[topicPage] ?? TOPIC_PAGES[0]!

  return (
    <div
      className="flex min-h-[min(720px,calc(100dvh-7rem))] flex-col gap-4"
      data-name="Query Studio"
    >
      <Tabs value={tab} onValueChange={setTab} className="flex flex-1 flex-col gap-4">
        <div className="flex flex-col gap-4 rounded-lg bg-background">
          <div className="flex flex-wrap items-center justify-between gap-3 border-b border-border pb-2">
            <h1 className="font-[family-name:var(--font-montserrat)] text-2xl font-semibold leading-8 tracking-[-0.144px] text-primary">
              Query Studio
            </h1>
            <div className="flex flex-wrap items-center gap-3.5">
              <Button
                type="button"
                variant="outline"
                className="h-10 gap-2 rounded-md border-border bg-background px-4 font-[family-name:var(--font-montserrat)] text-sm font-medium text-primary"
                render={<Link href="/query-studio/collaboration" />}
              >
                <Users2 className="size-4" />
                Collaboration
              </Button>
              <Sheet>
                <SheetTrigger
                  render={
                    <Button
                      type="button"
                      variant="outline"
                      className="h-10 gap-2 rounded-md border-border bg-background px-4 font-[family-name:var(--font-montserrat)] text-sm font-medium text-primary"
                    />
                  }
                >
                  <Bookmark className="size-4" />
                  Saved
                </SheetTrigger>
                <SheetContent side="right" className="w-full sm:max-w-md">
                  <SheetHeader>
                    <SheetTitle>Saved prompts</SheetTitle>
                  </SheetHeader>
                  <div className="mt-4 flex flex-col gap-3 px-4 pb-4">
                    <Button
                      type="button"
                      variant="secondary"
                      className="w-full"
                      onClick={saveCurrentPrompt}
                      disabled={!nlPrompt.trim()}
                    >
                      Save current question
                    </Button>
                    {saved.length === 0 ? (
                      <p className="text-sm text-muted-foreground">
                        No saved prompts yet.
                      </p>
                    ) : (
                      <ul className="flex max-h-[60vh] flex-col gap-2 overflow-y-auto">
                        {saved.map((s, i) => (
                          <li key={`${i}-${s.slice(0, 12)}`}>
                            <button
                              type="button"
                              className="w-full rounded-md border border-border bg-card px-3 py-2 text-left text-sm text-primary hover:bg-muted/80"
                              onClick={() => {
                                setNlPrompt(s)
                                setTab(TAB_NL)
                              }}
                            >
                              {s}
                            </button>
                          </li>
                        ))}
                      </ul>
                    )}
                  </div>
                </SheetContent>
              </Sheet>

              <Sheet
                onOpenChange={(open) => {
                  if (!open) setHistoryDetailId(null)
                }}
              >
                <SheetTrigger
                  render={
                    <Button
                      type="button"
                      variant="outline"
                      className="h-10 gap-2 rounded-md border-border bg-background px-4 font-[family-name:var(--font-montserrat)] text-sm font-medium text-primary"
                    />
                  }
                >
                  <History className="size-4" />
                  History
                </SheetTrigger>
                <SheetContent side="right" className="flex w-full flex-col sm:max-w-md">
                  {historyDetailId ? (
                    <HistoryDetailPanel
                      entry={promptHistory.find((e) => e.id === historyDetailId)}
                      onBack={() => setHistoryDetailId(null)}
                      onLoadInEditor={(entry) => {
                        const text = lastUserMessage(entry)
                        if (!text) return
                        if (entry.kind === "sql") {
                          setTab(TAB_SQL)
                          setSqlQuery(text)
                        } else {
                          setTab(TAB_NL)
                          setNlPrompt(text)
                        }
                        setHistoryDetailId(null)
                      }}
                    />
                  ) : (
                    <>
                      <SheetHeader>
                        <SheetTitle>Prompt &amp; chat history</SheetTitle>
                        <SheetDescription>
                          Past prompts and replies. Tap an entry to read the
                          full conversation.
                        </SheetDescription>
                      </SheetHeader>
                      <div className="mt-2 flex min-h-0 flex-1 flex-col px-4 pb-4">
                        {promptHistory.length === 0 ? (
                          <p className="text-sm text-muted-foreground">
                            No history yet.
                          </p>
                        ) : (
                          <ul className="flex max-h-[min(70vh,640px)] flex-col gap-2 overflow-y-auto">
                            {promptHistory.map((entry) => (
                              <li key={entry.id}>
                                <button
                                  type="button"
                                  className="flex w-full flex-col gap-1.5 rounded-lg border border-border bg-card px-3 py-2.5 text-left transition-colors hover:bg-muted/80"
                                  onClick={() => setHistoryDetailId(entry.id)}
                                >
                                  <div className="flex flex-wrap items-center gap-2">
                                    <Badge
                                      variant="outline"
                                      className={cn(
                                        "text-[10px] font-semibold uppercase tracking-wide",
                                        entry.kind === "natural_language"
                                          ? "border-primary/40 text-primary"
                                          : "border-muted-foreground/40 text-muted-foreground"
                                      )}
                                    >
                                      {entry.kind === "natural_language"
                                        ? "Chat"
                                        : "SQL"}
                                    </Badge>
                                    <span className="text-xs text-muted-foreground">
                                      {formatHistoryWhen(entry.createdAt)}
                                    </span>
                                    <span className="text-xs text-muted-foreground">
                                      · {entry.messages.length} messages
                                    </span>
                                  </div>
                                  <p
                                    className={cn(
                                      "line-clamp-2 text-sm leading-snug text-foreground",
                                      entry.kind === "sql" &&
                                        "font-mono text-xs text-muted-foreground"
                                    )}
                                  >
                                    {entry.preview}
                                  </p>
                                </button>
                              </li>
                            ))}
                          </ul>
                        )}
                      </div>
                    </>
                  )}
                </SheetContent>
              </Sheet>
            </div>
          </div>

          <div className="-mx-1 overflow-x-auto px-1 pb-1">
            <TabsList className="h-auto min-h-9 w-max max-w-full gap-1 rounded-md bg-secondary p-1 sm:inline-flex sm:w-auto sm:flex-nowrap">
              <TabsTrigger
                value={TAB_NL}
                className="gap-2 rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
              >
                <Sparkles className="size-4 shrink-0" />
                Natural language
              </TabsTrigger>
              <TabsTrigger
                value={TAB_SQL}
                className="gap-2 rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
              >
                <Terminal className="size-4 shrink-0" />
                SQL editor
              </TabsTrigger>
            </TabsList>
          </div>
        </div>

        <TabsContent
          value={TAB_NL}
          className="mt-0 flex flex-1 flex-col data-[state=inactive]:hidden"
        >
          <div
            className={cn(
              "flex min-h-0 flex-1 flex-col gap-4",
              showFloatingNlPrompt && "pb-44 sm:pb-48"
            )}
            data-name="naturalLanguage"
          >
            <div className="flex flex-1 flex-col gap-3.5 px-0 sm:px-7">
              {nlRunState === "idle" ? (
                <>
                  <div className="flex w-full items-start justify-between gap-3">
                    <p className="max-w-[342px] text-base leading-6 text-[#222] dark:text-foreground">
                      Start with a topic
                    </p>
                    <div className="flex shrink-0 gap-2">
                      <button
                        type="button"
                        aria-label="Previous topics"
                        className="flex size-5 items-center justify-center rounded-full border border-[#a8b0b8] text-primary disabled:opacity-40"
                        disabled={topicPage <= 0}
                        onClick={() => setTopicPage((p) => Math.max(0, p - 1))}
                      >
                        <ChevronLeft className="size-4" />
                      </button>
                      <button
                        type="button"
                        aria-label="Next topics"
                        className="flex size-5 items-center justify-center rounded-full border border-[#a8b0b8] text-primary disabled:opacity-40"
                        disabled={topicPage >= TOPIC_PAGES.length - 1}
                        onClick={() =>
                          setTopicPage((p) =>
                            Math.min(TOPIC_PAGES.length - 1, p + 1)
                          )
                        }
                      >
                        <ChevronRight className="size-4" />
                      </button>
                    </div>
                  </div>

                  <div className="grid min-h-[152px] gap-3.5 sm:grid-cols-2 xl:grid-cols-4">
                    {topics.map((topic) => (
                      <div
                        key={topic.id}
                        className="flex min-h-[152px] flex-col justify-between overflow-hidden rounded-lg bg-card shadow-[0px_4px_6px_-1px_rgba(0,0,0,0.1),0px_2px_4px_-1px_rgba(0,0,0,0.06)]"
                      >
                        <div className="flex flex-col gap-2 p-3">
                          <p className="font-[family-name:var(--font-montserrat)] text-sm font-medium leading-5 tracking-[-0.084px] text-primary">
                            {topic.title}
                          </p>
                          <p className="font-[family-name:var(--font-montserrat)] text-xs leading-4 tracking-[-0.072px] text-[#5d5d5d] dark:text-muted-foreground">
                            {topic.description}
                          </p>
                        </div>
                        <div className="px-3 pb-3">
                          <Button
                            type="button"
                            variant="outline"
                            size="icon"
                            className="size-10 rounded-md border-border"
                            aria-label={`Use topic: ${topic.title}`}
                            onClick={() => setNlPrompt(topic.description)}
                          >
                            <ArrowUp className="size-4" />
                          </Button>
                        </div>
                      </div>
                    ))}
                  </div>
                </>
              ) : (
                <div className="min-w-0 space-y-2 border-b border-border pb-4">
                  <p className="text-xs font-medium tracking-wide text-muted-foreground">
                    Your request
                  </p>
                  <h2 className="font-[family-name:var(--font-montserrat)] text-lg font-semibold leading-7 tracking-[-0.084px] text-[#222] sm:text-xl dark:text-foreground">
                    {(nlActiveRequest ?? nlPrompt.trim()) || "—"}
                  </h2>
                </div>
              )}

              {(nlRunState === "generating" || nlRunState === "ready") && (
                <div className="flex flex-col gap-3">
                  {nlRunState === "generating" && (
                    <Card className="border-border shadow-sm">
                      <CardContent className="flex items-center gap-3 p-4">
                        <Loader2
                          className="size-5 shrink-0 animate-spin text-primary"
                          aria-hidden
                        />
                        <div>
                          <p className="text-sm font-medium text-primary">
                            SQL query is running
                          </p>
                          <p className="text-xs text-muted-foreground">
                            Generating SQL from your question and executing
                            against the lakehouse.
                          </p>
                        </div>
                      </CardContent>
                    </Card>
                  )}

                  {(nlRunState === "generating" || nlRunState === "ready") &&
                    nlSqlPreRunEstimate && (
                      <SqlCostInlineDock
                        estimate={nlSqlPreRunEstimate}
                        post={nlSqlPostRunMetrics}
                        streaming={nlRunState === "generating"}
                        emptyHint="Run a question to see explain-style cost for the generated SQL."
                      />
                    )}

                  {nlRunState === "ready" && nlGeneratedSql && (
                    <Card className="border-border shadow-sm">
                      <CardHeader className="pb-2">
                        <p className="text-sm font-medium text-primary">
                          Generated SQL
                        </p>
                      </CardHeader>
                      <CardContent>
                        <pre className="max-h-48 overflow-auto rounded-md border border-border bg-muted/40 p-3 font-mono text-xs leading-relaxed text-muted-foreground">
                          {nlGeneratedSql}
                        </pre>
                      </CardContent>
                    </Card>
                  )}

                  {nlRunState === "ready" &&
                    nlResultRows &&
                    nlResultRows.length > 0 &&
                    nlResultExplanation && (
                      <Card className="border-border shadow-sm">
                        <CardHeader className="pb-2">
                          <p className="text-sm font-medium text-primary">
                            Query output
                          </p>
                          <p className="text-xs text-muted-foreground">
                            Switch between the preview grid and a plain-language
                            summary. The preview is not the full warehouse
                            result set.
                          </p>
                        </CardHeader>
                        <CardContent className="pt-0">
                          <Tabs
                            defaultValue={NL_OUTPUT_TAB_RESULT}
                            className="w-full"
                          >
                            <div className="-mx-1 overflow-x-auto px-1 pb-1">
                              <TabsList className="h-auto min-h-9 w-max max-w-full gap-1 rounded-md bg-secondary p-1 sm:inline-flex sm:w-auto sm:flex-nowrap">
                                <TabsTrigger
                                  value={NL_OUTPUT_TAB_RESULT}
                                  className="gap-2 rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
                                >
                                  <Table2 className="size-4 shrink-0" />
                                  Result Query
                                </TabsTrigger>
                                <TabsTrigger
                                  value={NL_OUTPUT_TAB_SUMMARY}
                                  className="gap-2 rounded px-3 py-2 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm sm:text-sm"
                                >
                                  <AlignLeft className="size-4 shrink-0" />
                                  Query explanation (AI trace)
                                </TabsTrigger>
                              </TabsList>
                            </div>
                            <TabsContent
                              value={NL_OUTPUT_TAB_RESULT}
                              className="mt-3 outline-none"
                            >
                              <div className="overflow-x-auto rounded-md border border-border">
                                <Table>
                                  <TableHeader>
                                    <TableRow className="hover:bg-transparent">
                                      <TableHead>Label</TableHead>
                                      <TableHead>Value</TableHead>
                                      <TableHead>As of</TableHead>
                                    </TableRow>
                                  </TableHeader>
                                  <TableBody>
                                    {nlResultRows.map((row, i) => (
                                      <TableRow key={i}>
                                        <TableCell className="font-medium text-primary">
                                          {row.colA}
                                        </TableCell>
                                        <TableCell className="text-muted-foreground">
                                          {row.colB}
                                        </TableCell>
                                        <TableCell className="text-muted-foreground">
                                          {row.colC}
                                        </TableCell>
                                      </TableRow>
                                    ))}
                                  </TableBody>
                                </Table>
                              </div>
                              <p className="mt-2 text-xs text-muted-foreground">
                                First rows returned from the query.
                              </p>
                            </TabsContent>
                            <TabsContent
                              value={NL_OUTPUT_TAB_SUMMARY}
                              className="mt-3 outline-none"
                            >
                              <div className="rounded-md border border-border/80 bg-muted/20 p-4 text-sm leading-relaxed text-foreground">
                                {nlResultExplanation.split("**").map(
                                  (chunk, i) =>
                                    i % 2 === 1 ? (
                                      <strong key={i} className="font-semibold">
                                        {chunk}
                                      </strong>
                                    ) : (
                                      <span key={i}>{chunk}</span>
                                    )
                                )}
                              </div>
                            </TabsContent>
                          </Tabs>
                        </CardContent>
                      </Card>
                    )}
                </div>
              )}
            </div>

            <div
              className={cn(
                "mt-auto flex flex-col gap-3.5 bg-popover px-4 py-4 shadow-[4px_-1px_4px_0px_rgba(0,0,0,0.25)] sm:px-8",
                showFloatingNlPrompt &&
                  "fixed bottom-0 left-0 right-0 z-40 rounded-t-xl border-t border-border shadow-[0_-8px_30px_rgba(0,0,0,0.12)] backdrop-blur-md md:left-[var(--sidebar-width)]"
              )}
              data-name="promptSection"
            >
              <input
                ref={fileRef}
                type="file"
                className="hidden"
                accept=".csv,.txt,.md,.json"
                onChange={(e) => {
                  const f = e.target.files?.[0]
                  if (f)
                    appendPrompt(
                      ` [Attached: ${f.name}]`
                    )
                  e.target.value = ""
                }}
              />
              {(nlAgentModel !== "default" ||
                nlUseIntelligenceKnowledge ||
                nlKnowledgeSourceIds.length > 0) && (
                <div className="-mt-1 flex flex-wrap items-center gap-1.5 px-1">
                  {nlAgentModel !== "default" && (
                    <Badge
                      variant="outline"
                      className="max-w-full truncate text-[11px] font-normal text-primary"
                    >
                      Model:{" "}
                      {AGENT_MODEL_OPTIONS.find((o) => o.id === nlAgentModel)
                        ?.label ?? nlAgentModel}
                    </Badge>
                  )}
                  {nlUseIntelligenceKnowledge && (
                    <Badge
                      variant="secondary"
                      className="text-[11px] font-normal"
                    >
                      Intelligence Knowledge
                      {nlKnowledgeSourceIds.length > 0
                        ? ` · ${nlKnowledgeSourceIds.length} source${
                            nlKnowledgeSourceIds.length === 1 ? "" : "s"
                          }`
                        : ""}
                    </Badge>
                  )}
                </div>
              )}
              <div className="flex w-full min-h-9 items-center gap-2 rounded-full border border-[#a8b0b8] bg-background px-3 py-2.5">
                <PromptActionsMenu
                  disabled={nlRunState === "generating"}
                  onUploadDocument={() => fileRef.current?.click()}
                  agentModel={nlAgentModel}
                  onAgentModelChange={setNlAgentModel}
                  useIntelligenceKnowledge={nlUseIntelligenceKnowledge}
                  onUseIntelligenceKnowledgeChange={
                    handleUseIntelligenceKnowledge
                  }
                  selectedKnowledgeIds={nlKnowledgeSourceIds}
                  onToggleKnowledgeId={toggleNlKnowledgeId}
                  onMentionToken={appendSchemaMention}
                />
                <input
                  type="text"
                  value={nlPrompt}
                  onChange={(e) => setNlPrompt(e.target.value)}
                  onKeyDown={(e) => {
                    if (e.key === "Enter" && nlRunState !== "generating") {
                      e.preventDefault()
                      runNl()
                    }
                  }}
                  placeholder="Ask your data question..."
                  disabled={nlRunState === "generating"}
                  className="min-w-0 flex-1 bg-transparent font-[family-name:var(--font-montserrat)] text-base leading-6 text-foreground outline-none placeholder:text-[#5e6975] disabled:opacity-60 dark:placeholder:text-muted-foreground"
                  aria-label="Natural language prompt"
                />
                <button
                  type="button"
                  className={cn(
                    "flex size-8 shrink-0 items-center justify-center rounded-full text-primary hover:bg-muted",
                    listening && "bg-primary/15",
                    !micSupported && "opacity-40"
                  )}
                  aria-label={
                    micSupported
                      ? listening
                        ? "Stop dictation"
                        : "Dictate with microphone"
                      : "Microphone not supported in this browser"
                  }
                  disabled={!micSupported || nlRunState === "generating"}
                  onClick={() => toggleMic()}
                >
                  <Mic className="size-5" />
                </button>
                <button
                  type="button"
                  className="flex size-8 shrink-0 items-center justify-center rounded-full text-primary hover:bg-muted disabled:opacity-40"
                  aria-label="Submit question"
                  disabled={!nlPrompt.trim() || nlRunState === "generating"}
                  onClick={runNl}
                >
                  {nlRunState === "generating" ? (
                    <Loader2 className="size-5 animate-spin" aria-hidden />
                  ) : (
                    <CircleArrowUp className="size-5" />
                  )}
                </button>
              </div>
              <p className="text-center font-[family-name:var(--font-montserrat)] text-base leading-6 text-[#5d5d5d] dark:text-muted-foreground">
                Text2SQL can generate responses in Markdown, charts, CSV files,
                and execute Python code.
              </p>
            </div>
          </div>
        </TabsContent>

        <TabsContent
          value={TAB_SQL}
          className="mt-0 flex flex-col gap-4 data-[state=inactive]:hidden"
        >
          <Card className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <CardHeader className="border-b border-border px-4 py-3">
              <h2 className="text-base font-medium text-primary">SQL editor</h2>
              <p className="text-sm text-muted-foreground">
                Write SQL against allowed catalogs and schemas. Execution
                respects row filters and classification policies.
              </p>
            </CardHeader>
            <CardContent className="flex flex-col gap-3 p-4">
              <SqlEditor
                value={sqlQuery}
                onChange={setSqlQuery}
                minHeight="240px"
              />
              <div className="flex flex-wrap items-center justify-end gap-2">
                <Button
                  type="button"
                  variant="outline"
                  className="h-10 rounded-md border-border px-4"
                  onClick={() => setSqlQuery(SQL_PLACEHOLDER)}
                >
                  Reset sample
                </Button>
                <Button
                  type="button"
                  className="h-10 gap-2 rounded-md bg-primary px-4 text-primary-foreground"
                  onClick={runSql}
                  disabled={sqlStreaming}
                >
                  {sqlStreaming ? (
                    <Loader2 className="size-4 animate-spin" aria-hidden />
                  ) : (
                    <Play className="size-4" />
                  )}
                  Run
                </Button>
              </div>
              <SqlCostInlineDock
                estimate={sqlPreRunEstimate}
                post={sqlPostRunMetrics}
                streaming={sqlStreaming}
              />
            </CardContent>
          </Card>

          {sqlRan ? (
            <div className="flex flex-col gap-4">
              <Card className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
                <CardHeader className="border-b border-border px-4 py-3">
                  <div className="flex flex-wrap items-center justify-between gap-2">
                    <h2 className="text-base font-medium text-primary">
                      Result grid
                    </h2>
                    {sqlStreaming && (
                      <Badge
                        variant="outline"
                        className="gap-1.5 border-primary/40 text-[11px] font-semibold uppercase tracking-wide text-primary"
                      >
                        <Loader2 className="size-3 animate-spin" aria-hidden />
                        Streaming
                      </Badge>
                    )}
                  </div>
                  <p className="text-sm text-muted-foreground">
                    Rows arrive in batches as the engine streams the result set.
                    You can cancel a long-running query from the toolbar.
                  </p>
                </CardHeader>
                <CardContent className="p-0">
                  <div className="overflow-x-auto">
                    <Table>
                      <TableHeader>
                        <TableRow className="hover:bg-transparent">
                          <TableHead className="pl-4">order_id</TableHead>
                          <TableHead>customer_id</TableHead>
                          <TableHead>amount</TableHead>
                          <TableHead className="pr-4">order_date</TableHead>
                        </TableRow>
                      </TableHeader>
                      <TableBody>
                        {sqlStreamRows.length === 0 ? (
                          <TableRow>
                            <TableCell
                              colSpan={4}
                              className="py-8 text-center text-sm text-muted-foreground"
                            >
                              Waiting for first batch…
                            </TableCell>
                          </TableRow>
                        ) : (
                          sqlStreamRows
                            .filter((row): row is SqlStreamRow => row != null)
                            .map((row, idx) => (
                            <TableRow key={`${row.order_id}-${idx}`}>
                              <TableCell className="pl-4 font-mono text-xs text-primary">
                                {row.order_id}
                              </TableCell>
                              <TableCell className="font-mono text-xs text-muted-foreground">
                                {row.customer_id}
                              </TableCell>
                              <TableCell className="text-muted-foreground">
                                {row.amount}
                              </TableCell>
                              <TableCell className="pr-4 font-mono text-xs text-muted-foreground">
                                {row.order_date}
                              </TableCell>
                            </TableRow>
                          ))
                        )}
                      </TableBody>
                    </Table>
                  </div>
                </CardContent>
              </Card>

              <Card className="overflow-hidden rounded-lg border border-border bg-card shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
                <CardHeader className="border-b border-border px-4 py-3">
                  <div className="flex items-center gap-2">
                    <Sparkles className="size-4 shrink-0 text-primary" aria-hidden />
                    <h2 className="text-base font-medium text-primary">
                      Query explanation (AI trace)
                    </h2>
                  </div>
                  <p className="text-sm text-muted-foreground">
                    Human-readable plan summary generated for this query.
                  </p>
                </CardHeader>
                <CardContent className="space-y-3 p-4">
                  <ol className="list-decimal space-y-2 pl-5 text-sm leading-relaxed text-foreground">
                    {sqlExplainSteps.map((step, i) => (
                      <li key={i} className="text-muted-foreground marker:text-primary">
                        {step}
                      </li>
                    ))}
                  </ol>
                </CardContent>
              </Card>
            </div>
          ) : (
            <ResultsPlaceholder label="Result grid" />
          )}
        </TabsContent>
      </Tabs>
    </div>
  )
}

/** Empty-state card shown in place of the SQL result grid before the first run. */
function ResultsPlaceholder({ label }: { label: string }) {
  return (
    <Card className="rounded-lg border border-dashed border-border bg-muted/30">
      <CardContent className="flex min-h-[160px] flex-col items-center justify-center gap-2 p-6 text-center">
        <p className="text-sm font-medium text-primary">{label}</p>
        <p className="max-w-md text-sm text-muted-foreground">
          Run a SQL query to see results in this grid.
        </p>
      </CardContent>
    </Card>
  )
}
