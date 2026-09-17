import { describe, expect, it } from "bun:test"
import { topoSortOps } from "./topo-sort-ops"
import type { PipelineOpNode } from "@/services/contracts/pipelines"

function op(name: string): PipelineOpNode {
  return { name, description: null, sourceRef: null, commit: null, sql: null }
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
    const ops = [op("run_bronze_maintenance")]
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
