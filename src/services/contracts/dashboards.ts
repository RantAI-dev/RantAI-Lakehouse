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
}

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
