import type { EngineCategory, EntityStatus, WorkloadClass } from "@/lib/status"
import type { Asset } from "./assets"

/**
 * The query-execution engine, chosen by the user in Query Studio (WS2 §4).
 * Not to be confused with `EngineCategory`, which describes a ClickHouse
 * storage tier (hot/warm/cold) rather than which execution engine ran the
 * query.
 */
export type QueryEngine = "clickhouse" | "trino"

/**
 * The level the caller holds on a saved query or on a folder (`QS-3`).
 *
 * The server writes the level as a word — "view", "edit" or "manage" —
 * and reads nothing else as a level. The console widens it so that a
 * level it does not know is shown rather than dropped.
 */
export type SavedQueryLevel = "view" | "edit" | "manage"

/**
 * The type of a query parameter (`QS-5`, `{{name}}` in SQL). The server
 * reads exactly these six words and refuses any other, so the type is not
 * widened with `| string`: the console cannot show a control for a type it
 * does not know.
 */
export type QueryParamType =
  | "text"
  | "number"
  | "date"
  | "dateRange"
  | "dropdown"
  | "multiSelect"

/**
 * One choice of a dropdown or multi-select. The server accepts a text or a
 * number as the value (all options of one list are one or the other) and
 * keeps the label the control shows.
 */
export type QueryParamOption = {
  value: string | number
  label: string
}

/** A date range as the server reads it: exactly these two keys, each `YYYY-MM-DD`. */
export type QueryDateRange = { start: string; end: string }

/**
 * A parameter's value, as a run or a default carries it. Text is a string,
 * a number is a finite JSON number, a date is `YYYY-MM-DD`, a date range is
 * a {@link QueryDateRange}, a dropdown is one option's value and a
 * multi-select is a non-empty list of option values. Absent or `null` means
 * "not given", and the default applies.
 */
export type QueryParamValue =
  | string
  | number
  | QueryDateRange
  | (string | number)[]

/**
 * One parameter's definition, as a saved query keeps it and as a run may
 * carry it (`QS-5`). The server omits `default` and `options` when there
 * are none, so both are optional here; a multi-select default of `[]` is
 * refused, so leave the default out instead.
 */
export type QueryParamDef = {
  name: string
  type: QueryParamType
  default?: QueryParamValue
  options?: QueryParamOption[]
}

/**
 * What a run or an estimate may carry for the `{{name}}` placeholders in its
 * SQL. Both are optional: a request that carries neither is the request
 * sent before parameters existed.
 */
export type QueryParamInputs = {
  /** The value of each parameter, by name. */
  parameters?: Record<string, QueryParamValue>
  /** The definitions those values are checked against. */
  definitions?: QueryParamDef[]
}

/**
 * One saved query, as the server sends it (`QS-3`).
 *
 * The nine fields are the server's own: `folderId` says where the query
 * sits (`null` is the shared space), `ownerId` names the author and
 * `level` says what the caller may do with it. The console needs the
 * level to offer the right row actions and the folder to draw the tree.
 */
export type SavedQuery = {
  id: string
  title: string
  sql: string
  owner: string
  updatedAt: string
  tags: string[]
  /** The folder the query sits in; `null` is the shared space. */
  folderId: string | null
  /** The author's user id, when the query has one. */
  ownerId: string | null
  /** The level the caller holds on it. */
  level: SavedQueryLevel | string
  /** The query's parameter definitions; the server always sends the list, empty when there are none. */
  parameters: QueryParamDef[]
}

/**
 * The filter the saved-query list takes (`GET /api/query/saved`): the
 * text typed in the search box and the folder asked for. Both are the
 * server's to apply — the console never filters the list in the browser,
 * because it may not see every row the server would send.
 */
export type SavedQueryFilter = {
  /** The text typed in the search box: the server matches it against the title and the SQL. */
  q?: string
  /** The folder asked for: an id, or "root" for the shared space. */
  folder?: string
}

/**
 * One entry in the trash (`GET /api/query/saved-trash`): a query that
 * waits to be restored or to be swept away, with the days it has left.
 */
export type SavedQueryTrashEntry = {
  id: string
  title: string
  /** The folder it waits in; `null` is the shared space. */
  folderId: string | null
  /** When it went to the trash, ISO 8601. */
  deletedAt: string
  /** How many days it has left before the sweep takes it away. */
  daysLeft: number
}

/**
 * One version of a saved query (`GET /api/query/saved/{id}/versions`).
 *
 * A change writes a version, so going back to an earlier one is itself
 * undoable: the version says what the query looked like then, and
 * `label` says why it was written ("Restored version 3").
 */
export type SavedQueryVersion = {
  /** The version's number: 1 for the first. */
  version: number
  title: string
  sql: string
  /** The author's user id, when the version has one. */
  authorId: string | null
  /** The author's name, as the history shows it. */
  authorName: string
  /** What the history says about this version; empty for a plain change. */
  label: string
  /** When it was written, ISO 8601. */
  createdAt: string
  /** The parameter definitions this version held; the server always sends the list. */
  parameters: QueryParamDef[]
}

/**
 * One folder of the saved-query tree (`GET /api/query/folders`).
 *
 * The tree holds the shared space as its root and one personal space
 * per person; the shared space has no node of its own, so a folder with
 * no parent sits in the shared space.
 */
export type QueryFolder = {
  id: string
  /** The folder this one sits in; `null` at the root. */
  parentId: string | null
  name: string
  /** Whether this is a personal space: one person's own space. */
  isPersonal: boolean
  /** The owner of a personal space, the creator of an ordinary folder. */
  ownerId: string | null
  /** The level the caller holds on it. */
  level: SavedQueryLevel | string
}

/** The kind of item an access list belongs to. */
export type AccessItemKind = "query" | "folder"

/** One rule of an item's own access list (`GET|PUT …/access`). */
export type SavedQueryAccessRule = {
  id: string
  /** The person the rule names; `null` when the rule names a role. */
  userId: string | null
  /** The role the rule names; `null` when the rule names a person. */
  roleId: string | null
  /** The grantee's name; `null` when the server cannot find it in the organisation. */
  granteeName: string | null
  /** The person's email; `null` for a role and for a person the server cannot find. */
  granteeEmail: string | null
  /** What the rule gives. */
  level: SavedQueryLevel | string
  /** Who wrote the rule; `null` when nobody is recorded. */
  grantedBy: string | null
  /** When the rule was written, ISO 8601. */
  grantedAt: string
}

/** One rule an item inherits from a folder above it. */
export type InheritedAccessRule = {
  /** The folder the rule sits on. */
  folderId: string
  /** The folder's name, as the tree shows it. */
  folderName: string
  /** The person the rule names; `null` when the rule names a role. */
  userId: string | null
  /** The role the rule names; `null` when the rule names a person. */
  roleId: string | null
  /** The grantee's name; `null` when the server cannot find it in the organisation. */
  granteeName: string | null
  /** The person's email; `null` for a role and for a person the server cannot find. */
  granteeEmail: string | null
  /** What the rule gives. */
  level: SavedQueryLevel | string
}

/**
 * The access list of one saved query or one folder
 * (`GET|PUT /api/query/saved/{id}/access`, `GET|PUT
 * /api/query/folders/{id}/access`).
 *
 * `yourLevel` is the level the caller held before the write: a rule the
 * caller writes for somebody else cannot change the caller's own level.
 */
export type SavedQueryAccessList = {
  itemType: AccessItemKind
  itemId: string
  /** The level the caller holds on the item. */
  yourLevel: SavedQueryLevel | string
  /** The item's own rules. */
  rules: SavedQueryAccessRule[]
  /** The rules of the folders above the item, the nearest folder first. */
  inherited: InheritedAccessRule[]
}

/** One rule of a `PUT …/access` body: exactly one grantee and one level. */
export type AccessRuleInput = {
  /** The person the rule names. */
  userId?: string
  /** The role the rule names. */
  roleId?: string
  /** What the rule gives. */
  level: SavedQueryLevel
}

/**
 * The body a `PUT …/access` sends: the rules the item shall carry, and
 * nothing else — the server refuses a body of another shape.
 */
export type SavedQueryAccessInput = { rules: AccessRuleInput[] }

/** One person the caller may share a saved query with (`GET /api/query/grantees`). */
export type GranteePerson = { id: string; name: string; email: string | null }

/** One role the caller may share a saved query with. */
export type GranteeRole = { id: string; name: string }

/** The people and the roles the caller may share with. */
export type GranteeList = { people: GranteePerson[]; roles: GranteeRole[] }

/** One result cell, in whatever type ClickHouse reported it as. */
export type QueryCell = string | number | boolean | null

export type QueryHistoryItem = {
  id: string
  sql: string
  user: string
  at: string
  /**
   * `stopped` is a run ended with Stop; `cancelled` stays for rows
   * written before that status existed, and nothing writes it now.
   */
  status: Extract<EntityStatus, "completed" | "failed" | "stopped" | "cancelled" | "blocked">
  durationMs: number
  scannedBytes: number
  costUnits: number
  workloadClass: WorkloadClass
  /**
   * The API now records the real execution engine (`"clickhouse"` /
   * `"trino"`) here; old rows written before that change still carry
   * `"hot-store"`, an `EngineCategory` value, hence the union — and
   * `| string` since either side is free to widen without breaking this
   * contract.
   */
  engine: EngineCategory | QueryEngine | string
  cacheAssisted: boolean
  /**
   * The tables the statement named as written (`QS-4`): empty for a
   * statement that did not parse, and for every row recorded before the
   * column existed — the server backfills nothing.
   */
  tablesRead: string[]
  /** The exact text the response gave the caller for a failed or refused run. */
  message?: string
  auditEventId?: string
}

/**
 * The history page's filters (`QS-4`, decision 5): every field is
 * optional, and an absent or empty field filters nothing. `from` and `to`
 * are RFC 3339 strings, the shape a row's `at` carries. `page` is
 * 1-based and defaults to the first page. The server clamps `from` to the
 * last 14 days whatever the caller sends (decision 2).
 */
export type QueryHistoryFilter = {
  status?: QueryHistoryItem["status"]
  engine?: string
  minDurationMs?: number
  table?: string
  from?: string
  to?: string
  page?: number
}

/**
 * One numbered page of the history (`GET /api/query/history`): the runs
 * of that page, newest first, the count of every run the filter matches,
 * the page the server answered and its size. A page past the end has no
 * runs and the true `total`.
 */
export type QueryHistoryPage = {
  runs: QueryHistoryItem[]
  total: number
  page: number
  pageSize: number
}

/** Simple federated / multi-source execution plan stage for UI. */
export type QueryPlanStage = {
  id: string
  label: string
  location: string
  operation: string
  estimatedBytes?: number
  status?: Extract<EntityStatus, "completed" | "running" | "failed" | "blocked">
}

export type QueryEstimate = {
  /**
   * `QS-6` (decision 7): whether the server could estimate this statement at
   * all. Absent from an older server, which only ever answered a ClickHouse
   * estimate — so absent reads as supported.
   */
  supported?: boolean
  /** Why there is no estimate, as the server's fixed sentence (`QS-6`). */
  reason?: string | null
  /**
   * `QS-6`: which engine answered. `null`/absent from an older server, whose
   * estimate always described ClickHouse.
   */
  queryEngine?: QueryEngine | string
  /** Null when the query could not be planned — see `error`. */
  estimatedBytes: number | null
  estimatedCostMin: number | null
  estimatedCostMax: number | null
  /**
   * Why there is no estimate, as the server sends it: today the engine's
   * trimmed diagnosis (`SEC-11`, decision 4).
   */
  error?: string | null
  /** Null for a Trino statement, which has no ClickHouse workload class. */
  workloadClass: WorkloadClass | null
  /** Null for a Trino statement, which has no ClickHouse storage tier. */
  engine: EngineCategory | null
  /**
   * `EXPLAIN ESTIMATE` doesn't report cache eligibility — null until a
   * real cache-eligibility check exists.
   */
  cacheEligible: boolean | null
  /** Lag of the stalest source table (`freshness_lag`, ClickHouse `system.parts`); null when no source is known. */
  freshnessLagSeconds: number | null
  /** No policy engine exists yet — WS7 builds one. */
  policyObligations: string[] | null
  sources: string[]
  /**
   * `DATA-12` part 3: the certified or deprecated mark of each table a
   * ClickHouse query reads, by the name in `sources` (`silver.orders`). Only a
   * marked source has an entry; absent when none is. Same field names as an
   * asset's, so the same notice renders them.
   */
  sourceMarks?: Record<
    string,
    Pick<
      Asset,
      | "certification"
      | "certificationNote"
      | "replacementAssetId"
      | "replacementName"
      | "tags"
      | "governedTags"
    >
  >
  /**
   * WS1 task 1.6: the old plan marked stages that had not run as
   * `status: "completed"`. Null until a real plan exists.
   */
  plan: QueryPlanStage[] | null
}

export type QueryResult = {
  id: string
  columns: string[]
  rows: Record<string, QueryCell>[]
  /**
   * The engine this specific run actually executed against — distinct from
   * `metrics.engine` below, which is an `EngineCategory` describing the
   * ClickHouse storage tier the query hit, not which engine ran it.
   */
  engine: QueryEngine | string
  /** How many rows the query produced, which `rows` may only be part of (`total_rows`). */
  rowCount: number
  /**
   * Which limit cut the result: `"rows"` for the Query Studio row limit
   * (64,000 by default), `"bytes"` for the server-side 10 MB cap. The key is
   * absent when nothing was cut — the server sends no `cutBy` at all then,
   * so an absent field and a `null` never have to be told apart.
   */
  cutBy?: "rows" | "bytes"
  /** True when the result was cut, by either limit. */
  truncated: boolean
  rowLimit: number
  metrics: {
    durationMs: number
    scannedBytes: number
    costUnits: number
    /** Null for a Trino run, which has no ClickHouse storage tier. */
    engine: EngineCategory | null
    /** Null for a Trino run, which has no ClickHouse workload class. */
    workloadClass: WorkloadClass | null
    /** ClickHouse's response carries no cache-hit flag — not measured. */
    cacheHit: boolean | null
    /** No pushdown computation exists — not measured. */
    pushdowns: string[] | null
    /** No policy engine exists yet — WS7 builds one. */
    policyObligations: string[] | null
  }
  /**
   * QA `LH-QS-2-06` (`QS-9`): whether this person may download a result at
   * all. `false` when every role they hold is switched off from downloading
   * (the same switch the dashboard reads); the console turns Download and
   * "Copy as TSV" off with it. Absent from an older server, which counts as
   * allowed — and the download route refuses on its own either way.
   */
  canDownload?: boolean
  /**
   * WS1 task 1.6: the old plan was a single hardcoded stage claiming
   * "scan + aggregate" for every query. Null until a real plan exists.
   */
  plan: QueryPlanStage[] | null
  auditEventId?: string
}

/**
 * `GET /api/query/scheduling` — a static capability probe. Scheduled
 * execution of a saved query needs a safe per-principal authority model
 * (a scheduled job has no live session to run "as," and a shared service
 * identity would bypass the author's own permissions) that does not exist
 * before WS7's policy-obligations engine, so this is always `supported:
 * false` today — never a real schedule, never a stored `schedule_cron`
 * (WS2 §13, WS2 plan review W10, round 2, item 1).
 */
export type QuerySchedulingCapability = { supported: false; reason: string }

/**
 * What "Save query" in Query Studio sends.
 *
 * The folder is three answers, not two: an id puts the query in that
 * folder, `null` puts it in the shared space, and an absent field puts
 * it in the caller's own personal space. The server reads the three
 * apart, so the console may not fold them into one.
 */
export type SaveQueryInput = {
  title: string
  sql: string
  tags: string[]
  /**
   * The folder the query lands in: an id, `null` for the shared space,
   * absent for the caller's own personal space.
   */
  folderId?: string | null
  /** The definitions of the `{{name}}` parameters in `sql`; absent when it has none. */
  parameters?: QueryParamDef[]
}

/**
 * What a change of a saved query sends (`PATCH /api/query/saved/{id}`).
 *
 * Every field is optional and every absent field means "leave it as it
 * is" — the one exception is `folderId`, where an absent field and a
 * `null` one are two different answers: absent leaves the query where
 * it is, `null` moves it to the shared space.
 */
export type UpdateSavedQueryInput = {
  /** A new title. */
  title?: string
  /** New SQL. */
  sql?: string
  /** New tags; an empty list clears them. */
  tags?: string[]
  /** The folder the query moves to: an id, `null` for the shared space, absent to leave it where it is. */
  folderId?: string | null
  /** New definitions; absent leaves them as they are and an empty list clears them. */
  parameters?: QueryParamDef[]
}

/**
 * What "New folder" sends (`POST /api/query/folders`): a name and the
 * folder it sits in. The root is named by an absent field or by `null`.
 */
export type CreateFolderInput = {
  name: string
  /** The folder it sits in: an id, `null` or absent for the root. */
  parentId?: string | null
}

/**
 * What a change of a folder sends (`PATCH /api/query/folders/{id}`): a
 * new name, a new place, or both. An absent `parentId` leaves the
 * folder where it is; `null` moves it to the root.
 */
export type UpdateFolderInput = {
  /** A new name. */
  name?: string
  /** The folder it moves to: an id, `null` for the root, absent to leave it where it is. */
  parentId?: string | null
}

/** One tool call the chat made while answering a question typed in words. */
export type NlAnswerStep = {
  /** The tool's name, e.g. `run_sql`. */
  step: string
  /** For `run_sql` its SQL, or its error text when it failed; empty for the other tools. */
  detail: string
}

/**
 * What Query Studio's "Natural language" box shows: the chat's answer and
 * the last query it ran successfully. `sql` is absent when the model ran
 * none, so the editor keeps what it had.
 */
export type NlAnswer = {
  answer: string
  sql?: string
  columns: string[]
  rows: Record<string, QueryCell>[]
  rowCount: number
  steps: NlAnswerStep[]
}

/**
 * What the assistant proposed for a failed statement: the whole corrected
 * statement and one sentence on what was wrong. The person accepts or
 * rejects it; nothing runs until they run it.
 */
export type SqlProposal = { sql: string; reason: string }

/**
 * The fix request's reply: the proposal when the assistant made one, or its
 * text alone when it proposed nothing.
 */
export type SqlFixOutcome = SqlProposal | { answer: string }

/**
 * The file kinds a Query Studio download can be.
 */
export type QueryDownloadFormat = "csv" | "tsv" | "xlsx" | "parquet"

/**
 * What a finished download holds.
 */
export type QueryDownload = {
  /** The file the server sent. */
  blob: Blob
  /** The name the server gave it (from `Content-Disposition`). */
  fileName: string
  /** The rows the file holds; null when the server does not say (the engine-written file). */
  rows: number | null
  /** Whether a limit cut the file short. */
  cut: boolean
  /** Which limit cut it; null when the file was not cut. */
  cutBy: "rows" | "bytes" | null
}

export interface QueryService {
  listSaved(signal?: AbortSignal): Promise<SavedQuery[]>
  /**
   * The saved queries the caller may see, filtered by the server
   * (`GET /api/query/saved?q=&folder=`). The console asks the server
   * for the folder it shows and for the text typed in the search box,
   * because it may not see every row the server would send.
   */
  listSavedFiltered(filter: SavedQueryFilter, signal?: AbortSignal): Promise<SavedQuery[]>
  /** One saved query the caller may see. */
  getSaved(id: string, signal?: AbortSignal): Promise<SavedQuery>
  saveQuery(input: SaveQueryInput, signal?: AbortSignal): Promise<SavedQuery>
  /**
   * Change a saved query. A refusal throws with the server's own
   * sentence: a move (`folderId`) needs manage on the query and is
   * refused with the sentence that says so.
   */
  updateSaved(id: string, input: UpdateSavedQueryInput, signal?: AbortSignal): Promise<SavedQuery>
  /** Take a saved query to the trash. The server answers with nothing. */
  deleteSaved(id: string, signal?: AbortSignal): Promise<void>
  /** Copy a saved query into the caller's own personal space. */
  duplicateSaved(id: string, signal?: AbortSignal): Promise<SavedQuery>
  /** Bring a trashed saved query back. */
  restoreSaved(id: string, signal?: AbortSignal): Promise<SavedQuery>
  /** The queries in the trash the caller may edit, with the days they have left. */
  listSavedTrash(signal?: AbortSignal): Promise<SavedQueryTrashEntry[]>
  /** The versions of a saved query, newest first. */
  listSavedVersions(id: string, signal?: AbortSignal): Promise<SavedQueryVersion[]>
  /** Go back to an earlier version of a saved query. */
  revertSavedVersion(id: string, version: number, signal?: AbortSignal): Promise<SavedQuery>
  /** The folders the caller may see, each with the level the caller holds on it. */
  listFolders(signal?: AbortSignal): Promise<QueryFolder[]>
  /** Make a folder. */
  createFolder(input: CreateFolderInput, signal?: AbortSignal): Promise<QueryFolder>
  /** Rename a folder or move it. */
  updateFolder(id: string, input: UpdateFolderInput, signal?: AbortSignal): Promise<QueryFolder>
  /** Delete a folder. The server answers with nothing. */
  deleteFolder(id: string, signal?: AbortSignal): Promise<void>
  /** Who has what on one saved query. */
  getSavedAccess(id: string, signal?: AbortSignal): Promise<SavedQueryAccessList>
  /** Replace the rules on one saved query. */
  putSavedAccess(
    id: string,
    input: SavedQueryAccessInput,
    signal?: AbortSignal
  ): Promise<SavedQueryAccessList>
  /** Who has what on one folder. */
  getFolderAccess(id: string, signal?: AbortSignal): Promise<SavedQueryAccessList>
  /** Replace the rules on one folder. */
  putFolderAccess(
    id: string,
    input: SavedQueryAccessInput,
    signal?: AbortSignal
  ): Promise<SavedQueryAccessList>
  /** The people and the roles the caller may share with, at most 20 of each. */
  findGrantees(query: string, signal?: AbortSignal): Promise<GranteeList>
  /**
   * The runs of the page the filter names (`filter.page`, 1-based), newest
   * first, 50 at most. Without `page` it is the first page. The studio and
   * the side panel show this page only. `listHistoryPage` returns the same
   * runs with the total count. The filter is optional: without it the
   * endpoint answers the newest page of the last 14 days (`QS-4`, decision 5).
   */
  listHistory(filter?: QueryHistoryFilter, signal?: AbortSignal): Promise<QueryHistoryItem[]>
  /**
   * One numbered page of the history with the total count, for the
   * History page's page numbers. `filter.page` is 1-based.
   */
  listHistoryPage(filter?: QueryHistoryFilter, signal?: AbortSignal): Promise<QueryHistoryPage>
  /**
   * `options` carries the `{{name}}` values and definitions (`QS-5`) and the
   * engine the tab runs on (`QS-6`): with none, the request is the one sent
   * before parameters existed, and it is a ClickHouse estimate. A `Trino`
   * statement has no pre-run estimate; the server answers `supported: false`
   * with the reason and asks no engine.
   */
  estimate(
    sql: string,
    options?: QueryParamInputs & { engine?: QueryEngine },
    signal?: AbortSignal
  ): Promise<QueryEstimate>
  /**
   * `runId` (`BI-5` plan D5) goes out as `X-Run-Id`, so a Stop can name the
   * run; absent, the server makes one and the run cannot be stopped by id.
   */
  run(
    sql: string,
    options: { engine: QueryEngine; runId?: string } & QueryParamInputs,
    signal?: AbortSignal
  ): Promise<QueryResult>
  /**
   * `POST /api/query/cancel { runId }` (`BI-5` plan D5): ends the caller's own
   * queries of that run on the engine and answers how many it found running.
   * **The route does not exist on the server yet** (T4 of the BI-5 plan), so
   * the call answers 404 until then; Stop aborts the request either way.
   */
  cancel(runId: string, signal?: AbortSignal): Promise<{ stopped: number }>
  /**
   * Downloads the result of a run as a file (`GET /api/query/run/{id}/download`).
   * The server re-runs the stored statement with the caller's current masks
   * and row filters; `runId` names the run, so Stop reaches the download's
   * query on the engine.
   */
  download(
    id: string,
    format: QueryDownloadFormat,
    runId: string,
    signal?: AbortSignal
  ): Promise<QueryDownload>
  /** Asks the chat's engine; masking, row filters and `query:read` apply to the SQL it runs. */
  askQuestion(question: string, signal?: AbortSignal): Promise<NlAnswer>
  generateSql(question: string, signal?: AbortSignal): Promise<{ sql: string; explanation: string }>
  /** Plain-words explanation of a statement; the assistant calls no tool and runs nothing. */
  explainSql(sql: string, engine: QueryEngine, signal?: AbortSignal): Promise<string>
  /**
   * Asks the assistant to correct a failed statement. The result is checked
   * by the server but never run here.
   * @throws ServiceError `invalid_request` when the statement is over 20,000 characters.
   */
  proposeFix(
    sql: string,
    engine: QueryEngine,
    error: string,
    signal?: AbortSignal
  ): Promise<SqlFixOutcome>
  getSchedulingCapability(signal?: AbortSignal): Promise<QuerySchedulingCapability>
}
