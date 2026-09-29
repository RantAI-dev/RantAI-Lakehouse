import { describe, expect, test } from "bun:test"
import { describeTransform } from "@/lib/transform-draft"
import type { PipelineOpNode } from "@/services/contracts/pipelines"
import { authoredFlow, dagsterFlow, edgePath, layoutFlow } from "./pipeline-flow-layout"

function op(name: string, reads: string[] = [], writes: string[] = []): PipelineOpNode {
  return { name, description: null, sourceRef: `dispar_orchestrate/x.py::${name}`, commit: "abc", sql: null, reads, writes }
}

describe("dagsterFlow", () => {
  test("a one-op job reads on the left, writes on the right", () => {
    const { nodes, edges } = dagsterFlow([op("run", ["ClickHouse a"], ["Iceberg b"])], [], () => "completed")
    expect(nodes.map((n) => [n.id, n.column])).toEqual([
      ["op:run", 1],
      ["source:ClickHouse a", 0],
      ["sink:Iceberg b", 2],
    ])
    expect(edges).toEqual([
      { from: "source:ClickHouse a", to: "op:run" },
      { from: "op:run", to: "sink:Iceberg b" },
    ])
    expect(nodes[0].status).toBe("completed")
  })

  test("ops sit at their dependency depth and writes come after the deepest op", () => {
    const { nodes } = dagsterFlow(
      [op("ingest", ["Postgres x"], ["Iceberg y"]), op("register", [], ["ClickHouse z"])],
      [{ from: "ingest", to: "register" }],
      () => undefined
    )
    const col = Object.fromEntries(nodes.map((n) => [n.id, n.column]))
    expect(col["op:ingest"]).toBe(1)
    expect(col["op:register"]).toBe(2)
    expect(col["sink:Iceberg y"]).toBe(3)
    expect(col["sink:ClickHouse z"]).toBe(3)
  })

  test("with nothing declared as read, ops start in the first column", () => {
    const { nodes } = dagsterFlow([op("run", [], ["POST /x"])], [], () => undefined)
    expect(nodes.find((n) => n.id === "op:run")?.column).toBe(0)
  })

  test("a phrase two ops declare is one node", () => {
    const { nodes, edges } = dagsterFlow([op("a", ["T"]), op("b", ["T"])], [], () => undefined)
    expect(nodes.filter((n) => n.id === "source:T")).toHaveLength(1)
    expect(edges.filter((e) => e.from === "source:T")).toHaveLength(2)
  })
})

describe("authoredFlow", () => {
  test("source, the transforms stacked in one column, then target", () => {
    const { nodes, edges } = authoredFlow(
      {
        sourceZone: "silver",
        sourceTable: "events",
        incrementalColumn: null,
        transforms: ["dedupe(id)", "select(id,name)"],
        fbicEnabled: false,
        targetZone: "gold",
        targetTable: "events_clean",
        connectorId: null,
      },
      describeTransform
    )
    expect(nodes.map((n) => [n.id, n.column])).toEqual([
      ["source", 0],
      ["t0", 1],
      ["t1", 1],
      ["target", 2],
    ])
    expect(edges.map((e) => `${e.from}>${e.to}`)).toEqual(["source>t0", "t0>t1", "t1>target"])
  })
})

describe("layoutFlow", () => {
  test("columns go left to right and a column is centred on the tallest", () => {
    const layout = layoutFlow(
      [
        { id: "a", column: 0, kind: "source", label: "a" },
        { id: "b", column: 1, kind: "op", label: "b" },
        { id: "c", column: 1, kind: "op", label: "c" },
      ],
      [{ from: "a", to: "b" }]
    )
    const [a, b, c] = layout.nodes
    expect(b.x).toBeGreaterThan(a.x + a.w)
    expect(c.y).toBeGreaterThan(b.y + b.h)
    expect(a.y + a.h / 2).toBeCloseTo((b.y + c.y + c.h) / 2, 5)
    expect(layout.edges).toHaveLength(1)
  })

  test("an edge to a node that is not drawn is dropped, not guessed", () => {
    const layout = layoutFlow([{ id: "a", column: 0, kind: "source", label: "a" }], [{ from: "a", to: "zz" }])
    expect(layout.edges).toHaveLength(0)
  })

  test("an edge inside one column drops straight down", () => {
    const layout = layoutFlow(
      [
        { id: "t0", column: 1, kind: "transform", label: "x" },
        { id: "t1", column: 1, kind: "transform", label: "y" },
      ],
      [{ from: "t0", to: "t1" }]
    )
    const { from, to } = layout.edges[0]
    expect(edgePath(from, to)).toMatch(/^M [\d.]+ [\d.]+ L /)
  })
})
