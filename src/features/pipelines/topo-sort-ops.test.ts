import { describe, expect, it } from "bun:test"
import { topoSortOps } from "./topo-sort-ops"
import type { PipelineOpNode } from "@/services/contracts/pipelines"

function op(name: string): PipelineOpNode {
  return { name, description: null, sourceRef: null, commit: null, sql: null, reads: [], writes: [] }
}

describe("topoSortOps", () => {
  it("orders a linear chain by its edges, upstream to downstream", () => {
    const ops = [op("c"), op("a"), op("b")]
    const edges = [
      { from: "a", to: "b" },
      { from: "b", to: "c" },
    ]
    const sorted = topoSortOps(ops, edges)
    expect(sorted.map((s) => s.node.name)).toEqual(["a", "b", "c"])
    expect(sorted[2]?.upstream).toEqual(["b"])
    expect(sorted[0]?.upstream).toEqual([])
  })

  it("orders independent ops (no edges between them) by declaration order", () => {
    const ops = [op("x"), op("y")]
    const sorted = topoSortOps(ops, [])
    expect(sorted.map((s) => s.node.name)).toEqual(["x", "y"])
  })

  it("falls back to declaration order on a cycle instead of throwing", () => {
    const ops = [op("a"), op("b")]
    const edges = [
      { from: "a", to: "b" },
      { from: "b", to: "a" },
    ]
    expect(() => topoSortOps(ops, edges)).not.toThrow()
    const sorted = topoSortOps(ops, edges)
    expect(sorted.map((s) => s.node.name)).toEqual(["a", "b"])
  })

  it("renders a single op with no edges deliberately (Phase A found this is the common real case), not as a broken empty graph", () => {
    // PART C of `parts/1b-dagster-one-op-per-unit-and-retries.md`:
    // `run_bronze_maintenance` was split into `list_bronze_tables` ->
    // `maintain_bronze_table[<key>]` -> `summarize_bronze_maintenance`.
    // The test only cares that a single op with no edges is rendered
    // as a single-node graph; using `list_bronze_tables` keeps it
    // pinned to a current op name.
    const ops = [op("list_bronze_tables")]
    const sorted = topoSortOps(ops, [])
    expect(sorted).toEqual([{ node: ops[0], upstream: [] }])
  })

  it("ignores an edge referencing an op not present in the ops list, rather than crashing", () => {
    const ops = [op("a")]
    const edges = [{ from: "ghost", to: "a" }]
    const sorted = topoSortOps(ops, edges)
    expect(sorted.map((s) => s.node.name)).toEqual(["a"])
    expect(sorted[0]?.upstream).toEqual([])
  })
})
