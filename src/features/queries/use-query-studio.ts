"use client"

import * as React from "react"

import type { EditorView } from "@codemirror/view"

import { useDebouncedCallback } from "@/hooks/use-debounced-callback"
import { useService, useServiceAction } from "@/hooks/use-service"
import { catalogRefusal } from "@/lib/catalog-refusal"
import { readDraft, writeDraft } from "@/lib/query-draft"
import {
  tablesInStatement,
  type ColumnState,
  type CompletionTable,
} from "@/lib/sql-completion"
import { hasStatement } from "@/lib/sql-text"
import { assetService, queryService } from "@/services"
import { toServiceError, type ServiceError } from "@/services/errors"
import type {
  NlAnswer,
  QueryEngine,
  SaveQueryInput,
  SqlFixOutcome,
} from "@/services/contracts/queries"
import {
  createSqlCompletionSource,
  sqlCompletionExtension,
} from "./sql-completion-source"

/** What the editor holds before anyone types. */
export const STARTER_SQL = "-- Write SQL here, or generate it from a question"

/** How long to wait after the last keystroke before asking for an estimate. */
const ESTIMATE_DEBOUNCE_MS = 500

/** How long to wait before writing the draft to storage. */
const DRAFT_DEBOUNCE_MS = 400

/**
 * The shape of a catalog id that is a SQL table name: `silver.orders`.
 *
 * A Bronze asset's id is a registry slug, not a table name, and its Trino
 * name sits on the asset's detail rather than on the list.
 */
const SQL_TABLE_NAME = /^[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*$/

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
  const [sql, setSqlState] = React.useState(STARTER_SQL)
  const [engine, setEngine] = React.useState<QueryEngine>("clickhouse")

  // A proposal is a diff against the editor as it was when it came back.
  // Once the editor changes, the diff describes something else, so it goes.
  const [fixResult, setFixResult] = React.useState<SqlFixOutcome | null>(null)
  const [fixBusy, setFixBusy] = React.useState(false)
  const [fixError, setFixError] = React.useState<ServiceError | null>(null)
  const [explainResult, setExplainResult] = React.useState<string | null>(null)
  const [explainBusy, setExplainBusy] = React.useState(false)
  const [explainError, setExplainError] = React.useState<ServiceError | null>(null)
  const setSql = React.useCallback((next: string) => {
    setSqlState(next)
    setFixResult(null)
  }, [])

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

  // The tables the catalog shows, from the real catalog so suggestions
  // cannot drift from what exists.
  //
  // A failure here is deliberately not shown as a page error: autocomplete
  // is a nicety, and Query Studio works without it.
  const assetsState = useService((s) => assetService.listAssets({}, s), [])

  // The tables in the shape the completion needs: the SQL name to insert,
  // the name a person reads, and every name the table answers to.
  //
  // A Bronze asset's registry slug is not a SQL name, and its Trino name
  // sits on the asset's detail rather than on this list, so such a table
  // gets no reference and is never suggested.
  const tables = React.useMemo<CompletionTable[]>(() => {
    if (assetsState.status !== "success") return []
    return assetsState.data.map((asset) => ({
      id: asset.id,
      label: asset.name,
      reference: SQL_TABLE_NAME.test(asset.id) ? asset.id : "",
      layer: asset.layer,
      names: [asset.id, `${asset.namespace}.${asset.name}`],
    }))
  }, [assetsState.status, assetsState.data])

  // The columns of the tables the buffer names: fetched once each and kept
  // for the session. A table that cannot be read is kept as a failure too,
  // so it is not asked for again on every keystroke.
  //
  // The ref holds the same cache as the state, because the completion
  // source reads it the moment a keystroke asks for suggestions, and a
  // state update is not there yet at that point.
  const [columns, setColumns] = React.useState<Record<string, ColumnState>>({})
  const columnsRef = React.useRef<Record<string, ColumnState>>({})
  const mountedRef = React.useRef(true)
  const rememberColumns = React.useCallback((id: string, state: ColumnState) => {
    columnsRef.current[id] = state
    // The person may have left the page while the request was in flight.
    if (!mountedRef.current) return
    setColumns((previous) => ({ ...previous, [id]: state }))
  }, [])

  React.useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  // Fetches one table's columns and keeps what came back.
  //
  // The tenant gate answers a refusal with `200` and a body that has no
  // `schema`, so a refusal is read before the schema is. Without that, the
  // `.map` below threw inside the handler and the table stayed "loading"
  // for good.
  const fetchColumns = React.useCallback(
    (id: string) => {
      rememberColumns(id, { status: "loading" })
      void assetService.getAsset(id).then(
        (detail) => {
          const refusal = catalogRefusal(detail)
          if (refusal) {
            rememberColumns(id, { status: "failed", reason: refusal.reason })
            return
          }
          rememberColumns(id, {
            status: "ready",
            columns: detail.schema.map((column) => ({
              name: column.name,
              dataType: column.dataType,
            })),
          })
        },
        // The request failed: the table is kept as a failure, and the
        // editor says so instead of asking again.
        () => rememberColumns(id, { status: "failed" })
      )
    },
    [rememberColumns]
  )

  React.useEffect(() => {
    if (assetsState.status !== "success") return
    for (const named of tablesInStatement(sql, tables)) {
      // The cache answers "has this table been asked for yet?": a table
      // named twice is fetched once, and a table that failed is not asked
      // for again on a keystroke.
      if (named.id in columnsRef.current) continue
      fetchColumns(named.id)
    }
  }, [sql, tables, assetsState.status, fetchColumns])

  // The schema browser asks for a table's columns when a person expands it.
  // It shares the cache above, so a table the statement already named is not
  // fetched twice. Only a person pressing "Retry" (`retry`) may fetch a
  // failed table again: a keystroke never does.
  const loadColumns = React.useCallback(
    (id: string, options: { retry?: boolean } = {}) => {
      const cached = columnsRef.current[id]
      if (cached && !(options.retry && cached.status === "failed")) return
      fetchColumns(id)
    },
    [fetchColumns]
  )

  // The editor's view is kept in a ref: the browser needs it at click time
  // only, and a state update would re-render the page for a view that
  // never changes what is drawn.
  const viewRef = React.useRef<EditorView | null>(null)
  const setEditorView = React.useCallback((view: EditorView) => {
    viewRef.current = view
  }, [])
  const insertAtCursor = React.useCallback((text: string) => {
    const view = viewRef.current
    if (!view) return
    const { from, to } = view.state.selection.main
    view.dispatch({
      changes: { from, to, insert: text },
      selection: { anchor: from + text.length },
    })
    view.focus()
  }, [])

  // The suggestions the editor offers. The source is built once and reads
  // the cache when it is asked, so a keystroke never waits for the network.
  const completionSource = React.useMemo(
    () =>
      createSqlCompletionSource({
        tables,
        columns: () => columnsRef.current,
      }),
    [tables]
  )
  const completionExtensions = React.useMemo(
    () => [sqlCompletionExtension(completionSource)],
    [completionSource]
  )

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
  }, [setSql])

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
  const loadSql = React.useCallback(
    (next: string) => {
      setSql(next)
      setTab("sql")
    },
    [setSql]
  )

  const generate = React.useCallback(async () => {
    const out = await generateAct.run(question)
    if (out) loadSql(out.sql)
  }, [generateAct, question, loadSql])

  const runQuery = React.useCallback(async () => {
    if (!hasStatement(sql)) return
    // A run means the person has moved on from the proposal; its diff is stale.
    setFixResult(null)
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
  }, [question, setSql])

  // Explain: the chat reads the statement in words and runs nothing.
  const explainRun = React.useCallback(async () => {
    if (!hasStatement(sql)) return
    setExplainBusy(true)
    setExplainError(null)
    setExplainResult(null)
    try {
      setExplainResult(await queryService.explainSql(sql, engine))
    } catch (e) {
      setExplainError(toServiceError(e))
    } finally {
      setExplainBusy(false)
    }
  }, [sql, engine])

  // Fix: the chat proposes a corrected statement for the failed one. The
  // proposal waits in `fixResult` until the person accepts or rejects it.
  const fixRun = React.useCallback(
    async (error: string) => {
      setFixBusy(true)
      setFixError(null)
      setFixResult(null)
      try {
        setFixResult(await queryService.proposeFix(sql, engine, error))
      } catch (e) {
        setFixError(toServiceError(e))
      } finally {
        setFixBusy(false)
      }
    },
    [sql, engine]
  )
  // Accept puts the proposal in the editor. It does not run it: the person
  // presses "Run query" to see its rows, as with any statement they typed.
  const fixAccept = React.useCallback(() => {
    if (fixResult && "sql" in fixResult) setSql(fixResult.sql)
  }, [fixResult, setSql])
  const fixReject = React.useCallback(() => setFixResult(null), [])

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
    assetsState,
    tables,
    columns,
    loadColumns,
    setEditorView,
    insertAtCursor,
    completionSource,
    completionExtensions,
    estimatable,
    generateAct,
    estimateAct,
    runAct,
    saveAct,
    savedState,
    historyState,
    agent: { busy: agentBusy, result: agentResult, error: agentError, ask },
    explain: {
      busy: explainBusy,
      result: explainResult,
      error: explainError,
      run: explainRun,
    },
    fix: {
      busy: fixBusy,
      result: fixResult,
      error: fixError,
      run: fixRun,
      accept: fixAccept,
      reject: fixReject,
    },
    loadSql,
    generate,
    runQuery,
    reEstimate: () => void runEstimate(sql),
    save,
  }
}
