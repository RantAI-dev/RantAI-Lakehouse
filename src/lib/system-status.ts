/**
 * Pure view logic for the System status page (`WEB-3` S2-D2), kept out of
 * the components so it is unit-tested with `bun test`.
 *
 * Every function here only reshapes what the API already returned. None of
 * them fills a gap: a service that was not probed is reported as not
 * probed, never as up, and a value the API did not send stays absent.
 */
import { lakehouseTableHref } from "./lakehouse-view"
import type { PlatformService } from "@/services/contracts/ops"
import type { MaintenanceRun, ReplicationSlot } from "@/services/contracts/governance"

/** The one-line answer to "is the system up?", from the services probe. */
export type OverallServiceState = {
  tone: "success" | "destructive" | "warning" | "neutral"
  /** The sentence the page shows. */
  headline: string
  /** Services the API probed and found not healthy, by name. */
  down: string[]
  /** Services the API did not probe this request (`checked: false`). */
  notProbed: string[]
}

/**
 * Summarises the probe table. Only probed services (`checked: true`) can
 * be "up" or "down": the API marks the rest `unknown` because nothing
 * probes them (`ops.rs`), and counting them as up would claim a check that
 * never ran.
 */
export function overallServiceState(services: readonly PlatformService[]): OverallServiceState {
  const probed = services.filter((s) => s.checked)
  const notProbed = services.filter((s) => !s.checked).map((s) => s.name)
  const down = probed.filter((s) => s.health !== "healthy").map((s) => s.name)

  if (probed.length === 0) {
    return {
      tone: "neutral",
      headline: "No service was probed, so whether the system is up is not known.",
      down,
      notProbed,
    }
  }
  if (down.length > 0) {
    return {
      tone: "destructive",
      headline:
        down.length === 1
          ? `${down[0]} is not healthy.`
          : `${down.length} services are not healthy: ${down.join(", ")}.`,
      down,
      notProbed,
    }
  }
  return {
    tone: notProbed.length > 0 ? "warning" : "success",
    headline:
      probed.length === 1
        ? "The one probed service is up."
        : `All ${probed.length} probed services are up.`,
    down,
    notProbed,
  }
}

/**
 * The newest reading of each replication slot.
 *
 * `GET /api/governance/replication` returns history: every 15-minute check
 * of every slot, newest first, up to 500 rows. A status view wants each
 * slot once, as it was last seen; the order of first appearance is kept,
 * so the result stays newest first.
 */
export function latestSlotReadings(slots: readonly ReplicationSlot[]): ReplicationSlot[] {
  const latest = new Map<string, ReplicationSlot>()
  for (const slot of slots) {
    const key = `${slot.connectorId}\u0000${slot.slotName}`
    const seen = latest.get(key)
    if (!seen || seen.checkedAt < slot.checkedAt) latest.set(key, slot)
  }
  return [...latest.values()].sort((a, b) => b.checkedAt.localeCompare(a.checkedAt))
}

/**
 * The newest maintenance run of each table, newest first.
 *
 * `GET /api/governance/maintenance` returns every recorded run. The status
 * page answers "what did maintenance last do to each table"; the full
 * history of one table belongs on that table's page.
 */
export function latestRunPerTable(runs: readonly MaintenanceRun[]): MaintenanceRun[] {
  const latest = new Map<string, MaintenanceRun>()
  for (const run of runs) {
    const seen = latest.get(run.tableName)
    if (!seen || seen.runAt < run.runAt) latest.set(run.tableName, run)
  }
  return [...latest.values()].sort((a, b) => b.runAt.localeCompare(a.runAt))
}

/**
 * The table page a maintenance run links to, or `null` when the name is
 * not the two-part `namespace.table` the maintenance job records
 * (`maintenance.py`: "the catalog's own two-part name, e.g.
 * `bronze.g3a_orders`"). A name that does not split is shown without a
 * link rather than linked to a guess.
 */
export function maintenanceTableHref(tableName: string): string | null {
  const dot = tableName.indexOf(".")
  if (dot <= 0 || dot === tableName.length - 1) return null
  return lakehouseTableHref(tableName.slice(0, dot), tableName.slice(dot + 1))
}
