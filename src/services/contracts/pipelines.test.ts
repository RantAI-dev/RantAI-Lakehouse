import { describe, expect, it } from "bun:test"
import type {
  PipelineDetail,
  PipelineOpNode,
  PipelineOpEdge,
  PipelineSource,
  PipelineRunStep,
  PipelineRunLogsPage,
} from "@/services/contracts/pipelines"

describe("WS4 pipelines contracts", () => {
  it("PipelineDetail.graph is null when Dagster could not resolve the job graph, not an empty stand-in array", () => {
    const fixture: PipelineDetail = {
      id: "pl-1",
      name: "authored pipeline",
      kind: "batch",
      status: "draft",
      owner: "system",
      source: "silver.raw",
      target: "serving.out",
      schedule: "manual",
      lastRunAt: null,
      slaOk: null,
      freshnessLagSeconds: null,
      engine: "authored",
      description: null,
      graph: null,
      config: [],
      definition: {
        sourceZone: "silver",
        sourceTable: "raw",
        incrementalColumn: null,
        transforms: ["dedupe(id)"],
        fbicEnabled: false,
        targetZone: "serving",
        targetTable: "out",
        connectorId: null,
      },
      runs: [],
    }
    expect(fixture.graph).toBeNull()
    expect(fixture.definition?.transforms).toEqual(["dedupe(id)"])
  })

  it("PipelineDetail.graph carries real ops+edges for a Dagster job, each op nullable field-by-field", () => {
    const ops: PipelineOpNode[] = [
      { name: "ingest_bronze_table", description: "ingest", sourceRef: "dispar_orchestrate/assets.py::ingest_bronze_table", commit: "abc123", sql: null, reads: [], writes: [] },
      { name: "register_in_catalog", description: null, sourceRef: null, commit: null, sql: null, reads: [], writes: [] },
    ]
    const edges: PipelineOpEdge[] = [{ from: "ingest_bronze_table", to: "register_in_catalog" }]
    const fixture: PipelineDetail = {
      id: "bronze_maintenance_job",
      name: "bronze_maintenance_job",
      kind: "batch",
      status: "ready",
      owner: "system",
      source: null,
      target: null,
      schedule: "manual",
      lastRunAt: null,
      slaOk: null,
      freshnessLagSeconds: null,
      engine: "dagster",
      description: null,
      graph: { ops, edges },
      config: [],
      definition: null,
      runs: [],
    }
    expect(fixture.graph?.ops).toHaveLength(2)
    expect(fixture.graph?.edges).toEqual([{ from: "ingest_bronze_table", to: "register_in_catalog" }])
    // A single-op job with no edges is a real, deliberate shape (Phase A
    // finding) — never treated as broken.
    expect(fixture.graph?.ops[1]?.sourceRef).toBeNull()
  })

  it("PipelineSource carries the resolved op text plus the verified commit, never fabricated", () => {
    const fixture: PipelineSource = {
      sourceRef: "dispar_orchestrate/assets.py::ingest_bronze_table",
      commit: "abc123",
      language: "python",
      text: "def ingest_bronze_table(...):\n    ...\n",
    }
    expect(fixture.language).toBe("python")
  })

  it("PipelineRunStep.materializations[].rows is Measured — null means not measured, not zero", () => {
    const fixture: PipelineRunStep = {
      stepKey: "ingest_bronze_table",
      status: "completed",
      startMs: 1000,
      endMs: 2000,
      materializations: [
        { assetKey: "bronze.table", rows: 1234 },
        { assetKey: null, rows: null },
      ],
    }
    expect(fixture.materializations[1]?.rows).toBeNull()
    expect(fixture.materializations[0]?.rows).toBe(1234)
  })

  it("PipelineRunLogsPage carries the backend's own opaque cursor to pass through unmodified on the next poll", () => {
    const fixture: PipelineRunLogsPage = {
      lines: [{ ts: 1700000000, level: "INFO", stepKey: "ingest_bronze_table", message: "started" }],
      cursor: "opaque-cursor-1",
    }
    expect(fixture.cursor).toBe("opaque-cursor-1")
  })

  it("Pipeline carries optional targetLag and lag (BLD-2 D8)", () => {
    const fixture: PipelineDetail = {
      id: "pl-lag",
      name: "lag pipeline",
      kind: "batch",
      status: "ready",
      owner: "system",
      source: "bronze.orders",
      target: "silver.orders_clean",
      schedule: "manual",
      lastRunAt: null,
      slaOk: null,
      freshnessLagSeconds: null,
      engine: "authored",
      description: null,
      graph: null,
      config: [],
      definition: null,
      runs: [],
      targetLag: { kind: "lag", minutes: 10 },
      lag: {
        status: "met",
        lagSeconds: 45,
        lastCheckAt: "2026-10-15T12:00:00Z",
        reason: null,
      },
    }
    expect(fixture.targetLag).toEqual({ kind: "lag", minutes: 10 })
    expect(fixture.lag?.status).toBe("met")
    expect(fixture.lag?.lagSeconds).toBe(45)
  })
})
