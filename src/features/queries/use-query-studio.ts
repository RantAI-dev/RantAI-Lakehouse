"use client"

import * as React from "react"

import { Prec } from "@codemirror/state"
import { EditorView, keymap } from "@codemirror/view"

import {
  assetQueryTarget,
  BRONZE_NAMESPACE,
  listedBronzeTable,
  trinoBronzeName,
} from "@/lib/asset-query"
import { isIcebergCandidate } from "@/lib/lakehouse-view"
import { useDebouncedCallback } from "@/hooks/use-debounced-callback"
import { useService, useServiceAction } from "@/hooks/use-service"
import { levelAtLeast } from "@/lib/access-level"
import { catalogRefusal } from "@/lib/catalog-refusal"
import { notifyInfo } from "@/lib/notify"
import { readableErrorMessage } from "@/lib/notify-message"
import { newRunId } from "@/lib/run-id"
import {
  dropLegacyDraft,
  MAX_QUERY_TABS,
  readTabs,
  writeTabs,
  type QueryTab,
  type QueryTabs,
} from "@/lib/query-draft"
import {
  activeParameters,
  fieldsNamedBy,
  firstMissingValueName,
  paramInputs,
  reconcileDefinitions,
  replaceDefinition,
} from "@/lib/query-param-names"
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
import { assetService, lakehouseService, queryService } from "@/services"
import { toServiceError, type ServiceError } from "@/services/errors"
import type {
  NlAnswer,
  QueryEngine,
  QueryParamDef,
  QueryParamInputs,
  QueryParamValue,
  QueryResult,
  SavedQuery,
  SqlFixOutcome,
} from "@/services/contracts/queries"
import {
  createSqlCompletionSource,
  sqlCompletionExtension,
} from "./sql-completion-source"
import { useQueryDownload } from "./use-query-download"

/** What the editor holds before anyone types. */
export const STARTER_SQL = "-- Write SQL here, or generate it from a question"

/** How long to wait after the last keystroke before asking for an estimate. */
const ESTIMATE_DEBOUNCE_MS = 500

/** How long to wait before writing the draft to storage. */
const DRAFT_DEBOUNCE_MS = 400

/** What a tab without definitions or values holds; one object, so the memo that reads it stays put. */
const NO_DEFINITIONS: QueryParamDef[] = []
const NO_VALUES: Record<string, QueryParamValue> = {}
/** What a request carries for SQL with no placeholder: nothing, one object so a memo that reads it stays put. */
const NO_INPUTS: QueryParamInputs = {}
const NO_NAMES: ReadonlySet<string> = new Set()

/**
 * The id of the tab that is there before anything is read from storage.
 * The draft that came before this had tabs is read as the tab with this
 * id, so the two live side by side in the browser.
 */
const FIRST_TAB_ID = "tab-1"

/**
 * What the studio says when a `?saved=` link names a query the caller may
 * not see (or one that is in the trash).
 *
 * The console's own sentence, not the server's: the link is the same for
 * a query that was deleted and for one nobody may read, and the person
 * following it learns that the query is not there for them (`QS-3` C5).
 */
export const SAVED_QUERY_UNAVAILABLE = "This query is not available to you."

/**
 * The shape of a catalog id that is a SQL table name: `silver.orders`.
 *
 * A Bronze asset's id is a registry slug, not a table name, and its Trino
 * name sits on the asset's detail rather than on the list.
 */
const SQL_TABLE_NAME = /^[A-Za-z_][A-Za-z0-9_]*\.[A-Za-z_][A-Za-z0-9_]*$/

export type StudioTab = "nl" | "sql"

/**
 * What the studio knows about a Bronze table's Iceberg table: that it is
 * there (`there`), that it is not (`missing` — a read that answered "not
 * found"), or that the read itself failed (`failed`), which says nothing
 * about the table and is tried again the next time the entry opens.
 *
 * A table with no proof yet has none either: the registry's `tableName`
 * says a Bronze table might exist, not that it does, and a Trino tab on a
 * table that is not there answers "table does not exist" (QS-4 D3, owner
 * 2026-11-09). Holding a failed read as `missing` would state as fact
 * what was only a failed request (#448 review SHOULD-FIX).
 */
export type TableProof = { status: "there" | "missing" | "failed" }

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
  | {
      status: "done"
      statement: SqlStatement
      result: QueryResult
      /**
       * The id the statement ran under. The download of the result names it,
       * so the file's query on the engine belongs to the same run and Stop
       * reaches it.
       */
      runId: string
    }
  | { status: "error"; statement: SqlStatement; error: ServiceError }
  | {
      status: "not-run"
      statement: SqlStatement
      /**
       * QA LH-QS-4-10: why it did not run. A failure of an earlier statement
       * stops the batch; so does the person's Stop, and the page must not
       * tell them a failure did it.
       */
      reason: "failure" | "stopped"
    }

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
 * How many Bronze tables are proven at once when the list loads (LH-QS-4-08).
 * Four keeps a long list moving without a burst of requests from one page.
 */
const MAX_PROOF_READS = 4

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
 *
 * `draftKey` is the storage key of the person whose tabs these are
 * (`queryDraftKey`), or `null` while the signed-in person is not known. One
 * hook instance has one owner for its whole life (`QS-8`, decision 12): a
 * later, different key is ignored here because the page mounts a new
 * workspace for it — the old one flushes its tabs under its own key as it
 * unmounts. `null` reads and writes nothing (fail closed).
 */
export function useQueryStudio(draftKey: string | null) {
  // The owner is latched: the session may answer after this hook has mounted,
  // so the first non-null key is taken and every later one is not.
  const [ownKey, setOwnKey] = React.useState<string | null>(draftKey)
  React.useEffect(() => {
    if (ownKey === null && draftKey !== null) setOwnKey(draftKey)
  }, [ownKey, draftKey])
  const [tab, setTab] = React.useState<StudioTab>("nl")
  const [question, setQuestion] = React.useState("")
  const [tabs, setTabs] = React.useState<QueryTab[]>(() => [
    {
      id: FIRST_TAB_ID,
      name: "Query 1",
      sql: STARTER_SQL,
      engine: "clickhouse",
      parameters: [],
      values: {},
    },
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

  // The moment Run was pressed, and the clock the elapsed time is read
  // from. The clock ticks only while a run is in flight: a timer that keeps
  // running on an idle page is a timer nobody looks at.
  const [startedAt, setStartedAt] = React.useState<number | null>(null)
  const [now, setNow] = React.useState(() => Date.now())
  React.useEffect(() => {
    if (!running) return
    const tick = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(tick)
  }, [running])

  // The download of a result: it is a query on the engine like any other, so
  // it belongs with the run it belongs to.
  const download = useQueryDownload()
  const stopDownload = download.stop

  // The open tab holds the SQL and the engine. The rest of the page keeps
  // using `sql` and `engine` and does not know there are tabs at all.
  const activeTab = React.useMemo(
    () => tabs.find((open) => open.id === activeTabId) ?? tabs[0] ?? null,
    [tabs, activeTabId]
  )
  const sql = activeTab?.sql ?? ""
  const engine = activeTab?.engine ?? "clickhouse"

  // The `{{name}}` parameters of the open tab (`QS-5`). The tab keeps every
  // definition it has had; the fields are the ones whose name the SQL uses
  // now, so a name cut out of the SQL loses its field and not its
  // definition or value. Read on `sql` and the definitions only, so a
  // keystroke that changes neither the names nor the definitions does not
  // rebuild the list the fields are drawn from.
  const parameterDefinitions = activeTab?.parameters ?? NO_DEFINITIONS
  const parameterValues = activeTab?.values ?? NO_VALUES
  const parameters = React.useMemo(
    () => activeParameters(sql, parameterDefinitions),
    [sql, parameterDefinitions]
  )
  // What a run carries for them: the values and the definitions of the names
  // the SQL uses, or nothing when it uses none (`paramInputs`).
  const runInputs = React.useMemo(
    () => (parameters.length === 0 ? NO_INPUTS : paramInputs(parameters, parameterValues)),
    [parameters, parameterValues]
  )

  // The saved query the open tab came from, when it came from one (`QS-3`
  // C4). Its level says what "Save" may do with it.
  const activeSaved = React.useMemo(() => {
    const savedId = activeTab?.savedId
    if (!savedId) return null
    return { id: savedId, level: activeTab?.level }
  }, [activeTab])
  // "Save" changes the saved query when the caller may edit it, and makes a
  // copy of it otherwise: a level the console cannot read widens nothing.
  const savesSavedQuery = activeSaved !== null && levelAtLeast(activeSaved.level, "edit")

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
        previous.map((open) =>
          open.id === activeTabId
            ? {
                ...open,
                sql: next,
                parameters: reconcileDefinitions(next, open.parameters, open.values),
              }
            : open
        )
      )
      setFixResult(null)
      setFixStatement(null)
    },
    [activeTabId]
  )
  // The fields the server's last refusal of a run named (`QS-5`). The wire
  // gives a sentence and no field, so `fieldsNamedBy` reads the names out of
  // it. A field is marked until its value or definition changes, and every
  // mark goes when the next run starts.
  const [refused, setRefused] = React.useState<{
    tabId: string
    names: ReadonlySet<string>
  } | null>(null)
  const unmark = React.useCallback(
    (name: string) => {
      setRefused((previous) => {
        if (!previous || previous.tabId !== activeTabId || !previous.names.has(name)) return previous
        const names = new Set(previous.names)
        names.delete(name)
        return names.size === 0 ? null : { tabId: previous.tabId, names }
      })
    },
    [activeTabId]
  )
  /** Gives the parameter a value; `undefined` takes the value away (the default applies). */
  const setParameterValue = React.useCallback(
    (name: string, value: QueryParamValue | undefined) => {
      unmark(name)
      setTabs((previous) =>
        previous.map((open) => {
          if (open.id !== activeTabId) return open
          const values = { ...open.values }
          if (value === undefined) delete values[name]
          else values[name] = value
          return { ...open, values }
        })
      )
    },
    [activeTabId, unmark]
  )
  /** Replaces the definition of the parameter it names; a value that no longer fits its type goes. */
  const setParameterDefinition = React.useCallback(
    (def: QueryParamDef) => {
      unmark(def.name)
      setTabs((previous) =>
        previous.map((open) =>
          open.id === activeTabId
            ? { ...open, ...replaceDefinition(open.parameters, open.values, def) }
            : open
        )
      )
    },
    [activeTabId, unmark]
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
  const estimateAct = useServiceAction(
    (signal, s: string, inputs: QueryParamInputs, engine: QueryEngine) =>
      // `QS-6` (decision 7): a ClickHouse estimate asks what it asked before
      // `QS-5` and `QS-6` — no options, or the inputs alone; any other engine
      // is named so the server can answer "no pre-run estimate" instead of
      // asking ClickHouse about Trino SQL.
      queryService.estimate(
        s,
        engine === "clickhouse"
          ? inputs.definitions
            ? inputs
            : undefined
          : { engine, ...(inputs.definitions !== undefined ? inputs : {}) },
        signal
      )
  )
  // The fields to mark: the ones the last run's refusal named, and the ones
  // the estimate's refusal names. An estimate's mark goes with its error,
  // when the next estimate starts.
  const estimateError = estimateAct.error
  const invalidParameters = React.useMemo<ReadonlySet<string>>(() => {
    const marked = new Set(refused && refused.tabId === activeTabId ? refused.names : NO_NAMES)
    if (estimateError?.code === "invalid_request") {
      const names = parameters.map((def) => def.name)
      for (const name of fieldsNamedBy(estimateError.message, names)) marked.add(name)
    }
    return marked.size === 0 ? NO_NAMES : marked
  }, [refused, activeTabId, estimateError, parameters])
  const savedState = useService((s) => queryService.listSaved(s), [])
  const historyState = useService((s) => queryService.listHistory(undefined, s), [])

  // The tables the catalog shows, from the real catalog so suggestions
  // cannot drift from what exists.
  //
  // A failure here is deliberately not shown as a page error: autocomplete
  // is a nicety, and Query Studio works without it.
  const assetsState = useService((s) => assetService.listAssets({}, s), [])

  // Whether the page is still there: a request that comes back after the
  // person left it must not set state.
  const mountedRef = React.useRef(true)
  React.useEffect(() => {
    mountedRef.current = true
    return () => {
      mountedRef.current = false
    }
  }, [])

  // Whether a table's Iceberg table is there, by catalog id. One read proves
  // it: the table's own detail (`getTableDetail`, on expand or on the
  // statement naming it), or — for a Trino tab — the `bronze` namespace's
  // table list, below. Only a proven table is offered (`QS-4` D3).
  const [proofs, setProofs] = React.useState<Record<string, TableProof>>({})
  const rememberProof = React.useCallback((id: string, proof: TableProof) => {
    // The person may have left the page while the request was in flight.
    if (!mountedRef.current) return
    setProofs((previous) => ({ ...previous, [id]: proof }))
  }, [])

  // The query target a table's detail named, keyed by the catalog id. The
  // detail is already fetched for a table a person expands or a statement
  // names; the same answer carries the SQL form.
  // A Bronze table also keeps its registry `tableName`: that is what its
  // Trino name is built from. Before the detail answers, the list row's
  // `tableKey` already names it (`listedBronzeTable`), so the Trino name is
  // known without expanding the entry (`QS-7`, `LH-QS-4-08`).
  const [targets, setTargets] = React.useState<
    Record<string, { target: ReturnType<typeof assetQueryTarget>; bronzeTable: string | null }>
  >({})

  // QA LH-QS-4-08 (QS-7), decision 11: one read of the `bronze` namespace's
  // table list proves every Bronze table the catalog lists, made once per
  // visit the first time a Trino tab is open. Before this, each entry had to
  // be expanded to be insertable, because only its own detail read proved it.
  const [trinoSeen, setTrinoSeen] = React.useState(false)
  React.useEffect(() => {
    if (engine === "trino") setTrinoSeen(true)
  }, [engine])
  const wantsBronzeListing =
    trinoSeen &&
    assetsState.status === "success" &&
    assetsState.data.some((asset) => listedBronzeTable(asset) !== null)
  const bronzeListing = useService(
    (s) =>
      wantsBronzeListing
        ? lakehouseService.listTables(BRONZE_NAMESPACE, undefined, s)
        : Promise.resolve(null),
    [wantsBronzeListing]
  )
  // The names the listing proves, or `null` when it has not answered: a
  // failed listing proves nothing, and is not shown as a page error —
  // expand-to-prove still works, as it did before.
  const listedBronzeNames = React.useMemo<ReadonlySet<string> | null>(() => {
    if (bronzeListing.status !== "success" || bronzeListing.data === null) return null
    return new Set(bronzeListing.data.map((table) => table.name))
  }, [bronzeListing.status, bronzeListing.data])
  // The proofs the schema browser and the completion read: the detail's own
  // answer where it exists (it is newer, so it wins), else the listing's.
  const tableProofs = React.useMemo<Record<string, TableProof>>(() => {
    if (listedBronzeNames === null) return proofs
    const next = { ...proofs }
    for (const asset of assetsState.data ?? []) {
      if (asset.id in next) continue
      // The detail wins when it answered — even with `null`, which says the
      // asset is not a Bronze candidate at all.
      const tableName =
        asset.id in targets ? targets[asset.id].bronzeTable : listedBronzeTable(asset)
      if (!tableName) continue
      next[asset.id] = listedBronzeNames.has(tableName)
        ? { status: "there" }
        : { status: "missing" }
    }
    return next
  }, [proofs, listedBronzeNames, assetsState.data, targets])

  // The tables in the shape the completion needs: the SQL name to insert,
  // the name a person reads, and every name the table answers to.
  //
  // A Bronze asset's registry slug is not a SQL name, so such a table's name
  // is the registry `tableName`: from the list row's `tableKey`, or from the
  // detail once it is loaded. A Bronze table has two names: the ClickHouse
  // one the server names (`queryTarget`, for the catalog page) and its Trino
  // name, built here. The studio offers only the Trino one, and only on a
  // Trino tab; on a ClickHouse tab the reference stays empty (`trinoOnly`
  // tells the schema browser why). An id that reads as a SQL name keeps
  // being the reference, as it always was. (QS-4 QA of 2026-11-05)
  const tables = React.useMemo<CompletionTable[]>(() => {
    if (assetsState.status !== "success") return []
    return assetsState.data.map((asset) => {
      const names = [asset.id, `${asset.namespace}.${asset.name}`]
      if (SQL_TABLE_NAME.test(asset.id)) {
        return { id: asset.id, label: asset.name, reference: asset.id, layer: asset.layer, names }
      }
      const detail = targets[asset.id]
      // Before the detail, the layer is all there is to tell a Bronze table.
      const trinoOnly = detail
        ? detail.bronzeTable !== null
        : asset.layer === "raw" || asset.layer === "bronze"
      if (trinoOnly) {
        // `QS-7`: the list row's `tableKey` names the table before the
        // detail does; the detail's own answer wins when both exist.
        const bronzeTable = detail ? detail.bronzeTable : listedBronzeTable(asset)
        const trino = bronzeTable ? trinoBronzeName(bronzeTable) : ""
        // Both spellings of a Trino name resolve to this table, whatever its id.
        if (bronzeTable) names.push(trino, `bronze.${bronzeTable}`)
        // The Trino name is offered only once the Iceberg table is proven to
        // be there: the registry's `tableName` says a Bronze table might
        // exist, not that it does (QS-4 D3, owner 2026-11-09).
        const proven = tableProofs[asset.id]?.status === "there"
        return {
          id: asset.id,
          label: asset.name,
          reference: engine === "trino" && proven ? trino : "",
          layer: asset.layer,
          names,
          trinoOnly: true,
        }
      }
      const target = detail?.target
      return {
        id: asset.id,
        label: asset.name,
        reference: target && (target.engine !== "trino" || engine === "trino") ? target.table : "",
        layer: asset.layer,
        names,
      }
    })
  }, [assetsState.status, assetsState.data, targets, tableProofs, engine])

  // The columns of the tables the buffer names: fetched once each and kept
  // for the session. A table that cannot be read is kept as a failure too,
  // so it is not asked for again on every keystroke.
  //
  // The ref holds the same cache as the state, because the completion
  // source reads it the moment a keystroke asks for suggestions, and a
  // state update is not there yet at that point.
  const [columns, setColumns] = React.useState<Record<string, ColumnState>>({})
  const columnsRef = React.useRef<Record<string, ColumnState>>({})
  const rememberColumns = React.useCallback((id: string, state: ColumnState) => {
    columnsRef.current[id] = state
    // The person may have left the page while the request was in flight.
    if (!mountedRef.current) return
    setColumns((previous) => ({ ...previous, [id]: state }))
  }, [])

  // The Iceberg table itself: a Bronze table's Trino name is offered only
  // once the table is proven to be there, and only a read that answered
  // "not found" proves it is not (QS-4 D3). Any other rejection failed, and
  // is held as failed so the entry offers a retry instead of a false "not
  // there" (#448 review SHOULD-FIX). The promise settles when the proof is
  // kept, which is what the list-load queue below waits for.
  const readProof = React.useCallback(
    (id: string, bronzeTable: string): Promise<void> =>
      lakehouseService.getTableDetail(BRONZE_NAMESPACE, bronzeTable).then(
        () => rememberProof(id, { status: "there" }),
        (error: unknown) =>
          rememberProof(
            id,
            toServiceError(error).code === "not_found" ? { status: "missing" } : { status: "failed" }
          )
      ),
    [rememberProof]
  )

  // Fetches one table's columns and keeps what came back. The promise
  // settles once the columns, and for a Bronze table its proof, are kept; it
  // never rejects.
  //
  // The tenant gate answers a refusal with `200` and a body that has no
  // `schema`, so a refusal is read before the schema is. Without that, the
  // `.map` below threw inside the handler and the table stayed "loading"
  // for good.
  const fetchColumns = React.useCallback(
    (id: string): Promise<void> => {
      rememberColumns(id, { status: "loading" })
      return assetService.getAsset(id).then(
        (detail): Promise<void> | void => {
          const refusal = catalogRefusal(detail)
          if (refusal) {
            rememberColumns(id, { status: "failed", reason: refusal.reason })
            return
          }
          // A slug id names no table, so the answer is asked for the SQL
          // form; an id that already reads as one keeps being the
          // reference and the answer is not read for names.
          let proving: Promise<void> | undefined
          if (!SQL_TABLE_NAME.test(id)) {
            const bronzeTable = isIcebergCandidate(detail) ? (detail.tableName ?? null) : null
            setTargets((previous) => ({
              ...previous,
              [id]: { target: assetQueryTarget(detail), bronzeTable },
            }))
            if (bronzeTable) proving = readProof(id, bronzeTable)
          }
          rememberColumns(id, {
            status: "ready",
            columns: detail.schema.map((column) => ({
              name: column.name,
              dataType: column.dataType,
            })),
          })
          return proving
        },
        // The request failed: the table is kept as a failure, and the
        // editor says so instead of asking again.
        () => rememberColumns(id, { status: "failed" })
      )
    },
    [rememberColumns, readProof]
  )

  React.useEffect(() => {
    if (assetsState.status !== "success") return
    for (const named of tablesInStatement(sql, tables)) {
      // The cache answers "has this table been asked for yet?": a table
      // named twice is fetched once, and a table that failed is not asked
      // for again on a keystroke.
      if (named.id in columnsRef.current) continue
      void fetchColumns(named.id)
    }
  }, [sql, tables, assetsState.status, fetchColumns])

  // Bronze rows are proven when the list loads (LH-QS-4-08), so a
  // Bronze row is insertable on a Trino tab without being expanded. It is
  // the read `fetchColumns` makes for a table a statement names, through the
  // same caches, so a table is read once however it was asked for. A
  // ClickHouse tab offers no Bronze name, so it reads nothing here, and a
  // keystroke reads nothing beyond the statement's own tables.
  //
  // At most `MAX_PROOF_READS` rows are read at a time, taken in list order
  // from a queue: a catalog with hundreds of Bronze tables would otherwise
  // open hundreds of requests at once from one page, and a row waiting its
  // turn is simply not insertable yet, which is what a row being proven
  // already is. The queue and the counter live in refs because they outlive
  // any one run of the effect; leaving the Trino tab stops the pump, and
  // coming back starts it again.
  const proofQueueRef = React.useRef<string[]>([])
  const proofReadsRef = React.useRef(0)
  const onTrinoTabRef = React.useRef(false)
  const pumpProofReads = React.useCallback(() => {
    while (
      onTrinoTabRef.current &&
      mountedRef.current &&
      proofReadsRef.current < MAX_PROOF_READS
    ) {
      const id = proofQueueRef.current.shift()
      if (id === undefined) return
      // A statement or an expand may have asked for it while it waited.
      if (id in columnsRef.current) continue
      proofReadsRef.current += 1
      void fetchColumns(id).finally(() => {
        proofReadsRef.current -= 1
        pumpProofReads()
      })
    }
  }, [fetchColumns])
  React.useEffect(() => {
    onTrinoTabRef.current = engine === "trino"
    if (engine !== "trino" || assetsState.status !== "success") return
    for (const asset of assetsState.data) {
      // A SQL-named id is its own reference, and only a Bronze layer has a
      // table to prove; the same test the rows use before their detail loads.
      if (SQL_TABLE_NAME.test(asset.id)) continue
      if (asset.layer !== "raw" && asset.layer !== "bronze") continue
      if (asset.id in columnsRef.current || proofQueueRef.current.includes(asset.id)) continue
      proofQueueRef.current.push(asset.id)
    }
    pumpProofReads()
  }, [engine, assetsState.status, assetsState.data, pumpProofReads])

  // The schema browser asks for a table's columns when a person expands it.
  // It shares the cache above, so a table the statement already named is not
  // fetched twice. Only a person pressing "Retry" (`retry`) may fetch a
  // failed table again: a keystroke never does. An entry whose proof read
  // failed is read again whenever it opens — the open is its retry — while
  // a keystroke still never is (#448 review SHOULD-FIX).
  const loadColumns = React.useCallback(
    (id: string, options: { retry?: boolean } = {}) => {
      const cached = columnsRef.current[id]
      if (
        cached &&
        !(options.retry && cached.status === "failed") &&
        proofs[id]?.status !== "failed"
      ) {
        return
      }
      void fetchColumns(id)
    },
    [fetchColumns, proofs]
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

  // The names `{{` completes: every definition the open tab holds, the ones
  // whose name is not in the SQL yet included. The source is built once, so
  // it reads the names through this ref.
  const parameterNamesRef = React.useRef<string[]>([])
  React.useEffect(() => {
    parameterNamesRef.current = parameterDefinitions.map((def) => def.name)
  }, [parameterDefinitions])

  // The suggestions the editor offers. The source is built once and reads
  // the cache when it is asked, so a keystroke never waits for the network.
  const completionSource = React.useMemo(
    () =>
      createSqlCompletionSource({
        tables,
        columns: () => columnsRef.current,
        parameters: () => parameterNamesRef.current,
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
  // Mirrors `hydratedRef` into state: the effects that must wait for the
  // owner's tabs to be in place (the `?saved=` link, below) read this one, so
  // they run in the same render the restored tabs land in.
  const [hydrated, setHydrated] = React.useState(false)
  // Whether the tab bar still holds the empty tab that was there before
  // anything was read. A link that arrives while it does takes its place
  // instead of leaving a blank tab next to the query it opened.
  const placeholderRef = React.useRef(true)
  React.useEffect(() => {
    if (hydratedRef.current) return
    // Nothing is read while the owner is not known (`QS-8`, `LH-QS-1-04` F1):
    // a key invented here would be somebody else's draft.
    if (ownKey === null) return
    hydratedRef.current = true
    // Same render as the restored tabs: the `?saved=` effect waits on this.
    setHydrated(true)
    // The entry every person shared before `QS-8` holds tabs whose owner
    // cannot be known: it is removed, not handed to this person.
    dropLegacyDraft()
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
    const stored = readTabs(ownKey)

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
          parameters: [],
          values: {},
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
        parameters: [],
        values: {},
      }
      placeholderRef.current = false
      setTabs([linked])
      setActiveTabId(linked.id)
      setTab("sql")
    }
  }, [ownKey])

  const persistDraft = useDebouncedCallback((key: string, next: QueryTabs) => {
    writeTabs(key, next)
  }, DRAFT_DEBOUNCE_MS)
  React.useEffect(() => {
    if (!hydratedRef.current || ownKey === null) return
    persistDraft(ownKey, { tabs, activeId: activeTabId, question, tab })
  }, [tabs, activeTabId, question, tab, ownKey, persistDraft])

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
  // What a save of that statement keeps: the definitions of the names it
  // uses. A save is checked against the SQL it holds, so a name only another
  // statement uses, or one that left the SQL, would be refused.
  const saveParameters = React.useMemo(
    () => activeParameters(targetStatement?.text ?? "", parameterDefinitions),
    [targetStatement, parameterDefinitions]
  )

  // ── Estimate ─────────────────────────────────────────────────────────
  const runEstimate = estimateAct.run
  const resetEstimate = estimateAct.reset
  // `null` inputs mean "skip": the statement has a name with nothing to run
  // with, so no estimate is asked for and the one on screen goes.
  const estimateDebounced = useDebouncedCallback(
    (next: string, inputs: QueryParamInputs | null, engine: QueryEngine) => {
      if (inputs === null) resetEstimate()
      else void runEstimate(next, inputs, engine)
    },
    ESTIMATE_DEBOUNCE_MS
  )
  // The estimate is asked for one statement at a time: the server plans a
  // statement, not a buffer with several of them.
  const targetText = targetStatement?.text ?? null
  // The names the statement itself uses (`QS-5`): a name that only another
  // statement of the tab uses is not this estimate's business.
  const estimateDefinitions = React.useMemo(
    () => activeParameters(targetText ?? "", parameterDefinitions),
    [targetText, parameterDefinitions]
  )
  const estimateInputs = React.useMemo(
    () =>
      estimateDefinitions.length === 0 ? NO_INPUTS : paramInputs(estimateDefinitions, parameterValues),
    [estimateDefinitions, parameterValues]
  )
  // A name with no value and no default would only be refused ("Fill in
  // ..."). The estimate waits for it instead, and the panel names it
  // (QS-5-AC14). SQL with no placeholder is never held back.
  const estimateWaitsFor = firstMissingValueName(estimateDefinitions, parameterValues)
  const estimateWaits = estimateWaitsFor !== null
  const estimatable = targetStatement !== null && !estimateWaits
  // Only ClickHouse has an estimate: the server runs `EXPLAIN ESTIMATE`,
  // which a Trino statement is not valid in, so asking for one on a Trino
  // tab got a refusal that read as the query's fault (`LH-QS-4-08`).
  // The merge with `main` (#482) kept this gate over this branch's engine
  // bookkeeping: a Trino tab asks nothing and drops what an earlier
  // ClickHouse statement left, so no answer for another engine shows.
  const estimateSupported = engine === "clickhouse"
  React.useEffect(() => {
    if (!estimateSupported) {
      // Drop what an earlier ClickHouse statement left: an ask in flight, a
      // pending debounced ask, and the estimate on screen. The skip input
      // replaces the pending debounced ask; the reset acts now, so no
      // answer for another engine shows after the switch.
      estimateDebounced("", null, engine)
      resetEstimate()
      return
    }
    // A buffer holding only the starter comment has nothing to estimate,
    // and asking anyway put an EXPLAIN ESTIMATE on ClickHouse for every
    // keystroke, including the ones that typed the comment.
    if (targetText !== null) estimateDebounced(targetText, estimateWaits ? null : estimateInputs, engine)
  }, [
    estimateSupported,
    engine,
    targetText,
    estimateWaits,
    estimateInputs,
    estimateDebounced,
    resetEstimate,
  ])

  // ── Actions ──────────────────────────────────────────────────────────
  /**
   * Puts `next` in the open tab. A saved query's `definitions` come with its
   * SQL (`QS-5`) and every field starts at its default; a statement from
   * History has none to bring, and the tab's own stay.
   */
  const loadSql = React.useCallback(
    (next: string, definitions?: QueryParamDef[]) => {
      setSql(next)
      if (definitions) {
        setTabs((previous) =>
          previous.map((open) =>
            open.id === activeTabId ? { ...open, parameters: definitions, values: {} } : open
          )
        )
        setRefused(null)
      }
      setTab("sql")
    },
    [setSql, activeTabId]
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
        parameters: [],
        values: {},
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
    async (
      tabId: string,
      list: SqlStatement[],
      runEngine: QueryEngine,
      inputs: QueryParamInputs
    ) => {
      // A run means the person has moved on from the proposal; its diff is stale.
      setFixResult(null)
      setFixStatement(null)
      setRefused(null)
      setRuns((previous) => ({
        ...previous,
        [tabId]: list.map(
          (statement) => ({ status: "pending", statement }) as StatementRun
        ),
      }))
      setRunIndex((previous) => ({ ...previous, [tabId]: 0 }))

      const controller = new AbortController()
      runControllerRef.current = controller
      const started = Date.now()
      setStartedAt(started)
      setNow(started)
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

      const markNotRun = (from: number, reason: "failure" | "stopped") => {
        setRuns((previous) => {
          const current = previous[tabId] ?? []
          return {
            ...previous,
            [tabId]: current.map((run, at) =>
              at >= from ? { status: "not-run", statement: run.statement, reason } : run
            ),
          }
        })
      }

      try {
        for (let index = 0; index < list.length; index += 1) {
          if (controller.signal.aborted) {
            markNotRun(index, "stopped")
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
              // Every statement gets the tab's values and definitions: the
              // server reads the names its own statement uses and ignores
              // the rest. Nothing is added when the tab has no placeholder.
              { engine: runEngine, runId, ...inputs },
              controller.signal
            )
            setRuns((previous) => ({
              ...previous,
              [tabId]: (previous[tabId] ?? []).map((run, at) =>
                at === index
                  ? { status: "done", statement: run.statement, result, runId }
                  : run
              ),
            }))
          } catch (error) {
            if (controller.signal.aborted) {
              // The person stopped the batch: the statement that was in
              // flight did not finish, and neither did the ones after it.
              markNotRun(index, "stopped")
              break
            }
            const serviceError = toServiceError(error)
            if (serviceError.code === "invalid_request") {
              const names = fieldsNamedBy(
                serviceError.message,
                (inputs.definitions ?? []).map((def) => def.name)
              )
              setRefused(names.size > 0 ? { tabId, names } : null)
            }
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
            if (!isPermissionRefusal) markNotRun(index + 1, "failure")
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
    await runBatch(activeTabId, statements, engine, runInputs)
  }, [statements, activeTabId, engine, runBatch, runInputs])

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
          engine,
          runInputs
        )
        return
      }
    }
    await runQuery()
  }, [readSelection, sql, activeTabId, engine, runBatch, runQuery, runInputs])

  /**
   * Stops the batch: the statement in flight and the ones after it did not
   * run, and the batch is over.
   */
  const stopRun = React.useCallback(() => {
    // A download is a query on the engine too, so it is stopped first: a
    // file that is being written must end even when no statement is in
    // flight, and the person pressed Stop for it.
    stopDownload()
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
          at >= batch.index ? { status: "not-run", statement: run.statement, reason: "stopped" } : run
        ),
      }
    })
    setRunning(false)
    if (!batch.reloaded) {
      batch.reloaded = true
      historyState.reload()
    }
  }, [historyState, stopDownload])

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
  //
  // The query is asked for by id, not looked for in the list the console
  // holds (`QS-3` C4): the list may not hold that query at all, and a query
  // the caller may not see is a 404 rather than a missing row.
  const [linkedSaved, setLinkedSaved] = React.useState<SavedQuery | null>(null)
  const [savedLinkError, setSavedLinkError] = React.useState<string | null>(null)
  const appliedSavedRef = React.useRef(false)
  React.useEffect(() => {
    if (appliedSavedRef.current) return
    const savedId = new URLSearchParams(window.location.search).get("saved")
    if (!savedId) return
    // The link is read once: StrictMode runs this effect a second time
    // right after the first one, and the second run must not ask for the
    // query again.
    appliedSavedRef.current = true
    void queryService.getSaved(savedId).then(
      (saved) => {
        // The hook's own "mounted" flag and not one of this effect's own:
        // StrictMode's second run would take a flag of this effect's own
        // down with it, and the answer of the only request that was made
        // would be dropped with it.
        if (mountedRef.current) setLinkedSaved(saved)
      },
      (error: unknown) => {
        if (!mountedRef.current) return
        const failure = toServiceError(error)
        // A query the caller may not see and one in the trash are the same
        // answer for them: the studio says so and opens no tab (C5).
        setSavedLinkError(
          failure.code === "not_found"
            ? SAVED_QUERY_UNAVAILABLE
            : readableErrorMessage(failure.message)
        )
      }
    )
  }, [])

  React.useEffect(() => {
    // `QS-8` (`LH-QS-1-04` F1): a restored draft must not replace a tab the
    // link opened, so this waits until the owner's tabs are in place.
    if (!hydrated || !linkedSaved) return
    const openedId = newTabId()
    // The empty tab that was there before anything was read takes the saved
    // query's place instead of sitting next to it.
    const takesPlaceholder = placeholderRef.current
    placeholderRef.current = false
    setTabs((previous) => {
      const name = linkedSaved.title.trim() || nextTabName(previous)
      const opened: QueryTab = {
        id: openedId,
        name,
        sql: linkedSaved.sql,
        engine: "clickhouse",
        // Its definitions, and no values: each field starts at its default.
        parameters: linkedSaved.parameters,
        values: {},
        savedId: linkedSaved.id,
        level: linkedSaved.level,
      }
      if (takesPlaceholder && previous.length === 1 && previous[0].id === FIRST_TAB_ID) {
        return [opened]
      }
      return [...previous, opened]
    })
    setActiveTabId(openedId)
    setTab("sql")
  }, [hydrated, linkedSaved])

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
    /** The parameters whose `{{name}}` the open tab's SQL uses, in order, with their definitions. */
    parameters,
    /** The open tab's values by name; a parameter without one is not given. */
    parameterValues,
    setParameterValue,
    setParameterDefinition,
    /** The parameters the last refusal of a run or an estimate named, by name. */
    invalidParameters,
    /**
     * The definitions a save of the statement in `targetStatement` carries:
     * the ones its SQL uses, defaults included, and never a current value.
     */
    saveParameters,
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
    /**
     * How long the run has been going, counted in the browser from the
     * moment Run was pressed; null when no run has started.
     */
    elapsedMs: startedAt === null ? null : Math.max(0, now - startedAt),
    download: {
      busy: download.busy,
      done: download.done,
      error: download.error,
      start: download.start,
      stop: download.stop,
    },
    runnable,
    targetStatement,
    assetsState,
    tables,
    columns,
    /**
     * Whether a table's Iceberg table is there, by catalog id. A table
     * with no entry has no proof yet, and is offered nothing (QS-4 D3).
     * `QS-7`: the entry's own read or the `bronze` namespace's table list,
     * whichever answered — the newer one wins.
     */
    proofs: tableProofs,
    loadColumns,
    setEditorView,
    insertAtCursor,
    completionSource,
    editorExtensions,
    estimatable,
    estimateWaitsFor,
    estimateSupported,
    generateAct,
    estimateAct,
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
      if (estimateSupported && targetText !== null && !estimateWaits) {
        void runEstimate(targetText, estimateInputs, engine)
      }
    },
    /**
     * What the Save button on the open tab does: "Save" changes the saved
     * query the tab came from, "Save as new" makes a copy of it. The write
     * itself is the Save query page's (`save-query-page.tsx`, through
     * `savedQueryRequest`); this hook reads only what the button offers
     * (`WEB-7` D5 — its own unused `save` left in the S2 review, S3).
     */
    saveTarget: {
      mode: savesSavedQuery ? ("update" as const) : ("create" as const),
      label: savesSavedQuery ? ("Save" as const) : ("Save as new" as const),
      savedId: activeSaved?.id ?? null,
      level: activeSaved?.level,
    },
    /**
     * What a `?saved=` link answered: the sentence the studio shows when
     * the query is not there for the caller, or `null` when it opened.
     */
    savedLinkError,
  }
}
