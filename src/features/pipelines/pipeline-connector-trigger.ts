/**
 * The `dependsOn` control a pipeline made from a connector carries (BLD-7
 * decisions 4 and 5): "Run after this connector loads the table".
 *
 * Pure, like [`./pipeline-reupload-trigger`] and for the same reason -- the
 * rule is enumerated here, cheaply, instead of being spread through the
 * five-step form. This module owns exactly one upstream, `ingest_job`, and
 * touches nothing else in the stored chain.
 *
 * - The switch is offered only for a Bronze source with a connector: a
 *   connector loads its objects into Bronze tables
 *   (`ingest_factory.py`), so its load can only be the upstream of a
 *   pipeline reading one of them.
 * - A new pipeline defaults ON. The entry point is the connector page's
 *   *Create pipeline*, and the whole point of that entry is that the
 *   pipeline runs once the table it reads has been loaded.
 * - OFF removes `ingest_job` and leaves every other entry alone. ON adds
 *   it, once, and never duplicates a stored one.
 * - A pipeline WITHOUT a connector is not this control's business: its
 *   chain is returned untouched (a stored `ingest_job` there belongs to
 *   whoever put it there; `withReuploadTrigger` is the helper that drops
 *   the per-table upstreams a non-Bronze source cannot have).
 *
 * The batch sensor that acts on this upstream already exists (`BLD-4`,
 * `authored_factory._latest_record_for_ingest_table`): it counts only the
 * ingest runs that materialized THIS pipeline's Bronze table, which is why
 * one static `ingest_job` can be the upstream of many pipelines. No sensor
 * is added here, and the API's `validate_per_table_upstream` refuses
 * `ingest_job` for a non-Bronze source, so this module never has to guess
 * what the server will reject.
 */

import { BRONZE_ZONE, INGEST_JOB } from "./pipeline-reupload-trigger"

/** Whether a pipeline with a connector may chain the connector's load at all. */
function offersConnectorLoad(sourceZone: string, hasConnector: boolean): boolean {
  return hasConnector && sourceZone.trim() === BRONZE_ZONE
}

/**
 * The switch's stored state: ON when this pipeline's chain already names
 * the ingest job. An absent chain is OFF, EXCEPT for a new pipeline, which
 * defaults ON (`existing === false` is what the editor knows; a pipeline
 * that was never saved has nothing stored to read).
 */
export function connectorLoadTriggerOn(
  dependsOn: readonly string[] | undefined,
  sourceZone: string,
  hasConnector: boolean,
  isExisting: boolean
): boolean {
  if (!offersConnectorLoad(sourceZone, hasConnector)) return false
  if (!isExisting) return true
  return (dependsOn ?? []).includes(INGEST_JOB)
}

/**
 * The chain to send: `dependsOn` with `ingest_job` added when `on` and
 * removed when off, every other entry kept exactly as it was and in order.
 *
 * A connector on a non-Bronze source still drops `ingest_job`: the form
 * shows no control for it there, and the server refuses it
 * (`validate_per_table_upstream`) -- sending a chain back unchanged would
 * fail the whole save for a rule the user cannot see a switch for.
 */
export function withConnectorLoadTrigger(
  dependsOn: readonly string[] | undefined,
  sourceZone: string,
  hasConnector: boolean,
  on: boolean
): string[] {
  if (!hasConnector) return [...(dependsOn ?? [])]
  const kept = (dependsOn ?? []).filter((entry) => entry !== INGEST_JOB)
  if (!offersConnectorLoad(sourceZone, hasConnector) || !on) return kept
  return [INGEST_JOB, ...kept]
}
