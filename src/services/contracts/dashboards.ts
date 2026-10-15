import type { Board } from "../clients/bi-store"

/**
 * Kontrak untuk halaman DAFTAR dashboard (`/dashboards/browse`) dan untuk
 * SQL sources / folders.
 *
 * Sengaja sempit. Kanvas dashboard (`/dashboards/[id]`) masih memanggil
 * `/api/dashboard/*` lewat `apiFetch` langsung untuk chart/layout/filter,
 * dan memindahkannya ke sini akan menyentuh berkas yang belum perlu
 * disentuh. Yang dibutuhkan lewat service ini: baca semua board, lalu
 * buat / rename / ubah deskripsi / duplikat / hapus satu board; SQL
 * sources (mirroring `lakehouse_bi::sources::{SqlSource, SourceColumn}`,
 * `rust/crates/lakehouse-bi/src/sources.rs`, dan route
 * `rust/crates/lakehouse-api/src/routes/dashboard_sources.rs`); dan
 * folders untuk mengelompokkan board & source.
 *
 * A SQL source is a saved read-only SELECT (usually a join across several
 * `serving` marts) a chart can read instead of one mart. Authoring needs the
 * `dashboard:sql` permission; listing needs `dashboard:read`.
 */
export type DashboardService = {
  /** Semua board, board bawaan (`default`) di urutan pertama. */
  listBoards(signal?: AbortSignal): Promise<Board[]>
  createBoard(input: CreateBoardInput, signal?: AbortSignal): Promise<Board>
  /** Salin sebuah board beserta seluruh chart-nya. */
  duplicateBoard(id: string, signal?: AbortSignal): Promise<Board>
  updateBoard(input: UpdateBoardInput, signal?: AbortSignal): Promise<void>
  deleteBoard(id: string, signal?: AbortSignal): Promise<void>

  listSqlSources(signal?: AbortSignal): Promise<SqlSource[]>
  createSqlSource(input: SaveSqlSourceInput, signal?: AbortSignal): Promise<SqlSource>
  updateSqlSource(
    input: SaveSqlSourceInput & { id: string },
    signal?: AbortSignal
  ): Promise<SqlSource>
  deleteSqlSource(id: string, signal?: AbortSignal): Promise<void>
  /**
   * Run `sql` without saving it. With `chart` (the chart input the builder
   * sends to `/api/dashboard/specs/preview`, minus any mart or source id) the
   * answer also carries that chart drawn over this SQL.
   */
  previewSqlSource(
    sql: string,
    chart?: Record<string, unknown>,
    signal?: AbortSignal
  ): Promise<SqlSourcePreview>
  /**
   * `GET /api/dashboard/join-suggestions` (`BI-7`): candidate column pairs for
   * joining the mart `right` to the mart `left`, best first, and whether the
   * suggested pairs cover the right table's sorting key. Column metadata only.
   */
  joinSuggestions(left: string, right: string, signal?: AbortSignal): Promise<JoinSuggestions>
  listFolders(signal?: AbortSignal): Promise<DashboardFolder[]>
  createFolder(input: { name: string; parentId?: string }, signal?: AbortSignal): Promise<DashboardFolder>
  updateFolder(
    input: { id: string; name?: string; parentId?: string },
    signal?: AbortSignal
  ): Promise<DashboardFolder>
  /** 409 (as `invalid_request`) while the folder still holds anything. */
  deleteFolder(id: string, signal?: AbortSignal): Promise<void>
  /** `folderId` "" = root. */
  moveBoard(boardId: string, folderId: string, signal?: AbortSignal): Promise<void>

  /**
   * SEC-12. The board's signed-embedding state: whether it can sign at all,
   * a sample token, what has been withdrawn and which sites may frame it.
   * Needs `dashboard:read`.
   */
  getEmbedInfo(board: string, signal?: AbortSignal): Promise<EmbedInfo>
  /**
   * Replace the sites allowed to frame the board's embed pages. The server
   * validates and normalises them; a refusal arrives as `invalid_request`
   * carrying its message. Resolves to the list that was stored. Needs
   * `dashboard:write`.
   */
  setEmbedOrigins(board: string, origins: string[], signal?: AbortSignal): Promise<string[]>
  /**
   * Withdraw every signed embed token of the board issued so far. Resolves to
   * the instant (Unix seconds) stored. Needs `dashboard:write`.
   */
  revokeAllEmbedTokens(board: string, signal?: AbortSignal): Promise<number>
  /**
   * Withdraw one signed token, presented whole. Refused with the server's
   * message for a token that does not verify, is for another dashboard or
   * carries no `jti`. Resolves to the withdrawn `jti`. Needs `dashboard:write`.
   */
  revokeEmbedToken(board: string, token: string, signal?: AbortSignal): Promise<string>

  /**
   * BI-18 part C, part 2. The last versions of a dashboard, newest first
   * (`GET /api/dashboard/versions`). Needs `dashboard:read`. The built-in
   * dashboard keeps none.
   */
  listVersions(board: string, signal?: AbortSignal): Promise<VersionHistory>
  /**
   * Write a stored version back (`POST /api/dashboard/versions/revert`). Needs
   * `dashboard:write`. A version that no longer holds against today's rules is
   * refused with a sentence naming why; nothing is changed then.
   */
  revertToVersion(board: string, version: number, signal?: AbortSignal): Promise<RevertResult>
  /** Deleted dashboards and charts the caller may see (`GET /api/dashboard/trash`). */
  listTrash(signal?: AbortSignal): Promise<TrashList>
  /** Bring one back (`POST /api/dashboard/trash/restore`). Needs `dashboard:write`. */
  restoreFromTrash(kind: ItemKind, id: string, signal?: AbortSignal): Promise<RestoredItem>
  /** The caller's personal space, created on first use. Needs `dashboard:write`. */
  ensurePersonalSpace(signal?: AbortSignal): Promise<DashboardFolder>
  /** The caller's favourites and recent dashboards and charts. */
  shortcuts(signal?: AbortSignal): Promise<Shortcuts>
  setFavourite(kind: ItemKind, id: string, on: boolean, signal?: AbortSignal): Promise<void>
  /** Note that the caller opened an item (best effort). */
  touchRecent(kind: ItemKind, id: string, signal?: AbortSignal): Promise<void>
  /** Dashboards and charts whose name contains `text`, across what the caller may read. */
  searchItems(text: string, signal?: AbortSignal): Promise<ShortcutItem[]>
}

/** What a favourite, a recent item, a search hit or a trash entry points at. */
export type ItemKind = "board" | "chart"

/** One version of a dashboard (`lakehouse_bi::versions::VersionEntry`). */
export type VersionEntry = {
  version: number
  /** Display name of whoever made the change. */
  author: string
  /** When, as the store formats it. */
  at: string
  /** What changed against the version before, in a few words. */
  summary: string
  charts: number
}

export type VersionHistory = { versions: VersionEntry[]; keep: number }

export type RevertResult = {
  restoredFrom: number
  /** The new version the revert wrote; `null` when the content was already that. */
  version: number | null
  /** Charts added since that moved to the trash. */
  chartsRemoved: number
  /** Charts the version had that were deleted since: they come back from the trash only. */
  chartsNotBrought: string[]
}

/** `lakehouse_bi::trash::TrashItem`. */
export type TrashItem = {
  kind: ItemKind
  id: string
  name: string
  /** For a chart, the dashboard it was on. */
  board: string
  boardName: string
  /** For a chart: whether that dashboard is `live`, in the `trash` or `gone`. */
  boardState: "" | "live" | "trash" | "gone"
  deletedBy: string
  deletedAt: string
  expiresAt: string
  personal: boolean
}

export type TrashList = { items: TrashItem[]; retentionDays: number }

export type RestoredItem = {
  kind: ItemKind
  id: string
  /** For a dashboard, how many charts came back with it. */
  charts: number
  boardState: "" | "live" | "trash" | "gone"
}

/** A favourite, a recent item or a search hit (`lakehouse_bi::items::Entry`). */
export type ShortcutItem = {
  kind: ItemKind
  id: string
  name: string
  /** For a chart, the dashboard it is on. */
  board?: string
  boardName?: string
}

export type Shortcuts = { favourites: ShortcutItem[]; recent: ShortcutItem[] }

/**
 * `GET /api/dashboard/embed-info` (`embed_info_body`,
 * `rust/crates/lakehouse-api/src/routes/dashboard.rs`). When `supported` is
 * false no secret is configured: `reason` says so and there is no
 * `sampleToken`.
 */
export type EmbedInfo = {
  enabled: boolean
  supported: boolean
  reason?: string
  sampleToken?: string
  /** The longest `exp - iat` the API accepts, in seconds. */
  maxLifetimeSeconds: number
  /** Unix seconds; tokens issued at or before it are withdrawn. `null` when none. */
  revokedBefore: number | null
  /** Sites allowed to frame the embed pages. */
  allowedOrigins: string[]
}

export type CreateBoardInput = {
  name: string
  description?: string
}

/**
 * Field yang tidak dikirim tidak akan diubah — bukan dikosongkan. Karena
 * itu `description: ""` adalah cara sah untuk MENGHAPUS deskripsi, dan
 * berbeda maknanya dari `description: undefined`.
 */
export type UpdateBoardInput = {
  id: string
  name?: string
  description?: string
}

/** One column a source returns — `SourceColumn` (`type` on the wire). */
export type SqlSourceColumn = {
  name: string
  type: string
}

/** `SqlSource` (`#[serde(rename_all = "camelCase")]`). */
export type SqlSource = {
  id: string
  title: string
  sql: string
  columns: SqlSourceColumn[]
  /** Empty string = root. */
  folderId: string
  /** Principal id of whoever last saved it. */
  createdBy: string
  /** When this version was saved; every save is a new version. */
  updatedAt?: string
}

/** `POST`/`PUT /api/dashboard/sources` body. */
export type SaveSqlSourceInput = {
  title: string
  sql: string
  folderId?: string
}

/** `POST /api/dashboard/sources/preview` response. */
export type SqlSourcePreview = {
  columns: SqlSourceColumn[]
  rows: Record<string, unknown>[]
  /**
   * Only when the request carried a `chart`: the render spec and rows, in
   * the shape `POST /api/dashboard/specs/preview` answers with (`result`
   * is `{ error }` when the chart's own query was refused).
   */
  chart?: { spec: Record<string, unknown>; result: Record<string, unknown> }
}

/** One suggested pair: `rank` says why (`keys`, `name_and_type` or `name`); `suggested` pairs are applied by default. */
export type JoinSuggestion = {
  left: string
  right: string
  rank: "keys" | "name_and_type" | "name" | string
  suggested: boolean
}

/** `GET /api/dashboard/join-suggestions` (`lakehouse_bi::joins::Suggestions`). */
export type JoinSuggestions = {
  left: string
  right: string
  pairs: JoinSuggestion[]
  /** The right table's sorting key (a set). */
  rightKey: string[]
  /** Whether the suggested right-hand columns cover that key; when not, rows may multiply. */
  coversRightKey: boolean
}

/**
 * A dashboard folder — `lakehouse_bi::folders::Folder`. The API returns a
 * flat list; `lib/folder-tree.ts` builds the tree from `parentId`.
 */
export type DashboardFolder = {
  id: string
  name: string
  /** Empty string = root. */
  parentId: string
  createdBy: string
  updatedAt?: string
  /**
   * The principal whose personal space this is (BI-18 part C, part 2); absent
   * for an ordinary folder. A personal space holds dashboards only and cannot
   * be renamed, moved or deleted.
   */
  ownerId?: string
}

/**
 * A tile (or an assistant tool) that failed. `error` is a fixed sentence the
 * server wrote; the database's own text never reaches it (SEC-11). `errorId`
 * is the reference under which the server logged the raw error, present when
 * the failure came from an upstream and absent for a message the product
 * wrote itself (a refused statement, a deleted source). Mirrors
 * `rust/crates/lakehouse-api/src/upstream_error.rs` (`UpstreamFailure::to_json`).
 */
export type TileFailure = { error: string; errorId?: string }
