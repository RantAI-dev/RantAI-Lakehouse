import { strict as assert } from "node:assert"
import { test } from "node:test"
import type { PlatformService } from "@/services/contracts/ops"
import type { MaintenanceRun, ReplicationSlot } from "@/services/contracts/governance"
import {
  latestRunPerTable,
  latestSlotReadings,
  maintenanceTableHref,
  overallServiceState,
} from "./system-status"

function service(name: string, health: PlatformService["health"], checked = true): PlatformService {
  return {
    id: name.toLowerCase(),
    name,
    health,
    checked,
    version: null,
    site: "local",
    replicas: null,
    errorRate: null,
    latencyMs: null,
    dependencies: [],
  }
}

test("overallServiceState says every probed service is up, and names none as down", () => {
  const state = overallServiceState([service("ClickHouse", "healthy"), service("Dagster", "healthy")])
  assert.equal(state.tone, "success")
  assert.equal(state.headline, "All 2 probed services are up.")
  assert.deepEqual(state.down, [])
})

test("overallServiceState names the probed service that is not healthy", () => {
  const state = overallServiceState([service("ClickHouse", "unhealthy"), service("Dagster", "healthy")])
  assert.equal(state.tone, "destructive")
  assert.equal(state.headline, "ClickHouse is not healthy.")
})

test("overallServiceState never counts an unprobed service as up", () => {
  const state = overallServiceState([service("ClickHouse", "healthy"), service("Trino", "unknown", false)])
  assert.equal(state.tone, "warning")
  assert.equal(state.headline, "The one probed service is up.")
  assert.deepEqual(state.notProbed, ["Trino"])
})

test("overallServiceState says it does not know when nothing was probed", () => {
  const state = overallServiceState([service("Trino", "unknown", false)])
  assert.equal(state.tone, "neutral")
  assert.match(state.headline, /not known/)
})

function slot(connectorId: string, slotName: string, checkedAt: string, status = "ok"): ReplicationSlot {
  return {
    connectorId,
    slotName,
    checkedAt,
    active: true,
    walRetainedBytes: "0",
    confirmedFlushLagBytes: "0",
    status,
  }
}

test("latestSlotReadings keeps the newest reading of each slot, newest first", () => {
  const rows = latestSlotReadings([
    slot("a", "s1", "2026-11-04T10:00:00Z", "critical"),
    slot("b", "s2", "2026-11-04T09:45:00Z"),
    slot("a", "s1", "2026-11-04T09:45:00Z", "ok"),
  ])
  assert.deepEqual(
    rows.map((r) => [r.connectorId, r.status]),
    [
      ["a", "critical"],
      ["b", "ok"],
    ]
  )
})

test("latestSlotReadings keeps two slots of one connector apart", () => {
  const rows = latestSlotReadings([
    slot("a", "s1", "2026-11-04T10:00:00Z"),
    slot("a", "s2", "2026-11-04T10:00:00Z"),
  ])
  assert.equal(rows.length, 2)
})

function run(tableName: string, runAt: string): MaintenanceRun {
  const counts = { deletedDataFiles: "0", deletedManifestFiles: "0" }
  return { tableName, runAt, dryRun: counts, applied: counts, skippedVerbs: "" }
}

test("latestRunPerTable keeps each table's newest run, newest first", () => {
  const rows = latestRunPerTable([
    run("bronze.a", "2026-11-03T02:00:00Z"),
    run("bronze.b", "2026-11-04T02:00:00Z"),
    run("bronze.a", "2026-11-04T01:00:00Z"),
  ])
  assert.deepEqual(
    rows.map((r) => [r.tableName, r.runAt]),
    [
      ["bronze.b", "2026-11-04T02:00:00Z"],
      ["bronze.a", "2026-11-04T01:00:00Z"],
    ]
  )
})

test("maintenanceTableHref links a two-part name to its table page", () => {
  assert.equal(maintenanceTableHref("bronze.g3a_orders"), "/lakehouse/tables/bronze/g3a_orders")
})

test("maintenanceTableHref gives no link for a name that does not split", () => {
  for (const name of ["orders", ".orders", "bronze."]) {
    assert.equal(maintenanceTableHref(name), null)
  }
})
