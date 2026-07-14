/** Shared mock data for Intelligence Knowledge library (UI fixtures). */

export type KnowledgeEntry = {
  id: string
  title: string
  sourceQuery: string
  rowCount: number
  distinctUsers: number
  zone: string
  excerpt: string
  ingestedAt: string
}

/** User-uploaded files or web-authored context saved to the knowledge library (UI fixtures). */
export type UserKnowledgeDocument =
  | {
      id: string
      kind: "upload"
      title: string
      fileName: string
      fileSizeBytes: number
      mimeType: string
      createdAt: string
    }
  | {
      id: string
      kind: "web_context"
      title: string
      body: string
      createdAt: string
    }

export const USER_KNOWLEDGE_DOCUMENTS_INITIAL: UserKnowledgeDocument[] = [
  {
    id: "ud-1",
    kind: "upload",
    title: "data_dictionary_q2.pdf",
    fileName: "data_dictionary_q2.pdf",
    fileSizeBytes: 482_000,
    mimeType: "application/pdf",
    createdAt: "2026-04-14T11:20:00Z",
  },
  {
    id: "ud-2",
    kind: "web_context",
    title: "Support escalation — SSO notes",
    body: "When users report SSO loops, check session cookie scope and IdP clock skew. Escalate to platform if redirect URIs changed in the last deploy.",
    createdAt: "2026-04-13T08:05:00Z",
  },
]

export const KNOWLEDGE_LIBRARY_INITIAL_ENTRIES: KnowledgeEntry[] = [
  {
    id: "k-1",
    title: "gold.orders_fact — last 7 days",
    sourceQuery:
      "SELECT order_id, amount, order_date FROM gold.orders_fact WHERE order_date >= CURRENT_DATE - 7",
    rowCount: 12840,
    distinctUsers: 42,
    zone: "Gold",
    excerpt:
      "Weekly order aggregates show a lift on weekends; median amount stays in the 95–130 range.",
    ingestedAt: "2026-04-17T14:22:00Z",
  },
  {
    id: "k-2",
    title: "silver.dim_customer — priority segment",
    sourceQuery:
      "SELECT region, segment, portfolio_value FROM silver.dim_customer WHERE segment = 'priority'",
    rowCount: 4200,
    distinctUsers: 18,
    zone: "Silver",
    excerpt:
      "Priority customers cluster in three regions; portfolio_value feeds downstream LTV features.",
    ingestedAt: "2026-04-16T09:10:00Z",
  },
  {
    id: "k-3",
    title: "semantic.support_tickets — timeout theme",
    sourceQuery:
      "SELECT ticket_id, body FROM semantic.support_tickets WHERE body ILIKE '%timeout%'",
    rowCount: 891,
    distinctUsers: 7,
    zone: "Semantic",
    excerpt:
      "Timeout issues often correlate with SSO redirect and session cookies cleared mid-flow.",
    ingestedAt: "2026-04-15T16:45:00Z",
  },
]

export const AGENT_MODEL_OPTIONS = [
  { id: "default", label: "Default · balanced" },
  { id: "fast", label: "Fast · low latency" },
  { id: "reasoning", label: "Reasoning · deeper analysis" },
] as const

export type AgentModelId = (typeof AGENT_MODEL_OPTIONS)[number]["id"]

/** Tables / schemas users can @-mention into the prompt. */
export const SCHEMA_TABLE_MENTIONS = [
  { id: "m1", label: "Table · gold.orders_fact", token: "gold.orders_fact" },
  { id: "m2", label: "Table · silver.dim_customer", token: "silver.dim_customer" },
  { id: "m3", label: "Table · semantic.support_tickets", token: "semantic.support_tickets" },
  { id: "m4", label: "Schema · prod.gold", token: "prod.gold" },
  { id: "m5", label: "Schema · prod.silver", token: "prod.silver" },
] as const
