/**
 * The ingest mode a connector is saved with when the console has no stored one
 * yet, decided from the adapter alone (`SRC-44`, QA `LH-SRC-6-41`).
 *
 * The loader runs a `kafka` connector only as `stream`
 * (`dagster/dispar_orchestrate/ingest_factory.py`: "A `kafka`-adapter
 * connector's `ingest_mode` is `"stream"`, never `"batch"`"); saved as `batch`
 * every run stops with `UnknownAdapter`. A `cdc` connector is `cdc`, and every
 * other adapter is a scheduled `batch` read. The three save sites (create page,
 * edit page, ingest panel) call this one function so none keeps its own copy of
 * the rule; the API refuses the mismatched pairs too (`validate_ingest_mode`).
 */
export function ingestModeFor(adapter: string | null | undefined): "cdc" | "stream" | "batch" {
  if (adapter === "kafka") return "stream"
  if (adapter === "cdc") return "cdc"
  return "batch"
}
