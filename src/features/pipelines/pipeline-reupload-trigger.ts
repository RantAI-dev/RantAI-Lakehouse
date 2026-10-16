/**
 * The one `dependsOn` control the console offers (DATA-19 decision 2,
 * plan D3): "Run again after this table is re-uploaded".
 *
 * Pure, so the rules are unit-tested without rendering the five-step
 * form. The form holds two pieces of state this module owns the meaning
 * of: the stored chain (`PipelineDetail.dependsOn`, which may hold
 * upstreams this switch knows nothing about) and the one boolean the
 * user sees. Every rule lives here, in one place, rather than inline in
 * the component where ON/OFF would be easy to get half-right:
 *
 * - The switch is OFF for a source that is not `bronze`. An uploaded
 *   file only ever lands in a Bronze table, so a `silver`/`gold` source
 *   has no table to be re-uploaded — and the server refuses that
 *   upstream outright (`routes::authored_pipelines::
 *   validate_per_table_upstream`), so sending it would be a 400.
 * - A new pipeline defaults ON: the feature's whole point is that a
 *   re-upload runs the pipeline, and a user building a pipeline over an
 *   uploaded table almost always wants it.
 * - Toggling changes ONLY this upstream. An `authored__`-free chain of
 *   other upstreams (another pipeline, `ingest_job` on bronze) is carried through
 *   untouched, and never duplicated. On a non-Bronze source, per-table upstreams
 *   (`file_ingest_job` and `ingest_job`) are dropped on save (BLD-4 review R1)
 *   because the server refuses them and the form offers no control for them.
 *
 * The one upstream the switch owns is named by the DAGSTER side's own
 * constant (`file_ingest.JOB_NAME`); this literal and the API's
 * `routes::uploads::FILE_INGEST_JOB` and the sensor's
 * `authored_factory.FILE_INGEST_JOB` are the same string, which is why
 * the API validates every entry of `dependsOn` against the live Dagster
 * job list.
 */

/** The Dagster job one uploaded file's load runs. */
export const FILE_INGEST_JOB = "file_ingest_job"

/** The Dagster job one connector's load runs (BLD-4 review R1). */
export const INGEST_JOB = "ingest_job"

/** The only zone an uploaded file can be re-uploaded into. */
export const BRONZE_ZONE = "bronze"

/**
 * Whether the switch is offered at all: only a Bronze source can be
 * re-uploaded (`file_ingest.py`'s load writes `bronze.<table>`).
 */
export function showsReuploadSwitch(sourceZone: string): boolean {
  return sourceZone.trim() === BRONZE_ZONE
}

/**
 * The switch's stored state: ON when this pipeline's chain already names
 * the upload job. An absent chain is OFF, EXCEPT for a new pipeline,
 * which defaults ON (`existing === false` is what the editor knows; a
 * pipeline that was never saved has nothing stored to read).
 */
export function reuploadTriggerOn(
  dependsOn: readonly string[] | undefined,
  sourceZone: string,
  isExisting: boolean
): boolean {
  if (!showsReuploadSwitch(sourceZone)) return false
  if (!isExisting) return true
  return (dependsOn ?? []).includes(FILE_INGEST_JOB)
}

/**
 * The chain to send: `dependsOn` with the upload job added when `on` and
 * removed when off, and every other entry kept exactly as it was, in
 * order, without duplicates.
 *
 * For a source that is not Bronze the upload job is always removed: the
 * switch is not shown there, and a stored chain that names it is one the
 * server refuses (`validate_per_table_upstream`) — sending it back
 * unchanged would make the whole save fail for a rule the user cannot
 * see a control for.
 */
export function withReuploadTrigger(
  dependsOn: readonly string[] | undefined,
  sourceZone: string,
  on: boolean
): string[] {
  const kept = (dependsOn ?? []).filter((entry) => entry !== FILE_INGEST_JOB)
  if (!showsReuploadSwitch(sourceZone)) {
    // BLD-4 review R1 / Decision R1: drop ingest_job on non-Bronze sources
    return kept.filter((entry) => entry !== INGEST_JOB)
  }
  if (!on) return kept
  return [FILE_INGEST_JOB, ...kept]
}
