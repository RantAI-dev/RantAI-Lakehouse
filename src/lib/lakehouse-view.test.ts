import { describe, expect, it } from "bun:test"
import {
  formatSignedBytes,
  isIcebergCandidate,
  lakehouseCapacityUrl,
  lakehouseCompactionStatusUrl,
  lakehouseMaintenanceUrl,
  lakehouseNamespacesUrl,
  lakehouseTableDetailUrl,
  lakehouseTableHref,
  lakehouseTablesUrl,
  maintenanceSummary,
  msToIso,
  snapshotRelativeTime,
  snapshotsNewestFirst,
  systemCompactionWarningMessage,
  tableCompactionWarningMessage,
  tableNeedsCompactionWarning,
  tablesNeedingCompaction,
  NEEDS_COMPACTION_FILE_COUNT,
} from "./lakehouse-view"
import type { LakehouseMaintenance, LakehouseSnapshot } from "@/services/contracts/lakehouse"

function snapshot(partial: Partial<LakehouseSnapshot>): LakehouseSnapshot {
  return {
    id: "1",
    parentId: null,
    timestampMs: 0,
    operation: "append",
    author: null,
    summary: {
      addedRecords: null,
      deletedRecords: null,
      totalRecords: null,
      totalDataFiles: null,
    },
    ...partial,
  }
}


describe("lakehouseNamespacesUrl", () => {
  it("omits the warehouse query when unset", () => {
    expect(lakehouseNamespacesUrl()).toBe("/api/lakehouse/namespaces")
  })

  it("encodes a warehouse value containing a space", () => {
    expect(lakehouseNamespacesUrl("my warehouse")).toBe(
      "/api/lakehouse/namespaces?warehouse=my%20warehouse"
    )
  })
})

describe("lakehouseTablesUrl", () => {
  it("encodes a namespace containing a slash", () => {
    expect(lakehouseTablesUrl("bronze/raw")).toBe(
      "/api/lakehouse/tables?namespace=bronze%2Fraw"
    )
  })

  it("includes an encoded warehouse when given", () => {
    expect(lakehouseTablesUrl("bronze", "my warehouse")).toBe(
      "/api/lakehouse/tables?namespace=bronze&warehouse=my%20warehouse"
    )
  })
})

describe("lakehouseTableDetailUrl", () => {
  it("encodes a table name containing a space and a slash", () => {
    expect(lakehouseTableDetailUrl("bronze", "orders/2026 q1")).toBe(
      "/api/lakehouse/tables/bronze/orders%2F2026%20q1"
    )
  })
})

describe("lakehouseCapacityUrl", () => {
  it("has no query parameters", () => {
    expect(lakehouseCapacityUrl()).toBe("/api/lakehouse/capacity")
  })
})

describe("formatSignedBytes", () => {
  it("prefixes a positive delta with +", () => {
    expect(formatSignedBytes(1536)).toBe("+1.5 KB")
  })

  it("prefixes a negative delta with - and formats its magnitude", () => {
    expect(formatSignedBytes(-1536)).toBe("-1.5 KB")
  })

  it("treats a zero delta as a genuine measured zero, not a dash", () => {
    expect(formatSignedBytes(0)).toBe("+0 B")
  })
})

describe("lakehouseMaintenanceUrl", () => {
  it("appends /maintenance to the encoded detail URL", () => {
    expect(lakehouseMaintenanceUrl("bronze", "orders")).toBe(
      "/api/lakehouse/tables/bronze/orders/maintenance"
    )
  })
})

describe("lakehouseTableHref", () => {
  it("encodes a namespace/table pair with a space", () => {
    expect(lakehouseTableHref("bronze", "my table")).toBe(
      "/lakehouse/tables/bronze/my%20table"
    )
  })
})

describe("snapshotsNewestFirst", () => {
  it("sorts by timestampMs descending", () => {
    const oldestFirst = [
      snapshot({ id: "1", timestampMs: 1_000 }),
      snapshot({ id: "2", timestampMs: 3_000 }),
      snapshot({ id: "3", timestampMs: 2_000 }),
    ]
    expect(snapshotsNewestFirst(oldestFirst).map((s) => s.id)).toEqual(["2", "3", "1"])
  })

  it("never mutates the input array", () => {
    const oldestFirst = [
      snapshot({ id: "1", timestampMs: 1_000 }),
      snapshot({ id: "2", timestampMs: 2_000 }),
    ]
    const copy = [...oldestFirst]
    snapshotsNewestFirst(oldestFirst)
    expect(oldestFirst).toEqual(copy)
  })

  // A5-F1: a snapshot id above Number.MAX_SAFE_INTEGER (2^53−1) must
  // survive the sort exactly, as a string — never coerced through `Number`.
  it("preserves a 64-bit snapshot id string exactly, past Number.MAX_SAFE_INTEGER", () => {
    const bigId = "9007199254740993"
    const oldestFirst = [
      snapshot({ id: bigId, timestampMs: 1_000 }),
      snapshot({ id: "2", timestampMs: 2_000 }),
    ]
    expect(snapshotsNewestFirst(oldestFirst).map((s) => s.id)).toEqual(["2", bigId])
  })
})

describe("isIcebergCandidate", () => {
  it("is true for a bronze asset with a non-empty tableName", () => {
    expect(isIcebergCandidate({ layer: "bronze", tableName: "orders" })).toBe(true)
  })

  it("is false for a non-bronze layer even with a tableName", () => {
    expect(isIcebergCandidate({ layer: "silver", tableName: "orders" })).toBe(false)
  })

  it("is true for an iceberg-table asset in a non-bronze layer", () => {
    expect(
      isIcebergCandidate({ layer: "raw", type: "iceberg-table", tableName: "orders" })
    ).toBe(true)
  })

  it("is false for an iceberg-table asset without a tableName", () => {
    expect(isIcebergCandidate({ layer: "raw", type: "iceberg-table", tableName: null })).toBe(
      false
    )
  })

  it("is false when tableName is null", () => {
    expect(isIcebergCandidate({ layer: "bronze", tableName: null })).toBe(false)
  })

  it("is false when tableName is undefined", () => {
    expect(isIcebergCandidate({ layer: "bronze" })).toBe(false)
  })

  it("is false when tableName is an empty string", () => {
    expect(isIcebergCandidate({ layer: "bronze", tableName: "" })).toBe(false)
  })
})

describe("msToIso", () => {
  it("converts an epoch-millisecond timestamp to an ISO string", () => {
    expect(msToIso(0)).toBe("1970-01-01T00:00:00.000Z")
  })
})

describe("snapshotRelativeTime", () => {
  it("renders a relative age from an epoch-millisecond timestamp", () => {
    const now = Date.parse("2026-01-01T01:00:00.000Z")
    const timestampMs = Date.parse("2026-01-01T00:00:00.000Z")
    expect(snapshotRelativeTime(timestampMs, now)).toBe("1h ago")
  })
})

function maintenance(partial: Partial<LakehouseMaintenance>): LakehouseMaintenance {
  return {
    namespace: "bronze",
    tableName: "orders",
    configured: false,
    snapshotsToKeep: null,
    orphanAgeHours: null,
    compactSmallFiles: false,
    schedule: null,
    lastRun: null,
    lastVerbRuns: [],
    ...partial,
  }
}

describe("maintenanceSummary", () => {
  it("names the default behavior when unconfigured", () => {
    expect(maintenanceSummary(maintenance({ configured: false }))).toBe(
      "No policy — default maintenance (orphan-file removal only)"
    )
  })

  it("summarizes a configured policy's fields", () => {
    const summary = maintenanceSummary(
      maintenance({
        configured: true,
        snapshotsToKeep: 10,
        orphanAgeHours: 48,
        compactSmallFiles: true,
        schedule: "daily",
      })
    )
    expect(summary).toContain("keep the 10 newest snapshots")
    expect(summary).toContain("remove orphan files older than 48h")
    expect(summary).toContain("compact small files")
    expect(summary).toContain("schedule: daily")
  })

  it("says unset for a configured policy missing snapshotsToKeep/orphanAgeHours", () => {
    const summary = maintenanceSummary(
      maintenance({ configured: true, snapshotsToKeep: null, orphanAgeHours: null })
    )
    expect(summary).toContain("keep snapshots: not set")
    expect(summary).toContain("orphan-file age: not set")
    expect(summary).toContain("no small-file compaction")
  })
})

describe("lakehouseCompactionStatusUrl", () => {
  it("points at the single System-status compaction endpoint", () => {
    expect(lakehouseCompactionStatusUrl()).toBe("/api/lakehouse/compaction-status")
  })
})

// OPS-21 / owner decision 2026-11-24: 20 is the G3-measured bar
// (`docs/plans/G3-RESULT.md`, ADR 0009), not a guessed tolerance; changing it
// is an owner decision and must update both this and the Rust constant.
describe("NEEDS_COMPACTION_FILE_COUNT", () => {
  it("is pinned at the measured 20", () => {
    expect(NEEDS_COMPACTION_FILE_COUNT).toBe(20)
  })
})

// OPS-21: the warning is driven only by a MEASURED count and a confirmed-off
// Trino. These pure helpers are the same ones both UI surfaces call.
describe("tableNeedsCompactionWarning", () => {
  it("warns from a measured count at the need count when Trino is off", () => {
    expect(
      tableNeedsCompactionWarning({
        smallFileCount: NEEDS_COMPACTION_FILE_COUNT,
        compactionAvailable: false,
      })
    ).toBe(true)
  })

  it("is silent when the count was not measured", () => {
    expect(tableNeedsCompactionWarning({ smallFileCount: null, compactionAvailable: false })).toBe(
      false
    )
  })

  it("is silent when the count is below the need count", () => {
    expect(
      tableNeedsCompactionWarning({
        smallFileCount: NEEDS_COMPACTION_FILE_COUNT - 1,
        compactionAvailable: false,
      })
    ).toBe(false)
  })

  // Literal boundaries: a change to the constant must not move the warning.
  it("is silent at a literal 19 measured files when Trino is off", () => {
    expect(tableNeedsCompactionWarning({ smallFileCount: 19, compactionAvailable: false })).toBe(
      false
    )
  })

  it("warns at a literal 20 measured files when Trino is off", () => {
    expect(tableNeedsCompactionWarning({ smallFileCount: 20, compactionAvailable: false })).toBe(
      true
    )
  })

  it("is silent when Trino can compact", () => {
    expect(tableNeedsCompactionWarning({ smallFileCount: 999, compactionAvailable: true })).toBe(
      false
    )
  })

  it("is silent when the API did not send the Trino flag", () => {
    expect(tableNeedsCompactionWarning({ smallFileCount: 999 })).toBe(false)
  })
})

describe("tableCompactionWarningMessage", () => {
  it("names the measured count, the threshold, and the trino profile fix", () => {
    const message = tableCompactionWarningMessage(42, 32 * 1024 * 1024)
    expect(message).toContain("42")
    expect(message).toContain("32.0 MB")
    expect(message).toContain("--profile trino")
    expect(message).toContain("trino profile")
  })
})

describe("systemCompactionWarningMessage", () => {
  it("names the trino profile fix and agrees with the table count", () => {
    const one = systemCompactionWarningMessage(1)
    const many = systemCompactionWarningMessage(3)
    expect(one).toContain("1 table has")
    expect(many).toContain("3 tables have")
    expect(one).toContain("--profile trino")
    expect(many).toContain("trino profile")
  })
})

describe("tablesNeedingCompaction", () => {
  const status = {
    compactionAvailable: false,
    needsCompactionFileCount: NEEDS_COMPACTION_FILE_COUNT,
    tables: [
      { namespace: "bronze", name: "orders", smallFileCount: 42 },
      { namespace: "bronze", name: "clean", smallFileCount: 1 },
      { namespace: "bronze", name: "events", smallFileCount: NEEDS_COMPACTION_FILE_COUNT },
    ],
  }

  it("lists only measured tables at or above the need count, sorted", () => {
    expect(tablesNeedingCompaction(status).map((t) => t.name)).toEqual(["events", "orders"])
  })

  // The System-status body carries its own need count; the console must honour
  // that server value rather than re-apply its table-page constant. Here 22 is
  // above the console's 20 but below the server's 25, so a constant-based
  // filter would wrongly list it.
  it("uses the server's need count when it differs from the console constant", () => {
    const serverStatus = {
      compactionAvailable: false,
      needsCompactionFileCount: 25,
      tables: [
        { namespace: "bronze", name: "twenty_two", smallFileCount: 22 },
        { namespace: "bronze", name: "twenty_six", smallFileCount: 26 },
      ],
    }
    expect(tablesNeedingCompaction(serverStatus).map((t) => t.name)).toEqual(["twenty_six"])
  })

  it("is empty when Trino is available, however high the counts", () => {
    expect(tablesNeedingCompaction({ ...status, compactionAvailable: true })).toEqual([])
  })

  it("is empty when the API did not send the Trino flag", () => {
    const withoutFlag = {
      tables: status.tables,
      needsCompactionFileCount: NEEDS_COMPACTION_FILE_COUNT,
    } as unknown as Parameters<typeof tablesNeedingCompaction>[0]
    expect(tablesNeedingCompaction(withoutFlag)).toEqual([])
  })
})
