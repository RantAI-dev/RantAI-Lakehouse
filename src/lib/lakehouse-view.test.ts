import { describe, expect, it } from "bun:test"
import {
  isIcebergCandidate,
  lakehouseMaintenanceUrl,
  lakehouseNamespacesUrl,
  lakehouseTableDetailUrl,
  lakehouseTableHref,
  lakehouseTablesUrl,
  lakehouseWarehousesUrl,
  maintenanceSummary,
  msToIso,
  snapshotRelativeTime,
  snapshotsNewestFirst,
} from "./lakehouse-view"
import type { LakehouseMaintenance, LakehouseSnapshot } from "@/services/contracts/lakehouse"

function snapshot(partial: Partial<LakehouseSnapshot>): LakehouseSnapshot {
  return {
    id: "1",
    parentId: null,
    timestampMs: 0,
    operation: "append",
    summary: {
      addedRecords: null,
      deletedRecords: null,
      totalRecords: null,
      totalDataFiles: null,
    },
    ...partial,
  }
}

describe("lakehouseWarehousesUrl", () => {
  it("has no query parameters", () => {
    expect(lakehouseWarehousesUrl()).toBe("/api/lakehouse/warehouses")
  })
})

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
