import type { FilterDef } from "../clients/bi-store"
import type { TileLimit } from "./settings"

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
/** Tile position on the grid canvas (12 columns). Key = chartId. */
export type TileBox = {
  x: number; y: number; w: number; h: number;
  /** The tab this tile sits on (BI-18 part C). Absent: the first tab, or no tabs. */
  tab?: string;
};
/** A tab of a dashboard (BI-18 part C); `id` is stable across renames. Mirrors `lakehouse_bi::layout::Tab`. */
export type BoardTab = { id: string; title: string };
/** A tile in the phone layout: its place in the one column and its height in rows. */
export type PhoneTile = { id: string; h: number };
export type LayoutMap = Record<string, TileBox>;
export type Board = {
  id: string;
  name: string;
  /** Satu kalimat tujuan board, tampil di halaman daftar dashboard. */
  description?: string;
  /** Nama tampilan pembuat board. Kosong untuk board lama / tanpa login. */
  createdBy?: string;
  /** Folder this board is filed in; empty or null = no folder. */
  folderId?: string | null;
  /** The principal whose personal space holds this board (BI-18 part C, part 2); absent when it is shared. */
  ownerId?: string;
  layout?: LayoutMap;
  filters?: FilterDef[];
  /** BI-18 part C: absent when the dashboard has no tabs. */
  tabs?: BoardTab[];
  /** BI-18 part C: the order and heights of the tiles below 640 px, when an editor saved them. */
  phoneLayout?: PhoneTile[];
  /**
   * The auto-refresh an editor saved, in seconds (one of 60, 300, 600, 900,
   * 1800, 3600). Absent when none was saved; BI-18·B.
   */
  refreshSeconds?: number;
  /** BI-17 part A: the dashboard's own theme (`light` or `dark`) and font id; absent follows the console. */
  theme?: string;
  font?: string;
  createdAt?: string;
  /**
   * Kapan board terakhir ditulis.
   *
   * Nilainya sama dengan `createdAt`, dan itu disengaja: tabelnya
   * `ReplacingMergeTree(created_at)`, jadi `created_at` adalah kolom versi
   * yang ditulis ulang `now()` setiap kali board di-rename / layout
   * disimpan / filter diubah. Jadi sejak awal isinya memang "terakhir
   * diubah", cuma namanya saja "created". Di UI, labeli **"Updated"** —
   * jangan pernah "Created", karena itu klaim yang tidak benar.
   */
  updatedAt?: string;
  /** Jumlah tile di board ini; dihitung server, bukan kolom tersimpan. */
  chartCount?: number;
  /** `true` hanya untuk board bawaan (`default`) yang tidak bisa dihapus. */
  builtin?: boolean;
  publicToken?: string;
  embedEnabled?: boolean;
};

export type DashboardService = {
  /** Semua board, board bawaan (`default`) di urutan pertama. */
  listBoards(signal?: AbortSignal): Promise<Board[]>
  createBoard(input: CreateBoardInput, signal?: AbortSignal): Promise<Board>
  /** Salin sebuah board beserta seluruh chart-nya. */
  duplicateBoard(id: string, signal?: AbortSignal): Promise<Board>
  updateBoard(input: UpdateBoardInput, signal?: AbortSignal): Promise<void>
  deleteBoard(id: string, signal?: AbortSignal): Promise<void>
  /**
   * Save the tile layout of a dashboard (`PUT /api/dashboard/boards`). The
   * console's own write; it needs `dashboard:write`.
   */
  saveBoardLayout(id: string, layout: LayoutMap, signal?: AbortSignal): Promise<void>
  /**
   * Delete one saved chart card (`DELETE /api/dashboard/specs`). It needs
   * `dashboard:write`.
   */
  deleteChart(id: string, signal?: AbortSignal): Promise<void>

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
  getEmbedInfo(board: string, signal?: AbortSignal, chart?: string): Promise<EmbedInfo>
  /** `BI-26`: save each filter's embed mode. Needs `dashboard:write`. */
  setEmbedFilterModes(board: string, modes: Record<string, FilterMode>, signal?: AbortSignal): Promise<void>
  /** `BI-26`: the charts of a dashboard, for the chart picker of the embed section. */
  listBoardCharts(board: string, signal?: AbortSignal): Promise<BoardChart[]>
  /** `BI-19` part A. Needs `dashboard:write`. */
  getPublicLink(board: string, signal?: AbortSignal): Promise<PublicLinkState>
  /** Make a link or change its expiry or password; refusals carry the server's sentence. */
  savePublicLink(input: PublicLinkInput, signal?: AbortSignal): Promise<PublicLink>
  /** End the link at once. */
  endPublicLink(board: string, signal?: AbortSignal): Promise<void>
  /**
   * `BI-19` part E: comments on a dashboard. Needs view on it; a dashboard the
   * caller cannot see answers like one that does not exist. Refusals arrive as
   * the server's sentence.
   */
  listComments(board: string, signal?: AbortSignal): Promise<DashboardComment[]>
  postComment(input: CommentInput, signal?: AbortSignal): Promise<DashboardComment>
  /** Author only. `mentions` omitted keeps the people already mentioned. */
  editComment(id: string, body: string, mentions?: string[], signal?: AbortSignal): Promise<DashboardComment>
  /** Author, or somebody who manages the dashboard. */
  deleteComment(id: string, signal?: AbortSignal): Promise<void>
  /** The @ picker: people who can see the dashboard (no addresses). */
  commentPeople(board: string, query: string, signal?: AbortSignal): Promise<CommentPerson[]>
  /**
   * `BI-19` part B: who has what on a dashboard or folder. Needs manage on the
   * item; anything else arrives as the server's sentence (403 for one the
   * caller can see and does not manage, 404 for one they cannot see).
   */
  getAccess(kind: AccessKind, id: string, signal?: AbortSignal): Promise<ItemAccess>
  /** Replace the item's rules and its "only the people listed" mark. */
  saveAccess(input: ItemAccessInput, signal?: AbortSignal): Promise<ItemAccess>
  /** People and roles for the picker; at most 20 people. */
  findGrantees(query: string, signal?: AbortSignal): Promise<GranteeMatches>
  /** Who gains and who loses sight of a dashboard when it moves to `folderId` (`""` = root). */
  previewMove(board: string, folderId: string, signal?: AbortSignal): Promise<MoveEffect>
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
  /**
   * `GET /api/dashboard/charts/{id}/lineage` (`BI-14` plan D1): the path from a
   * chart to its sources. Needs `lineage:read`; without it the server answers
   * 403 and the call throws `permission_denied`. **Not on the server yet** (T2
   * of the BI-14 plan).
   */
  chartLineage(id: string, signal?: AbortSignal): Promise<ChartLineage>
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
  /** Titles of charts the version had that were in the trash: the revert took them out of it. */
  chartsRestored: string[]
  /** Titles of charts the version had that were removed from the trash for good: they could not come back. */
  chartsGone: string[]
  /** Titles of charts the version had that now sit on another dashboard: they are not on this one any more. */
  chartsMoved: string[]
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
  /** `BI-26`: each filter column's embed mode; a column not listed is hidden. */
  filterModes?: Record<string, FilterMode>
  /** `BI-26`: the dashboard's saved filter columns, the ones that can be given a mode. */
  filterColumns?: string[]
  /** `BI-26` review fix R3: filter columns that do not pick values; they can only be hidden. */
  hiddenOnlyColumns?: string[]
}

/** What a viewer of a signed embed may do with one dashboard filter (`BI-26`). */
export type FilterMode = "locked" | "editable" | "hidden"

/** A chart of a dashboard, for the embed chart picker. */
export type BoardChart = { id: string; title: string }

/**
 * A dashboard's public link (`BI-19` part A), from `GET`/`PUT
 * /api/dashboard/public-link`. The password is never sent back: only
 * `hasPassword`.
 */
export type PublicLink = {
  token: string
  /** RFC 3339. */
  expiresAt: string
  /** True once `expiresAt` has passed; the link no longer works. */
  expired: boolean
  hasPassword: boolean
  createdAt: string
}

export type PublicLinkState = {
  /** False while an administrator has switched public links off (Admin > Settings). */
  enabled: boolean
  link: PublicLink | null
}

/**
 * What a save sends. `password` absent keeps it, `null` removes it, a string
 * (8 to 128 characters) sets it; `newLink` makes a new token even when a live
 * link exists.
 */
export type PublicLinkInput = {
  board: string
  expiresAt: string
  password?: string | null
  newLink?: boolean
}

/** What a person may do with a dashboard or folder (`BI-19` part B, decision B1). */
export type AccessLevel = "view" | "edit" | "manage"
export type AccessKind = "board" | "folder"

/** One rule: a level for a person or a role. `email` is only on a person. */
export type AccessGrantee = {
  kind: "user" | "role"
  id: string
  name: string
  email?: string
  /** `| string` widening: a level this console does not know is shown, not dropped. */
  level: AccessLevel | string
}

/** A rule that reaches the item from a folder above it. */
export type InheritedGrantee = AccessGrantee & { folderId: string; folderName: string }

/** `GET /api/dashboard/access`. */
export type ItemAccess = {
  kind: AccessKind
  id: string
  yourLevel: AccessLevel | string
  /** Marked "only the people listed" (feature B3). */
  restricted: boolean
  rules: AccessGrantee[]
  inherited: InheritedGrantee[]
  /** Folders above that are marked: the item is restricted by them. */
  restrictedAbove: { folderId: string; folderName: string }[]
}

export type ItemAccessInput = {
  kind: AccessKind
  id: string
  restricted: boolean
  rules: { userId?: string; roleId?: string; level: AccessLevel }[]
}

export type GranteeMatches = {
  people: { id: string; name: string; email: string }[]
  roles: { id: string; name: string }[]
}

/** `GET /api/dashboard/access-preview` (feature B7). */
export type MoveEffect = {
  gains: { kind: "user" | "role"; id: string; name: string }[]
  loses: { kind: "user" | "role"; id: string; name: string }[]
  /** Everyone whose role gives them the dashboard stops seeing it (the destination is restricted). */
  startingPointLoses: boolean
  /** Everyone whose role gives them the dashboard starts seeing it (it leaves a restricted folder). */
  startingPointGains: boolean
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
export type TileFailure = {
  error: string
  errorId?: string
  /** `BI-5` plan D6: set when the time limit ended the tile; the console says the limit. Not on the server yet. */
  limit?: TileLimit
}

/**
 * One step of a chart's path (`BI-14` plan D1). `kind` is `chart`,
 * `sql_source`, `metric`, `gold`, `silver`, `bronze`, `connector`, `upload` or
 * `pipeline`. `refreshedAt` is an ISO time; `refreshSource` names the record it
 * comes from ("last Gold build", "Iceberg last update", "last connector run",
 * "upload time"); `notRecorded` is true when no record exists, and then there is
 * no `refreshedAt` (never a guess).
 */
export type LineageStep = {
  id: string
  kind: "chart" | "sql_source" | "metric" | "gold" | "silver" | "bronze" | "connector" | "upload" | "pipeline" | string
  label: string
  refreshedAt?: string
  refreshSource?: string
  notRecorded?: boolean
}

/** A recorded connection between two steps; `from` feeds `to`. Same shape as the lineage page's edges. */
export type LineageStepEdge = { id: string; from: string; to: string; kind: string; evidence?: string }

/** The columns one field of the chart comes from, step by step (`via` names the step: a build, a formula or a SQL source). */
export type FieldColumns = { field: string; columns: { table: string; column: string; via: string }[] }

/**
 * `GET /api/dashboard/charts/{id}/lineage` (`BI-14` plan D1). `chart` is the
 * chart's id and title (the plan names the field, not its shape: the console's
 * choice, the contract for T2). `steps` come in path order, the chart first.
 * `coverage` lists in words what the path leaves out.
 */
export type ChartLineage = {
  chart: { id: string; title: string }
  steps: LineageStep[]
  edges: LineageStepEdge[]
  fields: FieldColumns[]
  coverage: string[]
  supported: boolean
}

/** A comment as the server returns it (`BI-19` part E). `body` is plain text. */
export type DashboardComment = {
  id: string
  board: string
  /** `null` once the author's account is gone. */
  author: { id: string; name: string } | null
  body: string
  createdAt: string
  editedAt: string | null
  mentions: { id: string; name: string }[]
  /** Only the author edits. */
  canEdit: boolean
  /** The author, or somebody who manages the dashboard. */
  canDelete: boolean
}

export type CommentInput = {
  board: string
  body: string
  /** User ids; each must be able to see the dashboard or the server refuses. */
  mentions: string[]
}

export type CommentPerson = { id: string; name: string }
