"use client"

import * as React from "react"

import { Prec } from "@codemirror/state"
import { EditorView, keymap } from "@codemirror/view"

import { useDebouncedCallback } from "@/hooks/use-debounced-callback"
import { useService, useServiceAction } from "@/hooks/use-service"
import { catalogRefusal } from "@/lib/catalog-refusal"
import { notifyInfo } from "@/lib/notify"
import { newRunId } from "@/lib/run-id"
import {
  MAX_QUERY_TABS,
  readTabs,
  writeTabs,
  type QueryTab,
  type QueryTabs,
} from "@/lib/query-draft"
import {
  tablesInStatement,
  type ColumnState,
  type CompletionTable,
} from "@/lib/sql-completion"
import {
  hasStatement,
  MAX_STATEMENTS_PER_RUN,
  splitStatements,
  type SqlStatement,
} from "@/lib/sql-text"
import { assetService, queryService } from "@/services"
import { toServiceError, type ServiceError } from "@/services/errors"
import type {
  NlAnswer,
  QueryEngine,
  QueryResult,
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
 * The id of the tab that is there before anything is read from storage.
 * The draft that came before this had tabs is read as the tab with this
 * id, so the two live side by side in the browser.
 */
const FIRST_TAB_ID = "tab-1"

/**
 * The shape of a catalog id that is a SQL table name: `silver.orders`.
 *
 * A Bronze asset's id is a registry slug, not a table name, and its Trino
 * name sits on the asset's detail rather than on the list.
 */
const SQL_TABLE_NAME = /^[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*$/

export type StudioTab = "nl" | "sql"

/**
 * What one statement of a run did.
 *
 * The server runs one statement per request, so a buffer with several
 * statements is a batch of requests. Each statement keeps its own outcome,
 * so a failure in the middle of the batch does not hide the rows the
 * statements before it returned.
 */
export type StatementRun =
  | { status: "pending"; statement: SqlStatement }
  | { status: "running"; statement: SqlStatement }
  | { status: "done"; statement: SqlStatement; result: QueryResult }
  | { status: "error"; statement: SqlStatement; error: ServiceError }
  | { status: "not-run"; statement: SqlStatement }

/** A range of the buffer, in the buffer's own offsets. */
type BufferRange = { from: number; to: number }

/**
 * The statement the single-statement actions work on.
 *
 * The selection when it holds exactly one statement, else the tab's only
 * statement when it holds exactly one. `null` when neither: several
 * statements and no selection is a choice the person has to make.
 */
function targetStatementOf(
  sql: string,
  selection: BufferRange | null
): SqlStatement | null {
  if (selection) {
    const picked = sql.slice(selection.from, selection.to)
    const parts = splitStatements(picked)
    if (parts.length === 1) {
      const only = parts[0]
      // The offsets of a statement inside the selection are the offsets of
      // the picked text; the buffer's own are the selection's plus those.
      return {
        text: only.text,
        from: selection.from + only.from,
        to: selection.from + only.to,
      }
    }
    // A selection that holds more than one statement is not one statement:
    // the action would have to guess which one the person meant.
    if (parts.length > 1) return null
  }
  const list = splitStatements(sql)
  return list.length === 1 ? list[0] : null
}

/**
 * The batch of requests that is in flight: which tab it belongs to, how
 * far it got, and whether the history has caught up with it already.
 */
type Batch = {
  tabId: string
  index: number
  reloaded: boolean
  active: boolean
  /**
   * BI-5 (plan D5): the id the statement in flight was sent under, which Stop
   * names in the cancel call. One id per statement, so a cancel ends only the
   * statement that is running and never one that already finished.
   */
  runId: string | null
  engine: QueryEngine
}

/**
 * The name a new tab gets: the first "Query N" nobody is using, so a name
 * is never handed out twice after a tab was closed.
 */
function nextTabName(tabs: QueryTab[]): string {
  const taken = new Set<number>()
  for (const tab of tabs) {
    const match = /^Query (\d+)$/.exec(tab.name)
    if (match) taken.add(Number(match[1]))
  }
  let number = 1
  while (taken.has(number)) number += 1
  return `Query ${number}`
}

/**
 * An id for a new tab. It is stored with the tab, so it stays the same
 * across a reload; a browser without `crypto.randomUUID` (an origin that
 * is not secure) falls back to a counter.
 */
let tabCounter = 0
function newTabId(): string {
  const random = globalThis.crypto?.randomUUID?.()
  if (random) return random
  tabCounter += 1
  // Random entropy prevents id collisions between windows sharing the same
  // localStorage draft when creating a tab in the same millisecond.
  const entropy = Math.random().toString(36).slice(2, 8)
  return `tab-${Date.now().toString(36)}-${tabCounter}-${entropy}`
}

/**
 * All of Query Studio's state in one place: the two editors, the query tabs
 * inside the SQL editor, the pre-run estimate, the agent, saving, and the
 * draft that survives a reload.
 *
 * It lives apart from the page because the page had grown into 260 lines
 * of mixed concerns, and because the rules that matter here — don't
 * estimate on every keystroke, don't estimate a buffer that holds only a
 * comment — are easy to lose sight of inside JSX.
 */
export function useQueryStudio() {
  const [tab, setTab] = React.useState<StudioTab>("nl")
  const [question, setQuestion] = React.useState("")
  const [tabs, setTabs] = React.useState<QueryTab[]>(() => [
    { id: FIRST_TAB_ID, name: "Query 1", sql: STARTER_SQL, engine: "clickhouse" },
  ])
  const [activeTabId, setActiveTabId] = React.useState(FIRST_TAB_ID)
  // What every statement of every run did. It belongs to the tab that ran
  // it and stays in memory: it is not stored, so a reload starts with no
  // rows.
  const [runs, setRuns] = React.useState<Record<string, StatementRun[]>>({})
  // Which of a tab's results is open. It is kept per tab, so a tab that is
  // left and come back to shows the result it showed.
  const [runIndex, setRunIndex] = React.useState<Record<string, number>>({})
  // Whether a batch of requests is in flight.
  const [running, setRunning] = React.useState(false)

  // The open tab holds the SQL and the engine. The rest of the page keeps
  // using `sql` and `engine` and does not know there are tabs at all.
  const activeTab = React.useMemo(
    () => tabs.find((open) => open.id === activeTabId) ?? tabs[0] ?? null,
    [tabs, activeTabId]
  )
  const sql = activeTab?.sql ?? ""
  const engine = activeTab?.engine ?? "clickhouse"

  // A proposal is a diff against the editor as it was when it came back.
  // Once the editor changes, the diff describes something else, so it goes.
  const [fixResult, setFixResult] = React.useState<SqlFixOutcome | null>(null)
  // The statement the proposal is for. Accept puts the fix back where the
  // failure was, and nowhere else.
  const [fixStatement, setFixStatement] = React.useState<SqlStatement | null>(null)
  const [fixBusy, setFixBusy] = React.useState(false)
  const [fixError, setFixError] = React.useState<ServiceError | null>(null)
  const [explainResult, setExplainResult] = React.useState<string | null>(null)
  const [explainBusy, setExplainBusy] = React.useState(false)
  const [explainError, setExplainError] = React.useState<ServiceError | null>(null)
  const setSql = React.useCallback(
    (next: string) => {
      setTabs((previous) =>
        previous.map((open) => (open.id === activeTabId ? { ...open, sql: next } : open))
      )
      setFixResult(null)
      setFixStatement(null)
    },
    [activeTabId]
  )
  const setEngine = React.useCallback(
    (next: QueryEngine) => {
      setTabs((previous) =>
        previous.map((open) => (open.id === activeTabId ? { ...open, engine: next } : open))
      )
    },
    [activeTabId]
  )

  const generateAct = useServiceAction((signal, q: string) =>
    queryService.generateSql(q, signal)
  )
  const estimateAct = useServiceAction((signal, s: string) =>
    queryService.estimate(s, signal)
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

  // The selection the editor holds, in the buffer's own offsets.
  //
  // The view is not a React value, so an update listener tells the hook
  // when the person picked something: the single-statement actions read
  // this to know which statement they work on, and the buttons that are
  // disabled without one are drawn from it.
  const [selectionRange, setSelectionRange] = React.useState<BufferRange | null>(null)

  /**
   * The selection the editor holds right now, read from the view.
   *
   * The state above is a snapshot for the render; an action reads the view
   * itself, because the person may have moved the cursor since the last
   * render and the action must work on what is picked now.
   */
  const readSelection = React.useCallback((): BufferRange | null => {
    const view = viewRef.current
    if (!view) return null
    const { from, to } = view.state.selection.main
    if (from === to) return null
    return { from, to }
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

  // The key binding runs the selection when there is one and the whole tab
  // otherwise. It reads the action through a ref: the extension array must
  // keep its identity, or the editor is reconfigured on every keystroke.
  const runSelectionRef = React.useRef<() => void>(() => {})

  // The extensions the editor runs: the console's own suggestions, the
  // run binding, and the listener that keeps the hook's picture of the
  // selection up to date.
  const editorExtensions = React.useMemo(
    () => [
      sqlCompletionExtension(completionSource),
      Prec.highest(
        keymap.of([
          {
            key: "Mod-Enter",
            run: () => {
              runSelectionRef.current()
              // The binding answers for the key: the editor must not
              // insert a blank line where the run went.
              return true
            },
          },
        ])
      ),
      EditorView.updateListener.of((update) => {
        if (!update.selectionSet && !update.docChanged) return
        const { from, to } = update.view.state.selection.main
        setSelectionRange((previous) =>
          previous && previous.from === from && previous.to === to
            ? previous
            : from === to
              ? null
              : { from, to }
        )
      }),
    ],
    [completionSource]
  )

  // ── Draft ────────────────────────────────────────────────────────────
  // Restored after mount, never during render: `localStorage` does not
  // exist on the server, and reading it in render would make the first
  // paint differ from the markup Next sent.
  const hydratedRef = React.useRef(false)
  // Whether the tab bar still holds the empty tab that was there before
  // anything was read. A link that arrives while it does takes its place
  // instead of leaving a blank tab next to the query it opened.
  const placeholderRef = React.useRef(true)
  React.useEffect(() => {
    if (hydratedRef.current) return
    hydratedRef.current = true
    // A `?sql=` link (e.g. an asset's "Open in Query Studio") names the
    // query the user came here to run, so it opens in a tab of its own and
    // the draft it finds stays where it was.
    // `?engine=` rides along because the SQL is only valid on the engine it
    // was written for (e.g. `iceberg.bronze.*` names exist only on Trino).
    // Read from `window.location`, like the `?saved=` handoff below, to
    // avoid a `useSearchParams` Suspense boundary.
    const params = new URLSearchParams(window.location.search)
    const linkedSql = params.get("sql")
    const linkedEngine = params.get("engine")
    const engine: QueryEngine = linkedEngine === "trino" ? "trino" : "clickhouse"
    const hasLink = Boolean(linkedSql && hasStatement(linkedSql))
    const stored = readTabs()

    if (stored) {
      placeholderRef.current = false
      setQuestion(stored.question)
      setTab(stored.tab)
      if (hasLink && linkedSql) {
        const linked: QueryTab = {
          id: newTabId(),
          name: nextTabName(stored.tabs),
          sql: linkedSql,
          engine,
        }
        setTabs([...stored.tabs, linked])
        setActiveTabId(linked.id)
        setTab("sql")
        return
      }
      setTabs(stored.tabs)
      setActiveTabId(stored.activeId)
      return
    }

    if (hasLink && linkedSql) {
      const linked: QueryTab = {
        id: newTabId(),
        name: "Query 1",
        sql: linkedSql,
        engine,
      }
      placeholderRef.current = false
      setTabs([linked])
      setActiveTabId(linked.id)
      setTab("sql")
    }
  }, [])

  const persistDraft = useDebouncedCallback((next: QueryTabs) => {
    writeTabs(next)
  }, DRAFT_DEBOUNCE_MS)
  React.useEffect(() => {
    if (!hydratedRef.current) return
    persistDraft({ tabs, activeId: activeTabId, question, tab })
  }, [tabs, activeTabId, question, tab, persistDraft])

  // ── Statements ───────────────────────────────────────────────────────
  // The statements of the open tab. The server runs one statement per
  // request, so a buffer with several of them is a batch of requests.
  const statements = React.useMemo(() => splitStatements(sql), [sql])

  // The statement the single-statement actions work on. It is `null` when
  // the tab holds several statements and nothing is picked: the action
  // would have to guess which one the person meant.
  const targetStatement = React.useMemo(
    () => targetStatementOf(sql, selectionRange),
    [sql, selectionRange]
  )
  // Whether there is anything to run at all.
  const runnable = statements.length > 0

  // ── Estimate ─────────────────────────────────────────────────────────
  const runEstimate = estimateAct.run
  const estimateDebounced = useDebouncedCallback((next: string) => {
    void runEstimate(next)
  }, ESTIMATE_DEBOUNCE_MS)
  // The estimate is asked for one statement at a time: the server plans a
  // statement, not a buffer with several of them.
  const estimatable = targetStatement !== null
  const targetText = targetStatement?.text ?? null
  React.useEffect(() => {
    // A buffer holding only the starter comment has nothing to estimate,
    // and asking anyway put an EXPLAIN ESTIMATE on ClickHouse for every
    // keystroke, including the ones that typed the comment.
    if (targetText !== null) estimateDebounced(targetText)
  }, [targetText, estimateDebounced])

  // ── Actions ──────────────────────────────────────────────────────────
  const loadSql = React.useCallback(
    (next: string) => {
      setSql(next)
      setTab("sql")
    },
    [setSql]
  )

  /**
   * Opens a tab and makes it the open one. `false` when the browser
   * already holds as many tabs as it keeps: the tab bar turns that into a
   * message, because a button that does nothing is a dead end.
   */
  const addTab = React.useCallback(
    (options?: { sql?: string; engine?: QueryEngine }): boolean => {
      if (tabs.length >= MAX_QUERY_TABS) return false
      const opened: QueryTab = {
        id: newTabId(),
        name: nextTabName(tabs),
        sql: options?.sql ?? STARTER_SQL,
        engine: options?.engine ?? "clickhouse",
      }
      setTabs((previous) =>
        previous.length >= MAX_QUERY_TABS ? previous : [...previous, opened]
      )
      setActiveTabId(opened.id)
      setFixResult(null)
      return true
    },
    [tabs]
  )

  /** Renames a tab. An empty name is refused: the tab keeps the one it had. */
  const renameTab = React.useCallback((id: string, name: string): boolean => {
    const trimmed = name.trim()
    if (!trimmed) return false
    setTabs((previous) =>
      previous.map((open) => (open.id === id ? { ...open, name: trimmed } : open))
    )
    return true
  }, [])

  /**
   * Closes a tab and opens its neighbour: the one before it, or the one
   * after it when it was the first. Closing the last tab leaves one empty
   * tab, so the bar is never empty and the editor is never gone.
   */
  const closeTab = React.useCallback(
    (id: string) => {
      const index = tabs.findIndex((open) => open.id === id)
      if (index === -1) return
      const closing = tabs[index]
      const rest = tabs.filter((open) => open.id !== id)
      if (rest.length === 0) {
        setTabs([{ ...closing, sql: "" }])
        setActiveTabId(closing.id)
      } else {
        const neighbour = tabs[index - 1] ?? tabs[index + 1]
        setTabs(rest)
        setActiveTabId(neighbour.id)
      }
      // The closed tab's rows go with it: a tab that never ran shows
      // nothing, and the empty tab that is left behind is not the tab that
      // ran.
      setRuns((previous) => {
        if (!(id in previous)) return previous
        const next = { ...previous }
        delete next[id]
        return next
      })
      setRunIndex((previous) => {
        if (!(id in previous)) return previous
        const next = { ...previous }
        delete next[id]
        return next
      })
      setFixResult(null)
    },
    [tabs]
  )

  /** Opens another tab. A proposal belongs to the statement it was made for. */
  const setActiveTab = React.useCallback((id: string) => {
    setActiveTabId(id)
    setFixResult(null)
  }, [])

  const generate = React.useCallback(async () => {
    const out = await generateAct.run(question)
    if (out) loadSql(out.sql)
  }, [generateAct, question, loadSql])

  // The batch that is in flight, and the request that stops it.
  const batchRef = React.useRef<Batch | null>(null)
  const runControllerRef = React.useRef<AbortController | null>(null)
  React.useEffect(() => () => runControllerRef.current?.abort(), [])

  /**
   * Runs a list of statements, one request each, in the order written.
   *
   * The batch stops at the first failure: the statements after it did not
   * run, and they say so. The controller is the batch's own, because a
   * service action aborts the call before it — and here every statement
   * after the first one is a call that must not cancel the one before it.
   */
  const runBatch = React.useCallback(
    async (tabId: string, list: SqlStatement[], runEngine: QueryEngine) => {
      // A run means the person has moved on from the proposal; its diff is stale.
      setFixResult(null)
      setFixStatement(null)
      setRuns((previous) => ({
        ...previous,
        [tabId]: list.map(
          (statement) => ({ status: "pending", statement }) as StatementRun
        ),
      }))
      setRunIndex((previous) => ({ ...previous, [tabId]: 0 }))

      const controller = new AbortController()
      runControllerRef.current = controller
      const batch: Batch = {
        tabId,
        index: 0,
        reloaded: false,
        active: true,
        runId: null,
        engine: runEngine,
      }
      batchRef.current = batch
      setRunning(true)

      const markNotRun = (from: number) => {
        setRuns((previous) => {
          const current = previous[tabId] ?? []
          return {
            ...previous,
            [tabId]: current.map((run, at) =>
              at >= from ? { status: "not-run", statement: run.statement } : run
            ),
          }
        })
      }

      try {
        for (let index = 0; index < list.length; index += 1) {
          if (controller.signal.aborted) {
            markNotRun(index)
            break
          }
          batch.index = index
          setRuns((previous) => ({
            ...previous,
            [tabId]: (previous[tabId] ?? []).map((run, at) =>
              at === index ? { status: "running", statement: run.statement } : run
            ),
          }))
          try {
            const runId = newRunId()
            batch.runId = runId
            const result = await queryService.run(
              list[index].text,
              { engine: runEngine, runId },
              controller.signal
            )
            setRuns((previous) => ({
              ...previous,
              [tabId]: (previous[tabId] ?? []).map((run, at) =>
                at === index
                  ? { status: "done", statement: run.statement, result }
                  : run
              ),
            }))
          } catch (error) {
            if (controller.signal.aborted) {
              // The person stopped the batch: the statement that was in
              // flight did not finish, and neither did the ones after it.
              markNotRun(index)
              break
            }
            const serviceError = toServiceError(error)
            const isPermissionRefusal = serviceError.code === "permission_denied"
            // A permission refusal denies execution rights for the entire
            // batch, so subsequent statements are marked refused without
            // calling the engine.
            setRuns((previous) => ({
              ...previous,
              [tabId]: (previous[tabId] ?? []).map((run, at) =>
                (isPermissionRefusal ? at >= index : at === index)
                  ? {
                      status: "error",
                      statement: run.statement,
                      error: serviceError,
                    }
                  : run
              ),
            }))
            if (!isPermissionRefusal) markNotRun(index + 1)
            break
          }
        }
      } finally {
        batch.active = false
        setRunning(false)
        if (runControllerRef.current === controller) runControllerRef.current = null
        // Every run is recorded, including the ones that failed, so the rail
        // has to catch up — otherwise the query you just ran is missing from
        // your own history until a reload. Once for the whole batch.
        if (!batch.reloaded) {
          batch.reloaded = true
          historyState.reload()
        }
      }
    },
    [historyState]
  )

  /**
   * Runs every statement of the open tab, one after another.
   *
   * A batch of more than the allowed number of statements is refused
   * before anything runs: twenty-one requests the person did not mean to
   * make are not a kindness.
   */
  const runQuery = React.useCallback(async () => {
    if (statements.length === 0) return
    if (statements.length > MAX_STATEMENTS_PER_RUN) {
      notifyInfo(
        `At most ${MAX_STATEMENTS_PER_RUN} statements are run at once`,
        "Split the buffer into smaller queries and run them one after another."
      )
      return
    }
    await runBatch(activeTabId, statements, engine)
  }, [statements, activeTabId, engine, runBatch])

  /**
   * Runs the selection when there is one, and the whole tab otherwise.
   *
   * The picked text is sent as it stands: the person marked what they
   * meant, and the server is the one that says whether it can run it.
   */
  const runSelectionOrAll = React.useCallback(async () => {
    // A batch that is in flight owns the engine: a second one would
    // interleave its requests with the first one's. The controller is read
    // here, not the state, because the state arrives a render late and two
    // quick presses would both get through.
    if (runControllerRef.current) return
    const selection = readSelection()
    if (selection) {
      const picked = sql.slice(selection.from, selection.to)
      if (hasStatement(picked)) {
        await runBatch(
          activeTabId,
          [{ text: picked, from: selection.from, to: selection.to }],
          engine
        )
        return
      }
    }
    await runQuery()
  }, [readSelection, sql, activeTabId, engine, runBatch, runQuery])

  /**
   * Stops the batch: the statement in flight and the ones after it did not
   * run, and the batch is over.
   */
  const stopRun = React.useCallback(() => {
    const batch = batchRef.current
    // BI-5: ask the server to end the statement's query first. Best effort (a
    // Trino run has no id to send: the built-in engine is the only one that can
    // stop a query, see the feature page), and Stop aborts the request either
    // way, so the page never waits on a query the person gave up on.
    if (batch?.active && batch.runId && batch.engine === "clickhouse") {
      void queryService.cancel(batch.runId).catch(() => undefined)
    }
    runControllerRef.current?.abort()
    // The controller goes now, not when the request settles: a request
    // that never comes back would otherwise hold the console shut.
    runControllerRef.current = null
    if (!batch || !batch.active) return
    setRuns((previous) => {
      const current = previous[batch.tabId] ?? []
      return {
        ...previous,
        [batch.tabId]: current.map((run, at) =>
          at >= batch.index ? { status: "not-run", statement: run.statement } : run
        ),
      }
    })
    setRunning(false)
    if (!batch.reloaded) {
      batch.reloaded = true
      historyState.reload()
    }
  }, [historyState])

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

  // Explain: the chat reads one statement in words and runs nothing.
  const explainRun = React.useCallback(async () => {
    const target = targetStatementOf(sql, readSelection())
    if (!target) return
    setExplainBusy(true)
    setExplainError(null)
    setExplainResult(null)
    try {
      setExplainResult(await queryService.explainSql(target.text, engine))
    } catch (e) {
      setExplainError(toServiceError(e))
    } finally {
      setExplainBusy(false)
    }
  }, [sql, engine, readSelection])

  // Fix: the chat proposes a corrected statement for the failed one. The
  // proposal waits in `fixResult` until the person accepts or rejects it.
  //
  // The statement it is made for is remembered with it: Accept puts the
  // fix back where the failure was, and nowhere else.
  const fixRun = React.useCallback(
    async (error: string, statement?: SqlStatement) => {
      const target = statement ?? targetStatementOf(sql, readSelection())
      setFixBusy(true)
      setFixError(null)
      setFixResult(null)
      setFixStatement(target)
      try {
        setFixResult(
          await queryService.proposeFix(target ? target.text : sql, engine, error)
        )
      } catch (e) {
        setFixError(toServiceError(e))
      } finally {
        setFixBusy(false)
      }
    },
    [sql, engine, readSelection]
  )
  // Accept puts the proposal in the editor. It does not run it: the person
  // presses "Run query" to see its rows, as with any statement they typed.
  //
  // Only the statement the proposal was made for is replaced. The rest of
  // the buffer is not the assistant's business.
  const fixAccept = React.useCallback(() => {
    const outcome = fixResult
    if (!outcome || !("sql" in outcome)) return
    const statement = fixStatement
    // No statement: the proposal was made for the whole buffer, as it was
    // before a buffer could hold several statements.
    if (!statement) {
      setSql(outcome.sql)
      return
    }
    const view = viewRef.current
    const buffer = view ? view.state.doc.toString() : sql
    // The person edited the statement since the proposal was made: the fix
    // would land on text that is no longer there, so nothing changes.
    if (buffer.slice(statement.from, statement.to) !== statement.text) return
    if (view) {
      view.dispatch({
        changes: { from: statement.from, to: statement.to, insert: outcome.sql },
      })
      return
    }
    setSql(buffer.slice(0, statement.from) + outcome.sql + buffer.slice(statement.to))
  }, [fixResult, fixStatement, sql, setSql])
  const fixReject = React.useCallback(() => {
    setFixResult(null)
    setFixStatement(null)
  }, [])

  // Handoff from Saved Queries: /query-studio?saved=<id> opens that query
  // in a tab of its own, so the tabs that were there stay where they were.
  // Read from window.location to avoid a useSearchParams Suspense boundary.
  const appliedSavedRef = React.useRef(false)
  React.useEffect(() => {
    if (appliedSavedRef.current || savedState.status !== "success") return
    appliedSavedRef.current = true
    const savedId = new URLSearchParams(window.location.search).get("saved")
    if (!savedId) return
    const match = savedState.data.find((q) => q.id === savedId)
    if (!match) return
    const opened: QueryTab = {
      id: newTabId(),
      // The saved query's own name is a better one than "Query 3".
      name: match.title.trim() || nextTabName(tabs),
      sql: match.sql,
      engine: "clickhouse",
    }
    if (placeholderRef.current && tabs.length === 1) {
      // Nothing was stored and no link came first: the saved query takes
      // the empty tab's place instead of sitting next to it.
      placeholderRef.current = false
      setTabs([{ ...tabs[0], sql: opened.sql, name: opened.name }])
      setActiveTabId(tabs[0].id)
    } else {
      placeholderRef.current = false
      setTabs([...tabs, opened])
      setActiveTabId(opened.id)
    }
    setTab("sql")
  }, [savedState, tabs])

  // The rows on screen for each tab: the result of the statement that is
  // open on it. The per-statement state behind it is `runs`.
  const results = React.useMemo<Record<string, QueryResult | null>>(() => {
    const out: Record<string, QueryResult | null> = {}
    for (const [tabId, list] of Object.entries(runs)) {
      const open = list[runIndex[tabId] ?? 0]
      if (!open) continue
      out[tabId] = open.status === "done" ? open.result : null
    }
    return out
  }, [runs, runIndex])

  // The statements of the open tab and the one of them that is open.
  const activeRuns = runs[activeTabId] ?? []
  const activeRunIndex = runIndex[activeTabId] ?? 0
  const activeRun = activeRuns[activeRunIndex] ?? null

  /** Opens another statement's result of the open tab. */
  const setActiveRunIndex = React.useCallback(
    (index: number) => {
      setRunIndex((previous) => ({ ...previous, [activeTabId]: index }))
    },
    [activeTabId]
  )

  // The key binding reads the action through a ref, so the extension array
  // keeps its identity and the editor is not reconfigured on every
  // keystroke.
  React.useEffect(() => {
    runSelectionRef.current = () => void runSelectionOrAll()
  })

  return {
    tab,
    setTab,
    question,
    setQuestion,
    sql,
    setSql,
    engine,
    setEngine,
    tabs,
    activeTabId,
    addTab,
    renameTab,
    closeTab,
    setActiveTab,
    statements,
    runs,
    activeRuns,
    activeRunIndex,
    setActiveRunIndex,
    activeRun,
    results,
    running,
    stopRun,
    runnable,
    targetStatement,
    assetsState,
    tables,
    columns,
    loadColumns,
    setEditorView,
    insertAtCursor,
    completionSource,
    editorExtensions,
    estimatable,
    generateAct,
    estimateAct,
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
      statement: fixStatement,
      run: fixRun,
      accept: fixAccept,
      reject: fixReject,
    },
    loadSql,
    generate,
    runQuery,
    runSelectionOrAll,
    reEstimate: () => {
      if (targetText !== null) void runEstimate(targetText)
    },
    save,
  }
}
