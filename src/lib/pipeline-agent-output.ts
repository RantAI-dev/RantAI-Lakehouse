import type { PipelineRunResult } from "@/types/pipeline"

/**
 * Builds a fake "successful run" payload for a freshly generated agentic pipeline.
 *
 * Produces a deterministic-ish output table name from the pipeline name (lowercased,
 * non-alphanumeric chars stripped, capped at 28 chars) and a current-time run ID,
 * so the pipeline detail page can render a realistic preview right after generation.
 */
export function buildAgentPipelineSuccessOutput(pipelineName: string): PipelineRunResult {
  const safe = pipelineName
    .toLowerCase()
    .replace(/[^a-z0-9_]/g, "_")
    .replace(/_+/g, "_")
    .slice(0, 28)
  const stamp = new Date().toISOString().slice(0, 19).replace("T", " ")
  return {
    runId: "run-agent-" + Date.now().toString(36).slice(-10),
    completedAt: stamp,
    outputTable: "silver." + (safe || "generated") + "_draft",
    rowsWritten: "12,440",
    duration: "2m 06s",
    preview: {
      columns: ["_batch_id", "row_hash", "status", "loaded_at"],
      rows: [
        ["b-901", "a1f2…c9", "committed", stamp],
        ["b-901", "d3e4…b1", "committed", stamp],
        ["b-901", "7aa0…22", "committed", stamp],
      ],
    },
  }
}
