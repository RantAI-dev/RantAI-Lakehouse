"use client"

import * as React from "react"

import { useDebouncedCallback } from "@/hooks/use-debounced-callback"
import { useService, useServiceAction } from "@/hooks/use-service"
import { readDraft, writeDraft } from "@/lib/query-draft"
import { hasStatement } from "@/lib/sql-text"
import { assetService, queryService } from "@/services"
import type { NlAnswer, QueryEngine, SaveQueryInput } from "@/services/contracts/queries"

/** What the editor holds before anyone types. */
export const STARTER_SQL = "-- Write SQL here, or generate it from a question"

/** How long to wait after the last keystroke before asking for an estimate. */
const ESTIMATE_DEBOUNCE_MS = 500

/** How long to wait before writing the draft to storage. */
const DRAFT_DEBOUNCE_MS = 400

export type StudioTab = "nl" | "sql"

/**
 * All of Query Studio's state in one place: the two editors, the pre-run
 * estimate, the agent, saving, and the draft that survives a reload.
 *
 * It lives apart from the page because the page had grown into 260 lines
 * of mixed concerns, and because the rules that matter here — don't
 * estimate on every keystroke, don't estimate a buffer that holds only a
 * comment — are easy to lose sight of inside JSX.
 */
export function useQueryStudio() {
  const [tab, setTab] = React.useState<StudioTab>("nl")
  const [question, setQuestion] = React.useState("")
  const [sql, setSql] = React.useState(STARTER_SQL)
  const [engine, setEngine] = React.useState<QueryEngine>("clickhouse")

  const generateAct = useServiceAction((signal, q: string) =>
    queryService.generateSql(q, signal)
  )
  const estimateAct = useServiceAction((signal, s: string) =>
    queryService.estimate(s, signal)
  )
  const runAct = useServiceAction((signal, s: string, runEngine: QueryEngine) =>
    queryService.run(s, { engine: runEngine }, signal)
  )
  const saveAct = useServiceAction((signal, input: SaveQueryInput) =>
    queryService.saveQuery(input, signal)
  )
  const savedState = useService((s) => queryService.listSaved(s), [])
  const historyState = useService((s) => queryService.listHistory(s), [])

  // Schema for editor autocomplete, from the real catalog so suggestions
  // cannot drift from what exists.
  //
  // TABLE NAMES ONLY, not their columns: the column list lives on
  // `AssetDetail`, and fetching it means one request per asset — too much
  // for a completion list. Columns can follow if the backend grows a bulk
  // schema endpoint.
  //
  // A failure here is deliberately not shown as a page error: autocomplete
  // is a nicety, and Query Studio works without it.
  const assetsState = useService((s) => assetService.listAssets({}, s), [])
  const sqlSchema = React.useMemo(() => {
    if (assetsState.status !== "success") return undefined
    const schema: Record<string, string[]> = {}
    for (const asset of assetsState.data) {
      schema[`${asset.namespace}.${asset.name}`] = []
    }
    return Object.keys(schema).length > 0 ? schema : undefined
  }, [assetsState.status, assetsState.data])

  // ── Draft ────────────────────────────────────────────────────────────
  // Restored after mount, never during render: `localStorage` does not
  // exist on the server, and reading it in render would make the first
  // paint differ from the markup Next sent.
  const hydratedRef = React.useRef(false)
  React.useEffect(() => {
    if (hydratedRef.current) return
    hydratedRef.current = true
    // A `?sql=` link (e.g. an asset's "Open in Query Studio") names the
    // query the user came here to run, so it wins over any stored draft.
    // `?engine=` rides along because the SQL is only valid on the engine it
    // was written for (e.g. `iceberg.bronze.*` names exist only on Trino).
    // Read from `window.location`, like the `?saved=` handoff below, to
    // avoid a `useSearchParams` Suspense boundary.
    const params = new URLSearchParams(window.location.search)
    const linkedSql = params.get("sql")
    if (linkedSql && hasStatement(linkedSql)) {
      setSql(linkedSql)
      setTab("sql")
      const linkedEngine = params.get("engine")
      if (linkedEngine === "clickhouse" || linkedEngine === "trino") setEngine(linkedEngine)
      return
    }
    const draft = readDraft()
    if (!draft) return
    if (draft.sql) setSql(draft.sql)
    if (draft.question) setQuestion(draft.question)
    setTab(draft.tab)
  }, [])

  const persistDraft = useDebouncedCallback(
    (next: { sql: string; question: string; tab: StudioTab }) => {
      writeDraft(next)
    },
    DRAFT_DEBOUNCE_MS
  )
  React.useEffect(() => {
    if (!hydratedRef.current) return
    persistDraft({ sql: sql === STARTER_SQL ? "" : sql, question, tab })
  }, [sql, question, tab, persistDraft])

  // ── Estimate ─────────────────────────────────────────────────────────
  const runEstimate = estimateAct.run
  const estimateDebounced = useDebouncedCallback((next: string) => {
    void runEstimate(next)
  }, ESTIMATE_DEBOUNCE_MS)
  const estimatable = hasStatement(sql)
  React.useEffect(() => {
    // A buffer holding only the starter comment has nothing to estimate,
    // and asking anyway put an EXPLAIN ESTIMATE on ClickHouse for every
    // keystroke, including the ones that typed the comment.
    if (estimatable) estimateDebounced(sql)
  }, [sql, estimatable, estimateDebounced])

  // ── Actions ──────────────────────────────────────────────────────────
  const loadSql = React.useCallback((next: string) => {
    setSql(next)
    setTab("sql")
  }, [])

  const generate = React.useCallback(async () => {
    const out = await generateAct.run(question)
    if (out) loadSql(out.sql)
  }, [generateAct, question, loadSql])

  const runQuery = React.useCallback(async () => {
    if (!hasStatement(sql)) return
    await runAct.run(sql, engine)
    // Every run is recorded, including the ones that failed, so the rail
    // has to catch up — otherwise the query you just ran is missing from
    // your own history until a reload.
    historyState.reload()
  }, [runAct, sql, engine, historyState])

  const save = React.useCallback(
    async (input: SaveQueryInput) => {
      const saved = await saveAct.run(input)
      if (saved) savedState.reload()
      return saved
    },
    [saveAct, savedState]
  )

  // Agentic ask: one call to the chat's engine, which writes the SQL, runs
  // it as the caller (masking and row filters apply), corrects itself on
  // error and explains the result (`/api/ai/chat`).
  const [agentBusy, setAgentBusy] = React.useState(false)
  const [agentResult, setAgentResult] = React.useState<NlAnswer | null>(null)
  const [agentError, setAgentError] = React.useState<string | null>(null)
  const ask = React.useCallback(async () => {
    setAgentBusy(true)
    setAgentError(null)
    setAgentResult(null)
    try {
      const out = await queryService.askQuestion(question)
      setAgentResult(out)
      // The final SQL goes into the editor so it can be reviewed and
      // tweaked rather than taken on trust. A reply with no SQL (the
      // model ran none) leaves the editor as it was.
      if (out.sql !== undefined) setSql(out.sql)
    } catch (e) {
      setAgentError(e instanceof Error ? e.message : String(e))
    } finally {
      setAgentBusy(false)
    }
  }, [question])

  // Handoff from Saved Queries: /query-studio?saved=<id> loads that SQL.
  // Read from window.location to avoid a useSearchParams Suspense boundary.
  const appliedSavedRef = React.useRef(false)
  React.useEffect(() => {
    if (appliedSavedRef.current || savedState.status !== "success") return
    appliedSavedRef.current = true
    const savedId = new URLSearchParams(window.location.search).get("saved")
    if (!savedId) return
    const match = savedState.data.find((q) => q.id === savedId)
    if (match) loadSql(match.sql)
  }, [savedState, loadSql])

  return {
    tab,
    setTab,
    question,
    setQuestion,
    sql,
    setSql,
    engine,
    setEngine,
    sqlSchema,
    estimatable,
    generateAct,
    estimateAct,
    runAct,
    saveAct,
    savedState,
    historyState,
    agent: { busy: agentBusy, result: agentResult, error: agentError, ask },
    loadSql,
    generate,
    runQuery,
    reEstimate: () => void runEstimate(sql),
    save,
  }
}
