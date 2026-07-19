/**
 * Shared Natural Language → SQL helpers for Query Studio and
 * Collaborative Query Studio. All generation is mocked client-side.
 */

export const NL_GENERATE_MS = 1800
export const CHAT_RUN_MS = 1100

export type Topic = { id: string; title: string; description: string }

export const TOPIC_PAGES: Topic[][] = [
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

export type QueryResultSet = { columns: string[]; rows: string[][] }

export type ChatRunResult = QueryResultSet & { durationMs: number }

export type ChatMessage = {
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

export type GeneratedQuery = {
  sql: string
  explanation: string
  resultSet: QueryResultSet
}

/** Fresh id for chat messages and related UI keys. */
export function newNlId(): string {
  return `${Date.now()}-${Math.random().toString(36).slice(2, 9)}`
}

/** Formats a duration in milliseconds as `123 ms`, `4.5 s`, or `2m 06s`. */
export function formatDurationMs(ms: number): string {
  if (ms < 1000) return `${Math.round(ms)} ms`
  const sec = ms / 1000
  if (sec < 60) return `${sec.toFixed(sec < 10 ? 1 : 0)} s`
  const m = Math.floor(sec / 60)
  const r = sec - m * 60
  return `${m}m ${r < 10 ? "0" : ""}${Math.floor(r)}s`
}

/**
 * Returns an error message when the prompt asks for something Query Studio
 * refuses to generate (data-modifying statements).
 */
export function nlGuardrailError(prompt: string): string | null {
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
 */
export function generateSqlFromPrompt(
  prompt: string,
  variant: number
): GeneratedQuery {
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
