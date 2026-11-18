/**
 * `/api/usage/queries`, `/api/usage/queries/export`, `/api/usage/rates` and
 * `/api/usage/storage` (`OPS-8` part B and C): the runs of a period summed
 * per day, person, engine or tenant, the same sums as a CSV file, the rates
 * behind the estimate in money, and the size of every layer.
 *
 * Field names mirror the API's serde names. Reading needs `usage:read`; adding
 * a rate needs `settings:write`. Every figure is a sum of recorded values. The
 * only money is `estimate`, computed from the rates an administrator entered,
 * and it is `null` when no rate applied.
 */

/** How the runs are grouped. `tenant` is for a caller holding `*:*`. */
export type UsageGroup = "day" | "person" | "engine" | "tenant" | string

/** What a request asks for: `YYYY-MM-DD` UTC days, `to` included. */
export type UsageQuery = {
  from: string
  to: string
  group: UsageGroup
  /** `"all"` reads every tenant and is allowed only for `*:*`; left out, the caller's own tenant. */
  tenant?: "all"
}

/** One group of runs, or the totals of the period. */
export type UsageRow = {
  /** The day, the person's id, the engine or the tenant. */
  key: string
  /** What to show for the key: a person's name when the caller may read it, else the id. */
  label: string
  queries: number
  completed: number
  failed: number
  blocked: number
  /** Stopped runs are counted here. */
  cancelled: number
  scannedBytes: number
  /** Wall-clock milliseconds, not CPU time. */
  engineMs: number
  /** A decimal with six places, or `null` when no rate applied. */
  estimate: string | null
}

export type UsageReport = {
  from: string
  to: string
  group: UsageGroup
  /** `"all"` for every tenant, `"tenant"` for the caller's own. */
  scope: "all" | "tenant" | string
  /** The currency of the estimate; `null` while no rate is entered. */
  currency: string | null
  rows: UsageRow[]
  totals: UsageRow
}

/** The CSV file the server built for a period, as the browser saves it. */
export type UsageExport = {
  blob: Blob
  /** The name the server gave the file: `usage-<from>-<to>.csv`. */
  fileName: string
}

export type UsageRateKind = "scan_tb" | "engine_hour" | "storage_gb_month" | string

export type UsageRate = {
  id: string
  kind: UsageRateKind
  currency: string
  /** A decimal with six places. */
  amount: string
  /** The first `YYYY-MM-DD` day the rate applies on. */
  effectiveFrom: string
  /** RFC 3339. */
  createdAt: string
}

export type UsageRates = {
  /** The one currency every rate uses; `null` while none is entered. */
  currency: string | null
  /** Newest `effectiveFrom` first, then the newest entered. */
  rates: UsageRate[]
}

/** A rate to add. A rate is never changed or removed: a new price is a new rate. */
export type NewUsageRate = {
  kind: UsageRateKind
  /** Three capital letters; the same one as the rates already entered. */
  currency: string
  /** Not negative, at most 12 digits before the point and 6 after. */
  amount: string
  effectiveFrom: string
}

/** What a storage read asks for: `YYYY-MM-DD` UTC days, `to` included. */
export type UsageStorageQuery = {
  from: string
  to: string
}

/**
 * One layer's measured size for one day. The bytes were measured once a day
 * by the storage job, never guessed.
 */
export type UsageStorageRow = {
  /** The UTC day the measurement was taken on. */
  day: string
  /** The layer: `bronze`, `gold`, `silver` or `serving`. */
  layer: string
  /** How many tables the layer holds. */
  tableCount: number
  /** The bytes of the tables that were measured. */
  totalBytes: number
  /** How many of the layer's tables could not be measured. */
  tablesSkipped: number
  /** Why they were skipped; `null` when none was. */
  skippedReason: string | null
  /** A decimal with six places, or `null` when no rate applied on that day. */
  estimate: string | null
}

/**
 * The size of every layer for a period. Only a caller holding `*:*` may read
 * it: a layer holds the tables of every tenant.
 */
export type UsageStorageReport = {
  from: string
  to: string
  /** The currency of the storage rate; `null` while no rate is entered. */
  currency: string | null
  /** One row per layer per day, oldest first. */
  rows: UsageStorageRow[]
}

/**
 * Every failure rejects with a `ServiceError` whose message is the API's own
 * fixed sentence: a bad date, a period over 366 days, `tenant=all` without
 * `*:*`, a second currency.
 */
export interface UsageService {
  /** The sums of a period. A caller with no tenant gets no rows, not an error. */
  getUsage(query: UsageQuery, signal?: AbortSignal): Promise<UsageReport>
  /** The same sums as a CSV file; the server records the export in the audit log. */
  exportUsageCsv(query: UsageQuery, signal?: AbortSignal): Promise<UsageExport>
  listRates(signal?: AbortSignal): Promise<UsageRates>
  /** Adds a rate; needs `settings:write`. */
  addRate(rate: NewUsageRate, signal?: AbortSignal): Promise<UsageRate>
  /**
   * The size of every layer. The server refuses it with 403 unless the
   * caller holds `*:*`, so only the platform operator asks for it.
   */
  getStorage(query: UsageStorageQuery, signal?: AbortSignal): Promise<UsageStorageReport>
}
