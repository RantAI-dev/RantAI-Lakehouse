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

  test("ops sit at their dependency depth, each op's data right beside it", () => {
    const { nodes } = dagsterFlow(
      [op("ingest", ["Postgres x"], ["Iceberg y"]), op("register", [], ["ClickHouse z"])],
      [{ from: "ingest", to: "register" }],
      () => undefined
    )
    const col = Object.fromEntries(nodes.map((n) => [n.id, n.column]))
    expect(col["source:Postgres x"]).toBe(0)
    expect(col["op:ingest"]).toBe(1)
    expect(col["op:register"]).toBe(2)
    expect(col["sink:Iceberg y"]).toBe(2)
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
  // BLD-1 part 1 D17: a pipeline is inputs, steps and an output, so a form
  // step is one node (its operations are listed in its panel), where the
  // chart of before drew one node per transform.
  test("an input, one form step, then the target", () => {
    const { nodes, edges } = authoredFlow(
      {
        // BLD-1 part 1 / D3: the API returns both forms of the definition.
        inputs: [{ name: "events", zone: "silver", table: "events" }],
        steps: [{ name: "transformed", kind: "transform", reads: "events", transforms: ["dedupe(id)", "select(id,name)"] }],
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
    expect(nodes.map((n) => [n.id, n.column, n.label])).toEqual([
      ["input:0", 0, "silver.events"],
      ["step:0", 1, "transformed"],
      ["target", 2, "gold.events_clean"],
    ])
    expect(nodes[1].sublabel).toBe("dedupe, select")
    expect(edges.map((e) => `${e.from}>${e.to}`)).toEqual(["input:0>step:0", "step:0>target"])
  })

  test("two inputs and a SQL step: each step sits right of what it reads, edges from what it names", () => {
    const { nodes, edges } = authoredFlow(
      {
        inputs: [
          { name: "orders", zone: "bronze", table: "orders_raw" },
          { name: "customers", zone: "bronze", table: "customers" },
        ],
        steps: [
          { name: "paid", kind: "transform", reads: "orders", transforms: ["filter(status = 'paid')"] },
          {
            name: "by_country",
            kind: "sql",
            sql: "SELECT c.country, sum(p.amount) FROM paid AS p LEFT JOIN customers AS c ON p.customer_id = c.id GROUP BY 1",
          },
        ],
        sourceZone: "bronze",
        sourceTable: "orders_raw",
        incrementalColumn: null,
        transforms: [],
        fbicEnabled: false,
        targetZone: "silver",
        targetTable: "sales_by_country",
        connectorId: null,
      },
      describeTransform
    )
    expect(nodes.map((n) => [n.id, n.column])).toEqual([
      ["input:0", 0],
      ["input:1", 0],
      ["step:0", 1],
      ["step:1", 2],
      ["target", 3],
    ])
    // An input read under another name says so.
    expect(nodes[0].sublabel).toBe("read as orders")
    expect(nodes[3]).toMatchObject({ kind: "step", stepKind: "sql", sublabel: "SQL" })
    expect(edges.map((e) => `${e.from}>${e.to}`)).toEqual([
      "input:0>step:0",
      "step:0>step:1",
      "input:1>step:1",
      "step:1>target",
    ])
  })

  // BLD-1 part 1 S3 review R1-S2: a kind this console does not know is drawn
  // with its name and kind and no edge in; the output still follows it.
  test("a step of a kind this console does not know is drawn with its name and kind", () => {
    const { nodes, edges } = authoredFlow(
      {
        inputs: [{ name: "orders", zone: "bronze", table: "orders" }],
        steps: [{ name: "joined", kind: "join", left: "orders" }],
        sourceZone: "bronze",
        sourceTable: "orders",
        incrementalColumn: null,
        transforms: [],
        fbicEnabled: false,
        targetZone: "silver",
        targetTable: "joined",
        connectorId: null,
      },
      describeTransform
    )
    expect(nodes[1]).toMatchObject({ id: "step:0", label: "joined", sublabel: "join, not shown here", stepKind: "other" })
    expect(edges.map((e) => `${e.from}>${e.to}`)).toEqual(["step:0>target"])
  })

  // BLD-10 D3: a pipeline with no transform copies its source as is. The
  // target moves into column 1 and says so, so an empty middle is not a
  // puzzle for the reader.
  test("a pipeline with no transform copies its source, and the target says so", () => {
    const { nodes, edges } = authoredFlow(
      {
        inputs: [{ name: "orders_raw", zone: "bronze", table: "orders_raw" }],
        steps: [],
        sourceZone: "bronze",
        sourceTable: "orders_raw",
        incrementalColumn: null,
        transforms: [],
        fbicEnabled: false,
        targetZone: "silver",
        targetTable: "orders",
        connectorId: null,
      },
      describeTransform
    )
    // The input's node id is `input:0` since BLD-1 part 1 D17 (it was
    // `source` when a pipeline had one source); the layout is the same.
    expect(nodes.map((n) => [n.id, n.column])).toEqual([
      ["input:0", 0],
      ["target", 1],
    ])
    expect(nodes.find((n) => n.id === "target")?.sublabel).toBe("plain copy of the source")
    expect(edges.map((e) => `${e.from}>${e.to}`)).toEqual(["input:0>target"])
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
        // BLD-1 part 1 S3 review R1-S9: the `transform` node kind is gone
        // (nothing produces it since a form step is one node); the test of
        // an edge inside one column uses the kind that replaced it.
        { id: "t0", column: 1, kind: "step", label: "x" },
        { id: "t1", column: 1, kind: "step", label: "y" },
      ],
      [{ from: "t0", to: "t1" }]
    )
    const { from, to } = layout.edges[0]
    expect(edgePath(from, to)).toMatch(/^M [\d.]+ [\d.]+ L /)
  })
})
