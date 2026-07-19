"use client"

import type { ReactNode } from "react"
import { useCallback, useEffect, useRef, useState } from "react"
import Image from "next/image"
import Link from "next/link"
import {
  AlertCircle,
  AlignLeft,
  ArrowLeft,
  ArrowUp,
  Bookmark,
  Check,
  CheckCircle2,
  ChevronLeft,
  ChevronRight,
  CircleArrowUp,
  CircleDashed,
  Coins,
  Eraser,
  History,
  Loader2,
  MessageSquarePlus,
  Mic,
  Play,
  RefreshCw,
  Save,
  Sparkles,
  Terminal,
  Timer,
  Users2,
} from "lucide-react"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Card, CardContent, CardHeader } from "@/components/ui/card"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
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
import { SqlCodeBlock } from "@/components/query-studio/sql-code-block"
import {
  AGENT_MODEL_OPTIONS,
  type AgentModelId,
} from "@/lib/knowledge-library-fixtures"
import { cn } from "@/lib/utils"

const TAB_NL = "natural-language"
const TAB_SQL = "sql-editor"

const PANEL_RESULTS = "panel-results"
const PANEL_MESSAGES = "panel-messages"
const PANEL_HISTORY = "panel-history"

const NL_GENERATE_MS = 1800
const CHAT_RUN_MS = 1100

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

/* ------------------------------------------------------------------ */
/* Natural-language chat model                                         */
/* ------------------------------------------------------------------ */

type QueryResultSet = { columns: string[]; rows: string[][] }

type ChatRunResult = QueryResultSet & { durationMs: number }

type ChatMessage = {
  id: string
  role: "user" | "assistant"
  status: "loading" | "done" | "error"
  /** User question, assistant explanation, or error message. */
  content: string
  /** The prompt that produced this assistant message (used by Regenerate). */
  sourcePrompt?: string
  sql?: string
  /** Sample rows to show when the generated SQL is run from the chat. */
  resultSet?: QueryResultSet
  /** Regeneration counter — bumps every time the user hits Regenerate. */
  variant?: number
  runState?: "idle" | "running" | "done"
  runResult?: ChatRunResult
}

type GeneratedQuery = {
  sql: string
  explanation: string
  resultSet: QueryResultSet
}

/**
 * Returns an error message when the prompt asks for something Query Studio
 * refuses to generate (data-modifying statements). Used to demo the chat
 * error state deterministically — no real LLM is involved.
 */
function nlGuardrailError(prompt: string): string | null {
  if (/\b(drop|truncate|delete|alter|grant|revoke|insert|update)\b/i.test(prompt)) {
    return (
      "Query Studio can only generate read-only SELECT queries. Statements that modify " +
      "data or schema (INSERT, UPDATE, DELETE, DROP, ALTER, …) are not allowed here. " +
      "Rephrase your request as a data question — for example: “Show total transactions per month.”"
    )
  }
  return null
}

/**
 * Mock NL→SQL generator. Pattern-matches the prompt to produce a plausible
 * query, a one-sentence explanation, and a matching sample result set.
 * `variant > 0` marks a regenerated answer and slightly changes the output.
 */
function generateSqlFromPrompt(prompt: string, variant: number): GeneratedQuery {
  const p = prompt.toLowerCase()
  const variantComment =
    variant > 0 ? `-- Regenerated (variant ${variant + 1})\n` : ""

  if (
    /(top|terbesar|largest|highest|biggest)/.test(p) &&
    /(customer|pelanggan|nasabah|client)/.test(p)
  ) {
    const nMatch = /\b(\d{1,3})\b/.exec(prompt)
    const n = nMatch?.[1] ?? "10"
    return {
      sql: `${variantComment}SELECT
  c.customer_id,
  c.customer_name,
  SUM(o.amount) AS total_amount,
  COUNT(*) AS trx_count
FROM gold.orders_fact o
JOIN gold.customers_dim c
  ON c.customer_id = o.customer_id
GROUP BY c.customer_id, c.customer_name
ORDER BY total_amount DESC
LIMIT ${n};`,
      explanation: `This joins orders with the customer dimension, aggregates transaction value per customer, and returns the top ${n} customers by total amount.`,
      resultSet: {
        columns: ["customer_id", "customer_name", "total_amount", "trx_count"],
        rows: [
          ["C-1042", "PT Maju Bersama", "4,210,500.00", "312"],
          ["C-3309", "CV Sinar Abadi", "3,884,200.00", "287"],
          ["C-881", "Ana Wijaya", "3,120,940.00", "198"],
          ["C-2201", "PT Nusantara Retail", "2,764,310.00", "244"],
          ["C-552", "Budi Santoso", "2,410,080.00", "176"],
        ],
      },
    }
  }

  if (/(bulan|month|monthly|tren|trend)/.test(p)) {
    const months = variant % 2 === 0 ? "12" : "24"
    return {
      sql: `${variantComment}SELECT
  DATE_TRUNC('month', order_date) AS month,
  SUM(amount) AS total_amount,
  COUNT(*) AS trx_count
FROM gold.orders_fact
WHERE order_date >= CURRENT_DATE - INTERVAL '${months}' MONTH
GROUP BY 1
ORDER BY 1;`,
      explanation: `This aggregates transactions per calendar month over the last ${months} months, returning the total amount and transaction count for each month.`,
      resultSet: {
        columns: ["month", "total_amount", "trx_count"],
        rows: [
          ["2026-01", "1,204,500.00", "8,421"],
          ["2026-02", "1,318,220.00", "8,977"],
          ["2026-03", "1,289,740.00", "8,650"],
          ["2026-04", "1,402,310.00", "9,204"],
          ["2026-05", "1,377,090.00", "9,012"],
          ["2026-06", "1,455,860.00", "9,488"],
        ],
      },
    }
  }

  if (/(region|wilayah|branch|cabang)/.test(p)) {
    return {
      sql: `${variantComment}SELECT
  r.region_name,
  SUM(o.amount) AS total_amount,
  COUNT(DISTINCT o.customer_id) AS active_customers
FROM gold.orders_fact o
JOIN gold.regions_dim r
  ON r.region_id = o.region_id
GROUP BY r.region_name
ORDER BY total_amount DESC;`,
      explanation:
        "This groups transactions by region, returning total amount and the number of active customers in each region, ordered by value.",
      resultSet: {
        columns: ["region_name", "total_amount", "active_customers"],
        rows: [
          ["Jakarta", "5,204,320.00", "1,204"],
          ["Jawa Barat", "3,887,410.00", "986"],
          ["Jawa Timur", "3,214,980.00", "874"],
          ["Sumatera Utara", "2,105,660.00", "603"],
        ],
      },
    }
  }

  const safe = prompt.replace(/'/g, "''").slice(0, 80)
  return {
    sql: `${variantComment}SELECT *
FROM gold.analytics_v
WHERE search_vector @@ plainto_tsquery('english', '${safe}')
ORDER BY relevance DESC
LIMIT ${variant > 0 ? 250 : 500};`,
    explanation:
      "No specific aggregation pattern was detected, so this runs a relevance-ranked search over the curated analytics view. Refine your question to get a more targeted query.",
    resultSet: {
      columns: ["label", "value", "as_of"],
      rows: [
        ["Row 1", "128.40", "2026-04-01"],
        ["Row 2", "94.12", "2026-04-02"],
        ["Row 3", "210.00", "2026-04-03"],
      ],
    },
  }
}

/* ------------------------------------------------------------------ */
/* SQL editor: validation & formatting                                 */
/* ------------------------------------------------------------------ */

type SqlRunStatus = "idle" | "running" | "success" | "error"

type SqlEditorError = { message: string; line: number; column: number }

type SqlLogEntry = {
  id: string
  at: number
  level: "info" | "error"
  text: string
}

type SqlRunHistoryEntry = {
  id: string
  sql: string
  status: "success" | "error"
  durationMs: number
  rows: number
  at: number
}

/**
 * Lightweight mock SQL validation used before "executing" a query.
 * Catches unterminated strings, unbalanced parentheses, and statements that
 * do not start with a read-only keyword — and reports line/column positions
 * so the editor can point at the offending spot.
 */
function validateSql(sqlText: string): SqlEditorError | null {
  let line = 1
  let col = 1
  let inString = false
  let inLineComment = false
  let stringStart: { line: number; column: number } | null = null
  const parenStack: { line: number; column: number }[] = []

  for (let i = 0; i < sqlText.length; i++) {
    const ch = sqlText[i]
    if (ch === "\n") {
      line += 1
      col = 1
      inLineComment = false
      continue
    }
    if (!inLineComment) {
      if (inString) {
        if (ch === "'") {
          if (sqlText[i + 1] === "'") {
            i += 1
            col += 2
            continue
          }
          inString = false
          stringStart = null
        }
      } else if (ch === "'") {
        inString = true
        stringStart = { line, column: col }
      } else if (ch === "-" && sqlText[i + 1] === "-") {
        inLineComment = true
      } else if (ch === "(") {
        parenStack.push({ line, column: col })
      } else if (ch === ")") {
        if (parenStack.length === 0) {
          return {
            message: "Syntax error: unmatched closing parenthesis",
            line,
            column: col,
          }
        }
        parenStack.pop()
      }
    }
    col += 1
  }

  if (inString && stringStart) {
    return {
      message: "Syntax error: unterminated string literal",
      line: stringStart.line,
      column: stringStart.column,
    }
  }
  const openParen = parenStack[parenStack.length - 1]
  if (openParen) {
    return {
      message: "Syntax error: unclosed parenthesis",
      line: openParen.line,
      column: openParen.column,
    }
  }

  const allowed = new Set(["select", "with", "show", "describe", "desc", "explain"])
  const srcLines = sqlText.split("\n")
  for (let li = 0; li < srcLines.length; li++) {
    const withoutComment = srcLines[li]!.replace(/--.*$/, "")
    const match = /\S+/.exec(withoutComment)
    if (!match) continue
    const token = match[0].replace(/[^A-Za-z_]/g, "")
    if (!token || !allowed.has(token.toLowerCase())) {
      return {
        message: `Syntax error near "${match[0]}": expected SELECT, WITH, SHOW, DESCRIBE, or EXPLAIN`,
        line: li + 1,
        column: match.index + 1,
      }
    }
    break
  }
  return null
}

/** Builds a small monospace code frame pointing at the error position. */
function buildErrorFrame(sqlText: string, err: SqlEditorError): string | null {
  const lineText = sqlText.split("\n")[err.line - 1]
  if (lineText === undefined) return null
  const lineNo = String(err.line)
  const caretPad = " ".repeat(Math.max(0, err.column - 1))
  return `${lineNo} | ${lineText}\n${" ".repeat(lineNo.length)} | ${caretPad}^`
}

const SQL_FORMAT_KEYWORDS = [
  "partition by",
  "current_date",
  "group by",
  "order by",
  "left join",
  "right join",
  "inner join",
  "full join",
  "cross join",
  "union all",
  "date_trunc",
  "distinct",
  "interval",
  "between",
  "extract",
  "exists",
  "having",
  "select",
  "offset",
  "union",
  "where",
  "count",
  "limit",
  "from",
  "join",
  "when",
  "then",
  "else",
  "case",
  "with",
  "like",
  "null",
  "over",
  "desc",
  "not",
  "and",
  "avg",
  "end",
  "sum",
  "min",
  "max",
  "asc",
  "on",
  "as",
  "or",
  "in",
  "is",
]

/**
 * Best-effort SQL formatter (mock — no external dependency): uppercases
 * keywords, collapses whitespace, and puts major clauses on their own lines.
 * String literals and comments are masked first so they are never rewritten.
 */
function formatSql(input: string): string {
  const tokens: string[] = []
  let masked = input.replace(/'(?:[^']|'')*'/g, (m) => {
    tokens.push(m)
    return `\u0000S${tokens.length - 1}\u0000`
  })
  masked = masked.replace(/--[^\n]*/g, (m) => {
    tokens.push(m)
    return `\u0000C${tokens.length - 1}\u0000`
  })

  masked = masked.replace(/\s+/g, " ").trim()

  for (const kw of SQL_FORMAT_KEYWORDS) {
    masked = masked.replace(
      new RegExp(`\\b${kw.replace(/ /g, "\\s+")}\\b`, "gi"),
      kw.toUpperCase()
    )
  }

  masked = masked
    .replace(
      /\s+(FROM|WHERE|GROUP BY|ORDER BY|HAVING|LIMIT|OFFSET|UNION ALL|UNION)\b/g,
      "\n$1"
    )
    .replace(/\s+((?:LEFT |RIGHT |INNER |FULL |CROSS )?JOIN)\b/g, "\n$1")
    .replace(/\s+(AND|OR)\b/g, "\n  $1")

  let out = masked
    .replace(/\u0000C(\d+)\u0000 ?/g, (_, i: string) => `${tokens[Number(i)]}\n`)
    .replace(/\u0000S(\d+)\u0000/g, (_, i: string) => tokens[Number(i)] ?? "")

  out = out
    .replace(/[ \t]+\n/g, "\n")
    .replace(/\n{3,}/g, "\n\n")
    .trim()
  return out
}

/* ------------------------------------------------------------------ */
/* Cost / explain mock metrics (unchanged behavior)                    */
/* ------------------------------------------------------------------ */

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
 * Heuristics only — no real planner is involved.
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
 * by combining the pre-run estimate with the actual elapsed wall time.
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
 * Inline cost / explain dock shown beneath the SQL editor.
 * See the pre-run / post-run split in the expandable details block.
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

/* ------------------------------------------------------------------ */
/* Prompt & chat history (side sheet)                                  */
/* ------------------------------------------------------------------ */

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

/** Generates a unique id for a history entry, chat message, or log line. */
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

/* ------------------------------------------------------------------ */
/* Speech recognition (dictation)                                      */
/* ------------------------------------------------------------------ */

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

/* ------------------------------------------------------------------ */
/* Small presentational helpers                                        */
/* ------------------------------------------------------------------ */

/** Colored status pill for the SQL editor toolbar: idle / running / success / error. */
function SqlStatusBadge({ status }: { status: SqlRunStatus }) {
  if (status === "running") {
    return (
      <Badge
        variant="outline"
        className="gap-1.5 border-primary/40 text-[11px] font-semibold uppercase tracking-wide text-primary"
      >
        <Loader2 className="size-3 animate-spin" aria-hidden />
        Running
      </Badge>
    )
  }
  if (status === "success") {
    return (
      <Badge
        variant="outline"
        className="gap-1.5 border-emerald-500/40 text-[11px] font-semibold uppercase tracking-wide text-emerald-600 dark:text-emerald-400"
      >
        <CheckCircle2 className="size-3" aria-hidden />
        Success
      </Badge>
    )
  }
  if (status === "error") {
    return (
      <Badge
        variant="destructive"
        className="gap-1.5 text-[11px] font-semibold uppercase tracking-wide"
      >
        <AlertCircle className="size-3" aria-hidden />
        Error
      </Badge>
    )
  }
  return (
    <Badge
      variant="outline"
      className="gap-1.5 text-[11px] font-semibold uppercase tracking-wide text-muted-foreground"
    >
      <CircleDashed className="size-3" aria-hidden />
      Idle
    </Badge>
  )
}

/**
 * Query Studio page at `/query-studio`.
 *
 * Two top-level tabs:
 * - **Natural language** — a chat surface: the user asks a data question,
 *   the assistant replies with a short explanation plus a highlighted SQL
 *   code block (`SqlCodeBlock`) offering Run Query / Open in SQL Editor /
 *   Copy / Regenerate. Loading and error states are rendered inline.
 * - **SQL editor** — CodeMirror editor with a Run / Save / Format / Clear
 *   toolbar, an idle/running/success/error status pill, and a results panel
 *   split into Results, Messages, and Query History tabs.
 *
 * Integration: "Open in SQL Editor" moves generated SQL into the editor tab
 * without running it; if the editor holds unsaved user changes, a
 * confirmation dialog is shown first. All state lives in this component so
 * both tabs keep their content when switching. Everything is mocked
 * client-side; nothing persists across reloads.
 */
export default function QueryStudioPage() {
  const [tab, setTab] = useState(TAB_NL)
  const [topicPage, setTopicPage] = useState(0)

  /* ---------------- Natural-language chat state ---------------- */
  const [nlPrompt, setNlPrompt] = useState("")
  const [chatMessages, setChatMessages] = useState<ChatMessage[]>([])
  const [nlGenerating, setNlGenerating] = useState(false)
  const nlGenerateTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null
  )
  const chatTimeoutsRef = useRef<Set<ReturnType<typeof setTimeout>>>(new Set())
  const chatEndRef = useRef<HTMLDivElement>(null)
  const fileRef = useRef<HTMLInputElement>(null)

  const [nlAgentModel, setNlAgentModel] = useState<AgentModelId>("default")
  const [nlUseIntelligenceKnowledge, setNlUseIntelligenceKnowledge] =
    useState(false)
  const [nlKnowledgeSourceIds, setNlKnowledgeSourceIds] = useState<string[]>(
    []
  )

  /* ---------------- SQL editor state ---------------- */
  const [sqlQuery, setSqlQuery] = useState(SQL_PLACEHOLDER)
  /** True once the user has hand-edited the editor since the last programmatic load. */
  const sqlDirtyRef = useRef(false)
  const [sqlRunStatus, setSqlRunStatus] = useState<SqlRunStatus>("idle")
  const [sqlError, setSqlError] = useState<SqlEditorError | null>(null)
  const [sqlStreamRows, setSqlStreamRows] = useState<SqlStreamRow[]>([])
  const [sqlRan, setSqlRan] = useState(false)
  const [sqlLogs, setSqlLogs] = useState<SqlLogEntry[]>([])
  const [sqlRunHistory, setSqlRunHistory] = useState<SqlRunHistoryEntry[]>([])
  const [resultsPanelTab, setResultsPanelTab] = useState(PANEL_RESULTS)
  const sqlStreamIntervalRef = useRef<ReturnType<typeof setInterval> | null>(
    null
  )
  const sqlRunStartedAtRef = useRef(0)
  const sqlPreAtRunRef = useRef<SqlPreRunEstimate | null>(null)
  const [sqlPreRunEstimate, setSqlPreRunEstimate] =
    useState<SqlPreRunEstimate | null>(null)
  const [sqlPostRunMetrics, setSqlPostRunMetrics] =
    useState<SqlPostRunMetrics | null>(null)

  /* ---------------- Cross-tab / dialogs ---------------- */
  const [pendingSqlLoad, setPendingSqlLoad] = useState<string | null>(null)
  const [confirmClearOpen, setConfirmClearOpen] = useState(false)
  const [sqlSavedFlash, setSqlSavedFlash] = useState(false)
  const savedFlashTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(
    null
  )

  /* ---------------- Saved & history sheets ---------------- */
  const [savedPrompts, setSavedPrompts] = useState<string[]>([])
  const [savedQueries, setSavedQueries] = useState<string[]>([])
  const [promptHistory, setPromptHistory] = useState<StudioHistoryEntry[]>(
    () => [...DUMMY_STUDIO_HISTORY]
  )
  const [historyDetailId, setHistoryDetailId] = useState<string | null>(null)

  const sqlRunning = sqlRunStatus === "running"

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

  /* ---------------- Lifecycle ---------------- */

  useEffect(() => {
    const chatTimeouts = chatTimeoutsRef.current
    return () => {
      if (nlGenerateTimeoutRef.current)
        clearTimeout(nlGenerateTimeoutRef.current)
      if (savedFlashTimeoutRef.current)
        clearTimeout(savedFlashTimeoutRef.current)
      chatTimeouts.forEach((t) => clearTimeout(t))
      chatTimeouts.clear()
      if (sqlStreamIntervalRef.current) {
        clearInterval(sqlStreamIntervalRef.current)
        sqlStreamIntervalRef.current = null
      }
    }
  }, [])

  // Debounced explain estimate while typing in the SQL tab.
  useEffect(() => {
    if (tab !== TAB_SQL) return
    const handle = setTimeout(() => {
      const q = sqlQuery.trim()
      setSqlPreRunEstimate(q ? mockSqlPreRunEstimate(q) : null)
    }, 280)
    return () => clearTimeout(handle)
  }, [sqlQuery, tab])

  // Keep the newest chat message in view.
  useEffect(() => {
    if (tab !== TAB_NL || chatMessages.length === 0) return
    chatEndRef.current?.scrollIntoView({ behavior: "smooth", block: "end" })
  }, [chatMessages, tab])

  /* ---------------- Natural-language chat actions ---------------- */

  /**
   * Sends a question to the mock NL→SQL "assistant": appends the user bubble
   * and a loading assistant bubble, records the exchange in prompt history,
   * and resolves after `NL_GENERATE_MS` with either a generated query or a
   * guardrail error.
   */
  const runNlWith = useCallback(
    (raw: string) => {
      const q = raw.trim()
      if (!q || nlGenerating) return

      const assistantId = newHistoryId()
      setChatMessages((prev) => [
        ...prev,
        { id: newHistoryId(), role: "user", status: "done", content: q },
        {
          id: assistantId,
          role: "assistant",
          status: "loading",
          content: "",
          sourcePrompt: q,
          variant: 0,
        },
      ])
      setNlPrompt("")
      setNlGenerating(true)

      const entryId = newHistoryId()
      setPromptHistory((h) =>
        prependHistoryEntry(h, {
          id: entryId,
          kind: "natural_language",
          preview: q.length > 90 ? `${q.slice(0, 90).trim()}…` : q,
          createdAt: Date.now(),
          messages: [msg("user", q)],
        })
      )

      nlGenerateTimeoutRef.current = setTimeout(() => {
        nlGenerateTimeoutRef.current = null
        const guard = nlGuardrailError(q)
        if (guard) {
          setChatMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? { ...m, status: "error", content: guard }
                : m
            )
          )
          setPromptHistory((h) => appendAssistantMessage(h, entryId, guard))
        } else {
          const generated = generateSqlFromPrompt(q, 0)
          setChatMessages((prev) =>
            prev.map((m) =>
              m.id === assistantId
                ? {
                    ...m,
                    status: "done",
                    content: generated.explanation,
                    sql: generated.sql,
                    resultSet: generated.resultSet,
                    runState: "idle",
                    runResult: undefined,
                  }
                : m
            )
          )
          setPromptHistory((h) =>
            appendAssistantMessage(
              h,
              entryId,
              `${generated.explanation}\n\n\`\`\`sql\n${generated.sql}\n\`\`\``
            )
          )
        }
        setNlGenerating(false)
      }, NL_GENERATE_MS)
    },
    [nlGenerating]
  )

  /** Regenerates (or retries after an error) the assistant message with a fresh variant. */
  const regenerateAssistant = useCallback(
    (messageId: string) => {
      if (nlGenerating) return
      const target = chatMessages.find((m) => m.id === messageId)
      const prompt = target?.sourcePrompt
      if (!prompt) return
      const nextVariant = (target.variant ?? 0) + 1

      setChatMessages((prev) =>
        prev.map((m) =>
          m.id === messageId
            ? {
                ...m,
                status: "loading",
                content: "",
                sql: undefined,
                resultSet: undefined,
                runState: undefined,
                runResult: undefined,
                variant: nextVariant,
              }
            : m
        )
      )
      setNlGenerating(true)

      const t = setTimeout(() => {
        chatTimeoutsRef.current.delete(t)
        const guard = nlGuardrailError(prompt)
        setChatMessages((prev) =>
          prev.map((m) => {
            if (m.id !== messageId) return m
            if (guard) return { ...m, status: "error", content: guard }
            const generated = generateSqlFromPrompt(prompt, nextVariant)
            return {
              ...m,
              status: "done",
              content: generated.explanation,
              sql: generated.sql,
              resultSet: generated.resultSet,
              runState: "idle",
              runResult: undefined,
            }
          })
        )
        setNlGenerating(false)
      }, NL_GENERATE_MS)
      chatTimeoutsRef.current.add(t)
    },
    [chatMessages, nlGenerating]
  )

  /** Executes a generated query from the chat and attaches a sample result to the message. */
  const runChatSql = useCallback(
    (messageId: string) => {
      const target = chatMessages.find((m) => m.id === messageId)
      if (!target?.sql || target.runState === "running") return
      const startedAt = Date.now()
      setChatMessages((prev) =>
        prev.map((m) =>
          m.id === messageId ? { ...m, runState: "running" } : m
        )
      )
      const t = setTimeout(() => {
        chatTimeoutsRef.current.delete(t)
        setChatMessages((prev) =>
          prev.map((m) => {
            if (m.id !== messageId) return m
            const rs = m.resultSet ?? { columns: ["label", "value"], rows: [] }
            return {
              ...m,
              runState: "done",
              runResult: { ...rs, durationMs: Date.now() - startedAt },
            }
          })
        )
      }, CHAT_RUN_MS)
      chatTimeoutsRef.current.add(t)
    },
    [chatMessages]
  )

  /** Starts a fresh conversation. Past exchanges remain in the History sheet. */
  const startNewChat = useCallback(() => {
    if (nlGenerating) return
    setChatMessages([])
  }, [nlGenerating])

  /* ---------------- SQL editor actions ---------------- */

  const pushSqlLog = useCallback(
    (level: SqlLogEntry["level"], text: string) => {
      setSqlLogs((prev) =>
        [...prev, { id: newHistoryId(), at: Date.now(), level, text }].slice(
          -100
        )
      )
    },
    []
  )

  /** User edit — marks the editor dirty so cross-tab loads ask before replacing. */
  const handleSqlChange = useCallback((v: string) => {
    sqlDirtyRef.current = true
    setSqlQuery(v)
  }, [])

  /** Loads SQL into the editor and switches to the SQL tab. Never auto-runs. */
  const applySqlLoad = useCallback((sql: string) => {
    setSqlQuery(sql)
    sqlDirtyRef.current = false
    setSqlRunStatus("idle")
    setSqlError(null)
    setTab(TAB_SQL)
  }, [])

  /**
   * Entry point for every "put this SQL into the editor" action (chat,
   * saved queries, run history, history sheet). Asks for confirmation when
   * the editor holds unsaved manual edits that would be overwritten.
   */
  const requestOpenInEditor = useCallback(
    (sql: string) => {
      const current = sqlQuery.trim()
      const hasUnsavedWork =
        sqlDirtyRef.current && current !== "" && current !== sql.trim()
      if (hasUnsavedWork) {
        setPendingSqlLoad(sql)
      } else {
        applySqlLoad(sql)
      }
    },
    [applySqlLoad, sqlQuery]
  )

  /**
   * Runs the current SQL. Validation errors surface immediately with a
   * line/column position; valid queries stream mock rows into the Results
   * panel and finish with duration + row-count metrics.
   */
  const runSql = useCallback(() => {
    const q = sqlQuery.trim()
    if (!q || sqlRunStatus === "running") return

    const validation = validateSql(q)
    if (validation) {
      setSqlRunStatus("error")
      setSqlError(validation)
      pushSqlLog(
        "error",
        `${validation.message} (line ${validation.line}, column ${validation.column})`
      )
      setSqlRunHistory((prev) =>
        [
          {
            id: newHistoryId(),
            sql: q,
            status: "error" as const,
            durationMs: 0,
            rows: 0,
            at: Date.now(),
          },
          ...prev,
        ].slice(0, 50)
      )
      setResultsPanelTab(PANEL_MESSAGES)
      return
    }

    if (sqlStreamIntervalRef.current) {
      clearInterval(sqlStreamIntervalRef.current)
      sqlStreamIntervalRef.current = null
    }

    setSqlError(null)
    setSqlRunStatus("running")
    setSqlRan(true)
    setSqlStreamRows([])
    setResultsPanelTab(PANEL_RESULTS)

    const pre = mockSqlPreRunEstimate(q)
    sqlPreAtRunRef.current = pre
    setSqlPreRunEstimate(pre)
    setSqlPostRunMetrics(null)
    sqlRunStartedAtRef.current = Date.now()

    pushSqlLog(
      "info",
      "Query accepted — compiling against allowed catalogs and schemas."
    )
    pushSqlLog(
      "info",
      `Explain estimate: ~${pre.estimatedBytesDisplay} scanned, ${pre.partitionsAssigned}/${pre.partitionsTotal} partitions.`
    )
    pushSqlLog(
      "info",
      "Row-level security applied: tenant_id = current_tenant() on fact tables."
    )

    let i = 0
    sqlStreamIntervalRef.current = setInterval(() => {
      const row = SAMPLE_SQL_STREAM_ROWS[i]
      if (row === undefined) {
        if (sqlStreamIntervalRef.current) {
          clearInterval(sqlStreamIntervalRef.current)
          sqlStreamIntervalRef.current = null
        }
        const wall = Date.now() - sqlRunStartedAtRef.current
        const rows = SAMPLE_SQL_STREAM_ROWS.length
        setSqlRunStatus("success")
        const preAtRun = sqlPreAtRunRef.current
        if (preAtRun) {
          setSqlPostRunMetrics(buildPostRunMetrics(preAtRun, wall, rows))
        }
        pushSqlLog(
          "info",
          `Query completed in ${formatDurationMs(wall)} — ${rows} rows returned.`
        )
        setSqlRunHistory((prev) =>
          [
            {
              id: newHistoryId(),
              sql: q,
              status: "success" as const,
              durationMs: wall,
              rows,
              at: Date.now(),
            },
            ...prev,
          ].slice(0, 50)
        )
        setPromptHistory((h) =>
          prependHistoryEntry(h, {
            id: newHistoryId(),
            kind: "sql",
            preview: q.length > 72 ? `${q.slice(0, 72).trim()}…` : q,
            createdAt: Date.now(),
            messages: [
              msg("user", q),
              msg(
                "assistant",
                `Execution completed in ${formatDurationMs(wall)} and returned ${rows} rows.`
              ),
            ],
          })
        )
        return
      }
      i += 1
      setSqlStreamRows((prev) => [...prev, row])
    }, 300)
  }, [pushSqlLog, sqlQuery, sqlRunStatus])

  /** Pretty-prints the editor content (keywords uppercased, clauses on new lines). */
  const handleFormatSql = useCallback(() => {
    const q = sqlQuery.trim()
    if (!q) return
    setSqlQuery(formatSql(q))
  }, [sqlQuery])

  /** Confirmed via dialog — empties the editor and resets run state. */
  const confirmClear = useCallback(() => {
    setSqlQuery("")
    sqlDirtyRef.current = false
    setSqlRunStatus("idle")
    setSqlError(null)
    setConfirmClearOpen(false)
  }, [])

  /** Saves the current SQL to the Saved sheet (deduped) with a brief "Saved" flash. */
  const handleSaveQuery = useCallback(() => {
    const q = sqlQuery.trim()
    if (!q) return
    setSavedQueries((s) => (s.includes(q) ? s : [q, ...s].slice(0, 30)))
    setSqlSavedFlash(true)
    if (savedFlashTimeoutRef.current) clearTimeout(savedFlashTimeoutRef.current)
    savedFlashTimeoutRef.current = setTimeout(
      () => setSqlSavedFlash(false),
      1600
    )
  }, [sqlQuery])

  /** Saves the current NL prompt to the in-memory "Saved" list (deduped, capped at 30). */
  const saveCurrentPrompt = useCallback(() => {
    const q = nlPrompt.trim()
    if (!q) return
    setSavedPrompts((s) => (s.includes(q) ? s : [q, ...s].slice(0, 30)))
  }, [nlPrompt])

  const topics = TOPIC_PAGES[topicPage] ?? TOPIC_PAGES[0]!
  const errorFrame =
    sqlError !== null ? buildErrorFrame(sqlQuery, sqlError) : null

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
                <SheetContent side="right" className="flex w-full flex-col sm:max-w-md">
                  <SheetHeader>
                    <SheetTitle>Saved prompts &amp; queries</SheetTitle>
                    <SheetDescription>
                      Reuse saved questions in the chat or load saved SQL into
                      the editor.
                    </SheetDescription>
                  </SheetHeader>
                  <div className="mt-2 flex min-h-0 flex-1 flex-col gap-6 overflow-y-auto px-4 pb-6">
                    <section className="flex flex-col gap-3">
                      <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                        Prompts
                      </p>
                      <Button
                        type="button"
                        variant="secondary"
                        className="w-full"
                        onClick={saveCurrentPrompt}
                        disabled={!nlPrompt.trim()}
                      >
                        Save current question
                      </Button>
                      {savedPrompts.length === 0 ? (
                        <p className="text-sm text-muted-foreground">
                          No saved prompts yet.
                        </p>
                      ) : (
                        <ul className="flex flex-col gap-2">
                          {savedPrompts.map((s, i) => (
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
                    </section>

                    <section className="flex flex-col gap-3">
                      <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                        SQL queries
                      </p>
                      {savedQueries.length === 0 ? (
                        <p className="text-sm text-muted-foreground">
                          No saved queries yet. Use{" "}
                          <span className="font-medium text-foreground">
                            Save
                          </span>{" "}
                          in the SQL editor toolbar.
                        </p>
                      ) : (
                        <ul className="flex flex-col gap-2">
                          {savedQueries.map((s, i) => (
                            <li key={`${i}-${s.slice(0, 12)}`}>
                              <button
                                type="button"
                                className="w-full rounded-md border border-border bg-card px-3 py-2 text-left hover:bg-muted/80"
                                onClick={() => requestOpenInEditor(s)}
                              >
                                <span className="line-clamp-3 font-mono text-xs leading-relaxed text-muted-foreground">
                                  {s}
                                </span>
                              </button>
                            </li>
                          ))}
                        </ul>
                      )}
                    </section>
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
                          requestOpenInEditor(text)
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

        {/* ------------------------------------------------------------ */}
        {/* Natural language (chat)                                      */}
        {/* ------------------------------------------------------------ */}
        <TabsContent
          value={TAB_NL}
          className="mt-0 flex flex-1 flex-col data-[state=inactive]:hidden"
        >
          <div
            className="mx-auto flex w-full max-w-4xl flex-1 flex-col"
            data-name="naturalLanguage"
          >
            {chatMessages.length === 0 ? (
              <div className="flex flex-1 flex-col justify-center gap-7 py-6">
                <div className="flex flex-col items-center gap-3 text-center">
                  <div className="relative size-14 shrink-0 overflow-hidden rounded-xl sm:size-16">
                    <Image
                      src="/rantai.png"
                      alt="Rantai"
                      fill
                      sizes="64px"
                      className="object-cover"
                      priority
                    />
                  </div>
                  <div className="space-y-1.5">
                    <h2 className="font-[family-name:var(--font-montserrat)] text-xl font-semibold tracking-[-0.1px] text-primary sm:text-2xl">
                      Ask your data anything
                    </h2>
                    <p className="mx-auto max-w-xl text-sm leading-relaxed text-muted-foreground">
                      Describe the data you need in plain language. The AI drafts
                      the SQL for you — run it here, or open it in the SQL editor
                      to refine and save it.
                    </p>
                  </div>
                </div>

                <div className="flex flex-col gap-3.5">
                  <div className="flex w-full items-start justify-between gap-3">
                    <p className="text-sm font-medium text-foreground">
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
                            aria-label={`Ask: ${topic.title}`}
                            disabled={nlGenerating}
                            onClick={() => runNlWith(topic.description)}
                          >
                            <ArrowUp className="size-4" />
                          </Button>
                        </div>
                      </div>
                    ))}
                  </div>
                </div>
              </div>
            ) : (
              <div className="flex flex-1 flex-col gap-5 pb-6 pt-1">
                <div className="flex items-center justify-between gap-2">
                  <p className="text-[11px] font-semibold uppercase tracking-wide text-muted-foreground">
                    Conversation
                  </p>
                  <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    onClick={startNewChat}
                    disabled={nlGenerating}
                  >
                    <MessageSquarePlus aria-hidden />
                    New chat
                  </Button>
                </div>

                {chatMessages.map((m) =>
                  m.role === "user" ? (
                    <div key={m.id} className="flex justify-end">
                      <div className="max-w-[min(90%,36rem)] whitespace-pre-wrap break-words rounded-2xl rounded-br-md bg-primary px-4 py-2.5 text-sm leading-relaxed text-primary-foreground">
                        {m.content}
                      </div>
                    </div>
                  ) : (
                    <div key={m.id} className="flex items-start gap-3">
                      <div className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full border border-border bg-card">
                        <Sparkles
                          className="size-3.5 text-primary"
                          aria-hidden
                        />
                      </div>
                      <div className="flex min-w-0 flex-1 flex-col gap-2.5">
                        {m.status === "loading" ? (
                          <div className="flex max-w-[min(100%,44rem)] items-center gap-3 rounded-xl rounded-tl-sm border border-border bg-card px-4 py-3">
                            <Loader2
                              className="size-4 shrink-0 animate-spin text-primary"
                              aria-hidden
                            />
                            <div>
                              <p className="text-sm font-medium text-primary">
                                Generating SQL…
                              </p>
                              <p className="text-xs text-muted-foreground">
                                Translating your question into a query against
                                the lakehouse.
                              </p>
                            </div>
                          </div>
                        ) : m.status === "error" ? (
                          <div className="max-w-[min(100%,44rem)] rounded-xl rounded-tl-sm border border-destructive/40 bg-destructive/5 px-4 py-3">
                            <div className="flex items-start gap-2.5">
                              <AlertCircle
                                className="mt-0.5 size-4 shrink-0 text-destructive"
                                aria-hidden
                              />
                              <div className="min-w-0 space-y-2">
                                <p className="text-sm font-medium text-destructive">
                                  Couldn&apos;t generate a query
                                </p>
                                <p className="text-sm leading-relaxed text-muted-foreground">
                                  {m.content}
                                </p>
                                <Button
                                  type="button"
                                  size="sm"
                                  variant="outline"
                                  onClick={() => regenerateAssistant(m.id)}
                                  disabled={nlGenerating}
                                >
                                  <RefreshCw aria-hidden />
                                  Try again
                                </Button>
                              </div>
                            </div>
                          </div>
                        ) : (
                          <>
                            {m.content && (
                              <div className="max-w-[min(100%,44rem)] rounded-xl rounded-tl-sm border border-border bg-card px-3.5 py-2.5 text-sm leading-relaxed text-foreground">
                                {m.content}
                              </div>
                            )}
                            {m.sql && (
                              <SqlCodeBlock
                                sql={m.sql}
                                className="max-w-[min(100%,44rem)]"
                                running={m.runState === "running"}
                                disabled={nlGenerating}
                                onRun={() => runChatSql(m.id)}
                                onOpenInEditor={() =>
                                  requestOpenInEditor(m.sql!)
                                }
                                onRegenerate={() => regenerateAssistant(m.id)}
                              />
                            )}
                            {m.runState === "running" && (
                              <div className="flex items-center gap-2 text-xs text-muted-foreground">
                                <Loader2
                                  className="size-3.5 animate-spin text-primary"
                                  aria-hidden
                                />
                                Running query against the lakehouse…
                              </div>
                            )}
                            {m.runResult && (
                              <div className="max-w-[min(100%,44rem)] overflow-hidden rounded-lg border border-border bg-card">
                                <div className="overflow-x-auto">
                                  <Table>
                                    <TableHeader>
                                      <TableRow className="hover:bg-transparent">
                                        {m.runResult.columns.map((col) => (
                                          <TableHead
                                            key={col}
                                            className="font-mono text-xs first:pl-3 last:pr-3"
                                          >
                                            {col}
                                          </TableHead>
                                        ))}
                                      </TableRow>
                                    </TableHeader>
                                    <TableBody>
                                      {m.runResult.rows.map((row, ri) => (
                                        <TableRow key={ri}>
                                          {row.map((cell, ci) => (
                                            <TableCell
                                              key={ci}
                                              className={cn(
                                                "text-muted-foreground first:pl-3 last:pr-3",
                                                ci === 0 &&
                                                  "font-medium text-primary"
                                              )}
                                            >
                                              {cell}
                                            </TableCell>
                                          ))}
                                        </TableRow>
                                      ))}
                                    </TableBody>
                                  </Table>
                                </div>
                                <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border bg-muted/40 px-3 py-1.5 text-[11px] text-muted-foreground">
                                  <span className="font-mono tabular-nums text-foreground">
                                    {m.runResult.rows.length} rows
                                  </span>
                                  <span aria-hidden className="text-border">
                                    ·
                                  </span>
                                  <span className="inline-flex items-center gap-1 font-mono tabular-nums text-foreground">
                                    <Timer
                                      className="size-3 opacity-60"
                                      aria-hidden
                                    />
                                    {formatDurationMs(m.runResult.durationMs)}
                                  </span>
                                  <span aria-hidden className="text-border">
                                    ·
                                  </span>
                                  <span>
                                    preview — open in the SQL editor for the
                                    full result set
                                  </span>
                                </div>
                              </div>
                            )}
                          </>
                        )}
                      </div>
                    </div>
                  )
                )}
                <div ref={chatEndRef} aria-hidden />
              </div>
            )}

            {/* Sticky prompt bar */}
            <div
              className="sticky bottom-0 z-30 mt-auto pt-2"
              data-name="promptSection"
            >
              <div className="flex flex-col gap-2.5 rounded-xl border border-border bg-popover/95 p-3 shadow-[0_-4px_24px_rgba(0,0,0,0.10)] backdrop-blur-md sm:p-4">
                <input
                  ref={fileRef}
                  type="file"
                  className="hidden"
                  accept=".csv,.txt,.md,.json"
                  onChange={(e) => {
                    const f = e.target.files?.[0]
                    if (f) appendPrompt(` [Attached: ${f.name}]`)
                    e.target.value = ""
                  }}
                />
                {(nlAgentModel !== "default" ||
                  nlUseIntelligenceKnowledge ||
                  nlKnowledgeSourceIds.length > 0) && (
                  <div className="flex flex-wrap items-center gap-1.5 px-1">
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
                    disabled={nlGenerating}
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
                      if (e.key === "Enter" && !nlGenerating) {
                        e.preventDefault()
                        runNlWith(nlPrompt)
                      }
                    }}
                    placeholder="Ask your data question..."
                    disabled={nlGenerating}
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
                    disabled={!micSupported || nlGenerating}
                    onClick={() => toggleMic()}
                  >
                    <Mic className="size-5" />
                  </button>
                  <button
                    type="button"
                    className="flex size-8 shrink-0 items-center justify-center rounded-full text-primary hover:bg-muted disabled:opacity-40"
                    aria-label="Submit question"
                    disabled={!nlPrompt.trim() || nlGenerating}
                    onClick={() => runNlWith(nlPrompt)}
                  >
                    {nlGenerating ? (
                      <Loader2 className="size-5 animate-spin" aria-hidden />
                    ) : (
                      <CircleArrowUp className="size-5" />
                    )}
                  </button>
                </div>
                <p className="text-center font-[family-name:var(--font-montserrat)] text-xs leading-5 text-[#5d5d5d] dark:text-muted-foreground">
                  Text2SQL can generate responses in Markdown, charts, CSV
                  files, and execute Python code.
                </p>
              </div>
            </div>
          </div>
        </TabsContent>

        {/* ------------------------------------------------------------ */}
        {/* SQL editor                                                   */}
        {/* ------------------------------------------------------------ */}
        <TabsContent
          value={TAB_SQL}
          className="mt-0 flex flex-col gap-4 data-[state=inactive]:hidden"
        >
          <Card className="gap-0 overflow-hidden rounded-lg border border-border bg-card py-0 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <CardHeader className="gap-2 border-b border-border px-4 py-3">
              <div className="flex flex-wrap items-center justify-between gap-3">
                <div className="flex flex-wrap items-center gap-2.5">
                  <h2 className="text-base font-medium text-primary">
                    SQL editor
                  </h2>
                  <SqlStatusBadge status={sqlRunStatus} />
                </div>
                <div className="flex flex-wrap items-center gap-1.5">
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleFormatSql}
                    disabled={!sqlQuery.trim() || sqlRunning}
                  >
                    <AlignLeft aria-hidden />
                    Format
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={() => {
                      if (sqlQuery.trim()) setConfirmClearOpen(true)
                    }}
                    disabled={!sqlQuery.trim() || sqlRunning}
                  >
                    <Eraser aria-hidden />
                    Clear
                  </Button>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={handleSaveQuery}
                    disabled={!sqlQuery.trim()}
                  >
                    {sqlSavedFlash ? (
                      <Check className="text-emerald-600 dark:text-emerald-400" aria-hidden />
                    ) : (
                      <Save aria-hidden />
                    )}
                    {sqlSavedFlash ? "Saved" : "Save"}
                  </Button>
                  <Button
                    type="button"
                    size="sm"
                    onClick={runSql}
                    disabled={sqlRunning || !sqlQuery.trim()}
                  >
                    {sqlRunning ? (
                      <Loader2 className="animate-spin" aria-hidden />
                    ) : (
                      <Play aria-hidden />
                    )}
                    Run
                  </Button>
                </div>
              </div>
              <p className="text-sm text-muted-foreground">
                Write SQL against allowed catalogs and schemas. Execution
                respects row filters and classification policies.
              </p>
            </CardHeader>
            <CardContent className="flex flex-col gap-3 p-4">
              <SqlEditor
                value={sqlQuery}
                onChange={handleSqlChange}
                minHeight="260px"
              />

              {sqlError && sqlRunStatus === "error" && (
                <div
                  className="rounded-md border border-destructive/40 bg-destructive/5 p-3"
                  role="alert"
                >
                  <div className="flex items-start gap-2.5">
                    <AlertCircle
                      className="mt-0.5 size-4 shrink-0 text-destructive"
                      aria-hidden
                    />
                    <div className="min-w-0 flex-1 space-y-1.5">
                      <p className="text-sm font-medium text-destructive">
                        {sqlError.message}
                      </p>
                      <p className="text-xs text-muted-foreground">
                        Line {sqlError.line}, column {sqlError.column}
                      </p>
                      {errorFrame && (
                        <pre className="overflow-x-auto rounded bg-muted/50 p-2 font-mono text-xs leading-relaxed text-foreground">
                          {errorFrame}
                        </pre>
                      )}
                    </div>
                  </div>
                </div>
              )}

              <SqlCostInlineDock
                estimate={sqlPreRunEstimate}
                post={sqlPostRunMetrics}
                streaming={sqlRunning}
              />
            </CardContent>
          </Card>

          <Card className="gap-0 overflow-hidden rounded-lg border border-border bg-card py-0 shadow-[0px_1px_2px_0px_rgba(0,0,0,0.05)]">
            <Tabs
              value={resultsPanelTab}
              onValueChange={setResultsPanelTab}
              className="gap-0"
            >
              <div className="flex flex-wrap items-center justify-between gap-2 border-b border-border px-4 py-2.5">
                <TabsList className="h-auto min-h-8 w-max gap-1 rounded-md bg-secondary p-1">
                  <TabsTrigger
                    value={PANEL_RESULTS}
                    className="gap-1.5 rounded px-3 py-1.5 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm"
                  >
                    Results
                    {sqlRan && (
                      <span className="rounded-full bg-muted px-1.5 font-mono text-[10px] tabular-nums text-muted-foreground">
                        {sqlStreamRows.length}
                      </span>
                    )}
                  </TabsTrigger>
                  <TabsTrigger
                    value={PANEL_MESSAGES}
                    className="gap-1.5 rounded px-3 py-1.5 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm"
                  >
                    Messages
                  </TabsTrigger>
                  <TabsTrigger
                    value={PANEL_HISTORY}
                    className="gap-1.5 rounded px-3 py-1.5 text-xs font-medium data-active:bg-background data-active:text-primary data-active:shadow-sm"
                  >
                    Query history
                  </TabsTrigger>
                </TabsList>

                {sqlRunning ? (
                  <Badge
                    variant="outline"
                    className="gap-1.5 border-primary/40 text-[11px] font-semibold uppercase tracking-wide text-primary"
                  >
                    <Loader2 className="size-3 animate-spin" aria-hidden />
                    Streaming
                  </Badge>
                ) : sqlPostRunMetrics ? (
                  <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs text-muted-foreground">
                    <span className="inline-flex items-center gap-1 font-mono tabular-nums text-foreground">
                      <Timer className="size-3 opacity-60" aria-hidden />
                      {formatDurationMs(sqlPostRunMetrics.wallClockMs)}
                    </span>
                    <span aria-hidden className="text-border">
                      ·
                    </span>
                    <span className="font-mono tabular-nums text-foreground">
                      {sqlPostRunMetrics.rowsReturned}
                      <span className="font-sans text-muted-foreground">
                        {" "}
                        rows
                      </span>
                    </span>
                  </div>
                ) : null}
              </div>

              <TabsContent value={PANEL_RESULTS} className="mt-0 outline-none">
                {sqlRan ? (
                  <div>
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
                    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 border-t border-border bg-muted/40 px-4 py-2 text-xs text-muted-foreground">
                      <span className="font-mono tabular-nums text-foreground">
                        {sqlStreamRows.length} rows
                      </span>
                      {sqlRunning ? (
                        <>
                          <span aria-hidden className="text-border">
                            ·
                          </span>
                          <span className="inline-flex items-center gap-1.5">
                            <Loader2
                              className="size-3 animate-spin text-primary"
                              aria-hidden
                            />
                            streaming in batches…
                          </span>
                        </>
                      ) : sqlPostRunMetrics ? (
                        <>
                          <span aria-hidden className="text-border">
                            ·
                          </span>
                          <span className="inline-flex items-center gap-1 font-mono tabular-nums text-foreground">
                            <Timer className="size-3 opacity-60" aria-hidden />
                            {formatDurationMs(sqlPostRunMetrics.wallClockMs)}
                          </span>
                          <span aria-hidden className="text-border">
                            ·
                          </span>
                          <span>
                            {sqlPostRunMetrics.bytesScannedDisplay} scanned
                          </span>
                        </>
                      ) : null}
                    </div>
                  </div>
                ) : (
                  <div className="flex min-h-[160px] flex-col items-center justify-center gap-2 p-6 text-center">
                    <p className="text-sm font-medium text-primary">
                      No results yet
                    </p>
                    <p className="max-w-md text-sm text-muted-foreground">
                      Run a SQL query to see results in this grid.
                    </p>
                  </div>
                )}
              </TabsContent>

              <TabsContent value={PANEL_MESSAGES} className="mt-0 outline-none">
                {sqlLogs.length === 0 ? (
                  <div className="flex min-h-[160px] flex-col items-center justify-center gap-2 p-6 text-center">
                    <p className="text-sm font-medium text-primary">
                      No messages yet
                    </p>
                    <p className="max-w-md text-sm text-muted-foreground">
                      Engine output, security notices, and errors appear here
                      when you run a query.
                    </p>
                  </div>
                ) : (
                  <div className="max-h-80 overflow-y-auto p-4">
                    {sqlLogs.map((l) => (
                      <div
                        key={l.id}
                        className={cn(
                          "flex gap-3 py-0.5 font-mono text-xs leading-relaxed",
                          l.level === "error"
                            ? "text-destructive"
                            : "text-muted-foreground"
                        )}
                      >
                        <span className="shrink-0 tabular-nums opacity-60">
                          {new Date(l.at).toLocaleTimeString([], {
                            hour: "2-digit",
                            minute: "2-digit",
                            second: "2-digit",
                          })}
                        </span>
                        <span className="whitespace-pre-wrap break-words">
                          {l.text}
                        </span>
                      </div>
                    ))}
                  </div>
                )}
              </TabsContent>

              <TabsContent value={PANEL_HISTORY} className="mt-0 outline-none">
                {sqlRunHistory.length === 0 ? (
                  <div className="flex min-h-[160px] flex-col items-center justify-center gap-2 p-6 text-center">
                    <p className="text-sm font-medium text-primary">
                      No runs yet
                    </p>
                    <p className="max-w-md text-sm text-muted-foreground">
                      Every query you run in this session is listed here so you
                      can reload it into the editor.
                    </p>
                  </div>
                ) : (
                  <ul className="max-h-80 divide-y divide-border overflow-y-auto">
                    {sqlRunHistory.map((e) => (
                      <li
                        key={e.id}
                        className="flex items-start justify-between gap-3 px-4 py-3"
                      >
                        <div className="min-w-0 flex-1 space-y-1">
                          <div className="flex flex-wrap items-center gap-2">
                            {e.status === "success" ? (
                              <Badge
                                variant="outline"
                                className="gap-1 border-emerald-500/40 text-[10px] font-semibold uppercase tracking-wide text-emerald-600 dark:text-emerald-400"
                              >
                                <CheckCircle2 className="size-3" aria-hidden />
                                Success
                              </Badge>
                            ) : (
                              <Badge
                                variant="destructive"
                                className="gap-1 text-[10px] font-semibold uppercase tracking-wide"
                              >
                                <AlertCircle className="size-3" aria-hidden />
                                Error
                              </Badge>
                            )}
                            <span className="text-xs text-muted-foreground">
                              {formatHistoryWhen(e.at)}
                            </span>
                            {e.status === "success" && (
                              <span className="text-xs text-muted-foreground">
                                · {formatDurationMs(e.durationMs)} · {e.rows}{" "}
                                rows
                              </span>
                            )}
                          </div>
                          <p className="line-clamp-2 font-mono text-xs leading-relaxed text-muted-foreground">
                            {e.sql}
                          </p>
                        </div>
                        <Button
                          type="button"
                          variant="outline"
                          size="sm"
                          className="shrink-0"
                          onClick={() => requestOpenInEditor(e.sql)}
                        >
                          Load
                        </Button>
                      </li>
                    ))}
                  </ul>
                )}
              </TabsContent>
            </Tabs>
          </Card>
        </TabsContent>
      </Tabs>

      {/* Confirm: replace unsaved editor content */}
      <Dialog
        open={pendingSqlLoad !== null}
        onOpenChange={(open) => {
          if (!open) setPendingSqlLoad(null)
        }}
      >
        <DialogContent className="sm:max-w-md" showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>Replace the current SQL?</DialogTitle>
            <DialogDescription>
              The SQL editor has unsaved changes. Opening this query will
              overwrite what you&apos;re working on.
            </DialogDescription>
          </DialogHeader>
          <div className="px-5 py-4">
            <pre className="max-h-36 overflow-auto rounded-md border border-border bg-muted/40 p-3 font-mono text-xs leading-relaxed text-muted-foreground">
              {pendingSqlLoad ?? ""}
            </pre>
          </div>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setPendingSqlLoad(null)}
            >
              Cancel
            </Button>
            <Button
              type="button"
              onClick={() => {
                if (pendingSqlLoad !== null) applySqlLoad(pendingSqlLoad)
                setPendingSqlLoad(null)
              }}
            >
              Replace query
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>

      {/* Confirm: clear editor */}
      <Dialog open={confirmClearOpen} onOpenChange={setConfirmClearOpen}>
        <DialogContent className="sm:max-w-md" showCloseButton={false}>
          <DialogHeader>
            <DialogTitle>Clear the SQL editor?</DialogTitle>
            <DialogDescription>
              This removes the current query from the editor. Saved queries and
              run history are kept.
            </DialogDescription>
          </DialogHeader>
          <DialogFooter>
            <Button
              type="button"
              variant="outline"
              onClick={() => setConfirmClearOpen(false)}
            >
              Cancel
            </Button>
            <Button type="button" variant="destructive" onClick={confirmClear}>
              Clear editor
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
    </div>
  )
}
