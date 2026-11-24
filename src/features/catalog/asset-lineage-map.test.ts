// The Lineage tab draws the asset as a mind map: itself in the middle,
// what it is built from to the left, what is built from it to the right,
// and — dashed — what reads it and what the catalog ties to it.
import { describe, expect, it } from "bun:test"
import { chartPlacement, groupCharts, mindmapLayout, mindmapModel, shortLabel } from "./asset-lineage-map"

const GRAPH = {
  focus: "bronze.orders",
  nodes: [
    { id: "connector:pg", label: "Postgres", kind: "connector" },
    { id: "bronze:orders", label: "bronze.orders", kind: "focus" },
    { id: "table:silver.orders", label: "silver.orders", kind: "silver" },
    { id: "table:serving.mart_orders", label: "serving.mart_orders", kind: "gold" },
  ],
  edges: [
    { id: "e0", from: "connector:pg", to: "bronze:orders", kind: "ingest", evidence: "ingest spec (sql adapter)" },
    { id: "e1", from: "bronze:orders", to: "table:silver.orders", kind: "pipeline", evidence: "authored pipeline clean (pl-clean)" },
    { id: "e2", from: "table:silver.orders", to: "table:serving.mart_orders", kind: "view", evidence: "view definition" },
  ],
  columnMappings: [],
  supported: true,
}
const SELF = { label: "bronze.orders", kind: "bronze" }
const NONE = { dependents: [], relatedUpstream: [], relatedDownstream: [] }

describe("mindmapModel", () => {
  it("puts the asset in the middle and every node as many columns away as its longest path", () => {
    const { nodes, edges } = mindmapModel({ graph: GRAPH, self: SELF, ...NONE })
    const level = Object.fromEntries(nodes.map((n) => [n.id, n.level]))
    expect(level).toEqual({
      "connector:pg": -1,
      "bronze:orders": 0,
      "pipeline:pl-clean": 1,
      "table:silver.orders": 2,
      "table:serving.mart_orders": 3,
    })
    expect(nodes.filter((n) => n.centre).map((n) => n.id)).toEqual(["bronze:orders"])
    // The pipeline is a stop of its own, with a page to open.
    const pipeline = nodes.find((n) => n.id === "pipeline:pl-clean")
    expect(pipeline).toMatchObject({ caption: "Pipeline", label: "clean", href: "/pipelines/pl-clean" })
    expect(edges.map((e) => `${e.from}>${e.to}`)).toEqual([
      "connector:pg>bronze:orders",
      "bronze:orders>pipeline:pl-clean",
      "pipeline:pl-clean>table:silver.orders",
      "table:silver.orders>table:serving.mart_orders",
    ])
    expect(edges.every((e) => e.recorded)).toBe(true)
  })

  it("branches into what reads the asset and what the catalog ties to it, as unrecorded", () => {
    const { nodes, edges } = mindmapModel({
      graph: GRAPH,
      self: SELF,
      dependents: [{ id: "b_1", name: "Sales board", kind: "dashboard", href: "/dashboards?board=b_1" }],
      relatedUpstream: [{ id: "orders-raw", name: "bronze.orders_raw", kind: "table", href: "/data/assets/orders-raw" }],
      relatedDownstream: [],
    })
    const board = nodes.find((n) => n.label === "Sales board")
    expect(board).toMatchObject({ level: 1, caption: "Dashboard", recorded: false })
    expect(nodes.find((n) => n.label === "bronze.orders_raw")).toMatchObject({ level: -1, recorded: false })
    // Each hangs off the asset: read from it, or leading into it.
    expect(edges.filter((e) => !e.recorded).map((e) => [e.from, e.to, e.label])).toEqual([
      ["bronze:orders", "down:b_1", "reads"],
      ["up:orders-raw", "bronze:orders", "related"],
    ])
  })

  it("still has a centre when no lineage is recorded, for the branches to hang from", () => {
    const empty = { ...GRAPH, nodes: [], edges: [] }
    const { nodes } = mindmapModel({
      graph: empty,
      self: { label: "serving.mart_orders", kind: "gold" },
      dependents: [{ id: "default", name: "Default dashboard", kind: "dashboard", href: "/dashboards?board=default" }],
      relatedUpstream: [],
      relatedDownstream: [],
    })
    expect(nodes.map((n) => [n.label, n.level, n.centre])).toEqual([
      ["serving.mart_orders", 0, true],
      ["Default dashboard", 1, false],
    ])
    expect(nodes[0].caption).toBe("Gold")
  })

  it("draws one way out of a pipeline that reads two tables", () => {
    const two = {
      ...GRAPH,
      nodes: [
        { id: "bronze:a", label: "bronze.a", kind: "bronze" },
        { id: "bronze:b", label: "bronze.b", kind: "bronze" },
        { id: "table:silver.joined", label: "silver.joined", kind: "focus" },
      ],
      edges: [
        { id: "e0", from: "bronze:a", to: "table:silver.joined", kind: "pipeline", evidence: "authored pipeline join (pl-join)" },
        { id: "e1", from: "bronze:b", to: "table:silver.joined", kind: "pipeline", evidence: "authored pipeline join (pl-join)" },
      ],
    }
    const { nodes, edges } = mindmapModel({ graph: two, self: SELF, ...NONE })
    expect(nodes.filter((n) => n.caption === "Pipeline")).toHaveLength(1)
    expect(edges.map((e) => `${e.from}>${e.to}`)).toEqual([
      "bronze:a>pipeline:pl-join",
      "pipeline:pl-join>table:silver.joined",
      "bronze:b>pipeline:pl-join",
    ])
    expect(Object.fromEntries(nodes.map((n) => [n.id, n.level]))).toMatchObject({
      "bronze:a": -2,
      "bronze:b": -2,
      "pipeline:pl-join": -1,
      "table:silver.joined": 0,
    })
  })
})

describe("mindmapLayout", () => {
  it("lays the columns out left to right and centres a short column on a tall one", () => {
    const model = mindmapModel({
      graph: GRAPH,
      self: SELF,
      dependents: [
        { id: "b_1", name: "Sales board", kind: "dashboard", href: null },
        { id: "sq-1", name: "Orders by day", kind: "saved query", href: null },
      ],
      relatedUpstream: [],
      relatedDownstream: [],
    })
    const { nodes, width, height } = mindmapLayout(model)
    const x = (id: string) => nodes.find((n) => n.id === id)!.x
    const y = (id: string) => nodes.find((n) => n.id === id)!.y
    expect(x("connector:pg")).toBeLessThan(x("bronze:orders"))
    expect(x("bronze:orders")).toBeLessThan(x("pipeline:pl-clean"))
    expect(x("pipeline:pl-clean")).toBe(x("down:b_1"))
    expect(x("pipeline:pl-clean")).toBeLessThan(x("table:silver.orders"))
    // Three nodes share the column right of the asset; the asset sits level
    // with the middle of them.
    const column = nodes.filter((n) => n.level === 1).map((n) => n.y)
    expect(new Set(column).size).toBe(3)
    expect(y("bronze:orders")).toBe((Math.min(...column) + Math.max(...column)) / 2)
    expect(width).toBeGreaterThan(0)
    expect(height).toBeGreaterThan(0)
    // Nothing is drawn outside the canvas.
    expect(nodes.every((n) => n.x >= 0 && n.y >= 0 && n.y <= height)).toBe(true)
  })
})

describe("shortLabel", () => {
  it("drops the schema the caption already states, so marts of one prefix stay apart", () => {
    expect(shortLabel({ caption: "Gold", label: "serving.mart_material_by_group" })).toBe("mart_material_by_group")
    expect(shortLabel({ caption: "Silver", label: "silver.sap_material_master" })).toBe("sap_material_master")
    expect(shortLabel({ caption: "Bronze", label: "bronze.orders" })).toBe("orders")
    // Nothing to drop: the caption says what it is, not where it lives.
    expect(shortLabel({ caption: "Connector", label: "northwind" })).toBe("northwind")
    expect(shortLabel({ caption: "Dashboard", label: "serving.board" })).toBe("serving.board")
  })
})

// BI-71 decision 2: serving.mart_wisman is read by 111 charts, which drew a
// column of 111 nodes. They become one node per dashboard.
describe("groupCharts", () => {
  const TABLE = { id: "table:serving.mart_wisman", label: "serving.mart_wisman", kind: "focus" }
  const METRIC = { id: "metric:visitors", label: "visitors", kind: "metric" }
  // 50 on "QA Auto", 40 on "Arrivals", 20 on "Busy" (one private board the
  // viewer cannot read: no title), 1 not on any listed board.
  const boardOfIndex = (i: number) => (i < 50 ? "b_qa" : i < 90 ? "b_arr" : i < 110 ? "b_private" : "b_gone")
  const charts = Array.from({ length: 111 }, (_, i) => ({ id: `chart:c${i}`, label: `Chart ${i}`, kind: "chart" }))
  const graph = {
    ...GRAPH,
    focus: "serving.mart_wisman",
    nodes: [TABLE, METRIC, ...charts],
    // Most charts read the table; the first five read it through the metric.
    edges: [
      { id: "m", from: TABLE.id, to: METRIC.id, kind: "metric" },
      ...charts.map((c, i) => ({ id: `e${i}`, from: i < 5 ? METRIC.id : TABLE.id, to: c.id, kind: "chart" })),
    ],
  }
  const model = mindmapModel({ graph, self: { label: "serving.mart_wisman", kind: "gold" }, ...NONE })
  const placement = chartPlacement(
    charts.map((c, i) => ({ id: c.id.slice("chart:".length), board: boardOfIndex(i), kind: "bar" })),
    [
      { id: "b_qa", name: "QA Auto" },
      { id: "b_arr", name: "Arrivals" },
    ]
  )

  it("makes 111 charts on three dashboards into one node per readable dashboard and one Other group", () => {
    const { nodes, edges } = groupCharts(model, placement, null)
    expect(nodes.some((n) => n.id.startsWith("chart:"))).toBe(false)
    const groups = nodes.filter((n) => n.group)
    // Largest first; the private board's charts are not named, they are "Other".
    expect(groups.map((n) => n.label)).toEqual(["QA Auto · 50 charts", "Arrivals · 40 charts", "Other charts · 21 charts"])
    expect(groups.map((n) => n.caption)).toEqual(["Dashboard", "Dashboard", "Charts"])
    expect(nodes.some((n) => n.label.includes("b_private"))).toBe(false)
    // What led into the charts leads into their group, once per source.
    expect(edges.filter((e) => e.to === "group:b_qa").map((e) => e.from).sort()).toEqual([METRIC.id, TABLE.id])
    expect(edges.filter((e) => e.to === "group:b_arr").map((e) => e.from)).toEqual([TABLE.id])
    // Nothing else is grouped.
    expect(nodes.find((n) => n.id === METRIC.id)).toBeTruthy()
    expect(nodes).toHaveLength(2 + 3)
  })

  it("orders groups of one size by title", () => {
    const even = chartPlacement(
      [
        { id: "x", board: "b_z", kind: "bar" },
        { id: "y", board: "b_a", kind: "bar" },
      ],
      [
        { id: "b_z", name: "Zebra" },
        { id: "b_a", name: "Apple" },
      ]
    )
    const two = mindmapModel({
      graph: { ...graph, nodes: [TABLE, { id: "chart:x", label: "X", kind: "chart" }, { id: "chart:y", label: "Y", kind: "chart" }], edges: [] },
      self: { label: "serving.mart_wisman", kind: "gold" },
      ...NONE,
    })
    expect(groupCharts(two, even, null).nodes.filter((n) => n.group).map((n) => n.label)).toEqual([
      "Apple · 1 chart",
      "Zebra · 1 chart",
    ])
  })

  it("lists an open group's charts as their own nodes, one column right of the group", () => {
    const { nodes, edges } = groupCharts(model, placement, "b_arr")
    const group = nodes.find((n) => n.id === "group:b_arr")!
    expect(group.group).toMatchObject({ open: true, count: 40 })
    const listed = nodes.filter((n) => n.id.startsWith("chart:"))
    expect(listed).toHaveLength(40)
    expect(listed.every((n) => n.level === group.level + 1)).toBe(true)
    expect(edges.filter((e) => e.from === "group:b_arr")).toHaveLength(40)
    // The other groups stay shut.
    expect(nodes.filter((n) => n.group?.open)).toHaveLength(1)
    // And the laid-out map puts the opened charts beside their group.
    const laid = mindmapLayout({ nodes, edges })
    const x = (id: string) => laid.nodes.find((n) => n.id === id)!.x
    expect(x("chart:c50")).toBeGreaterThan(x("group:b_arr"))
  })

  it("leaves the map as it came without a placement (the read failed) or without charts", () => {
    expect(groupCharts(model, null, null)).toBe(model)
    const plain = mindmapModel({ graph: GRAPH, self: SELF, ...NONE })
    expect(groupCharts(plain, placement, null)).toBe(plain)
  })
})
