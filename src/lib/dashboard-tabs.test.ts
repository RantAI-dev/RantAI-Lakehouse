import { strict as assert } from "node:assert"
import { test } from "node:test"
import { readFileSync } from "node:fs"
import { join } from "node:path"
import type { BoardTab, FilterDef, LayoutMap } from "@/services/clients/bi-store"
import {
  addTab, appliesOnTab, bottomOfTab, collapseLayout, defaultTileHeight, deleteTab, filtersOnTab, foldedTiles, isStaticKind, moveTab, moveTileToTab, placeUnboxed,
  phoneOrder, readingOrder, renameTab, selectedTab, tabOf, tilesOnTab, withTabParam, withTies, movePhoneTile, resizePhoneTile,
} from "./dashboard-tabs"
import { decodeFilters, encodeFilters, normalizeFilters } from "./dashboard-filter-state"

const A: BoardTab = { id: "t_a", title: "Overview" }
const B: BoardTab = { id: "t_b", title: "Maps" }
const f = (column: string, tabs?: string[]): FilterDef => ({ column, values: ["x"], ...(tabs ? { tabs } : {}) })

test("a tile with no tab sits on the first tab and a board without tabs has none", () => {
  const layout: LayoutMap = { a: { x: 0, y: 0, w: 6, h: 4 }, b: { x: 0, y: 0, w: 6, h: 4, tab: "t_b" }, s: { x: 0, y: 0, w: 6, h: 4, tab: "gone" } }
  assert.equal(tabOf([A, B], layout, "a"), "t_a")
  assert.equal(tabOf([A, B], layout, "b"), "t_b")
  assert.equal(tabOf([A, B], layout, "s"), "t_a")
  assert.equal(tabOf([A, B], layout, "unplaced"), "t_a")
  assert.equal(tabOf([], layout, "b"), undefined)
})

test("a filter tied to a tab is given to that tab only, and ties on a board without tabs are ignored", () => {
  assert.equal(appliesOnTab(f("c"), "t_a"), true)
  assert.equal(appliesOnTab(f("c", []), "t_a"), true)
  assert.equal(appliesOnTab(f("c", ["t_a"]), "t_b"), false)
  assert.equal(appliesOnTab(f("c", ["t_a"]), undefined), true)
  const filters = [f("u"), f("a", ["t_a"]), f("b", ["t_b"])]
  assert.deepEqual(filtersOnTab(filters, "t_b").map((x) => x.column), ["u", "b"])
  const untied = [f("u")]
  assert.equal(filtersOnTab(untied, "t_b"), untied)
})

test("adding the first tab puts every existing tile in it, and later tabs start empty", () => {
  const layout: LayoutMap = { a: { x: 0, y: 0, w: 6, h: 4 }, b: { x: 6, y: 0, w: 6, h: 4 } }
  const first = addTab({ tabs: [], layout, filters: [] }, "Overview", "t_a")!
  assert.deepEqual(first.tabs, [{ id: "t_a", title: "Overview" }])
  assert.equal(first.layout.a.tab, "t_a")
  assert.equal(first.layout.b.tab, "t_a")
  const second = addTab(first, "  Maps  ", "t_b")!
  assert.equal(second.tabs[1].title, "Maps")
  assert.deepEqual(second.layout, first.layout)
  const full = { tabs: Array.from({ length: 20 }, (_, i) => ({ id: `t${i}`, title: "x" })), layout: {}, filters: [] }
  assert.equal(addTab(full, "one more"), null)
})

test("renaming trims, refuses a blank title, and moving a tab swaps it with its neighbour", () => {
  assert.equal(renameTab([A, B], "t_a", "  New  ")[0].title, "New")
  assert.deepEqual(renameTab([A, B], "t_a", "   "), [A, B])
  assert.deepEqual(moveTab([A, B], "t_b", -1).map((t) => t.id), ["t_b", "t_a"])
  assert.deepEqual(moveTab([A, B], "t_a", -1), [A, B])
})

test("a tab with tiles is deleted only when they move to the first tab, below what is there", () => {
  const layout: LayoutMap = {
    a: { x: 0, y: 0, w: 12, h: 5, tab: "t_a" },
    b: { x: 0, y: 0, w: 6, h: 3, tab: "t_b" },
    c: { x: 6, y: 0, w: 6, h: 4, tab: "t_b" },
  }
  const ids = ["a", "b", "c"]
  const edit = { tabs: [A, B], layout, filters: [f("only_b", ["t_b"]), f("both", ["t_a", "t_b"]), f("free")] }
  assert.ok("error" in (deleteTab(edit, ids, "t_b", false) as object))
  const done = deleteTab(edit, ids, "t_b", true) as typeof edit
  assert.deepEqual(done.tabs, [A])
  assert.deepEqual([done.layout.b, done.layout.c].map((t) => [t.tab, t.x, t.y]), [["t_a", 0, 5], ["t_a", 0, 8]])
  // The tie that named only the deleted tab follows the tiles; the other loses one id; untied stays untied.
  assert.deepEqual(done.filters.map((x) => x.tabs), [["t_a"], ["t_a"], undefined])
  // An empty tab goes without moving anything.
  const empty = deleteTab({ ...edit, layout: { a: layout.a } }, ["a"], "t_b", false) as typeof edit
  assert.deepEqual(empty.tabs, [A])
})

test("deleting the last tab leaves no tabs, no tile tabs and no ties", () => {
  const edit = { tabs: [A], layout: { a: { x: 0, y: 0, w: 6, h: 4, tab: "t_a" } }, filters: [f("c", ["t_a"])] }
  const out = deleteTab(edit, ["a"], "t_a", true) as typeof edit
  assert.deepEqual(out.tabs, [])
  assert.deepEqual(out.layout.a, { x: 0, y: 0, w: 6, h: 4 })
  assert.equal(out.filters[0].tabs, undefined)
})

test("a tile moves to another tab below what is there", () => {
  const layout: LayoutMap = { a: { x: 0, y: 0, w: 6, h: 4, tab: "t_a" }, b: { x: 3, y: 2, w: 6, h: 4, tab: "t_b" } }
  const out = moveTileToTab([A, B], layout, ["a", "b"], "a", "t_b")
  assert.deepEqual(out.a, { x: 0, y: 6, w: 6, h: 4, tab: "t_b" })
  assert.deepEqual(tilesOnTab([A, B], out, ["a", "b"], "t_a"), [])
})

test("moving a tile to another tab closes the space it leaves, and only that (BI-18 part C review fix R4)", () => {
  const layout: LayoutMap = {
    map: { x: 0, y: 0, w: 12, h: 6, tab: "t_a" },
    kpi: { x: 0, y: 6, w: 6, h: 4, tab: "t_a" },
    bar: { x: 6, y: 6, w: 6, h: 4, tab: "t_a" },
    low: { x: 0, y: 10, w: 6, h: 4, tab: "t_a" },
    gap: { x: 6, y: 20, w: 6, h: 4, tab: "t_a" },
  }
  const ids = Object.keys(layout)
  const out = moveTileToTab([A, B], layout, ids, "map", "t_b")
  assert.equal(out.kpi.y, 0)
  assert.equal(out.bar.y, 0)
  assert.equal(out.low.y, 4, "it stays under the tile that moved up")
  assert.equal(out.gap.y, 4, "a tile under the moved one closes up as far as the tiles above it allow")
  // A gap the user made by hand, with nothing moved out from above it, is not touched.
  const kept = moveTileToTab([A, B], { ...layout, gap: { x: 0, y: 30, w: 6, h: 4, tab: "t_a" } }, ids, "bar", "t_b")
  assert.equal(kept.gap.y, 30)
  // Moving onto the tab it is on closes nothing.
  assert.deepEqual(moveTileToTab([A, B], layout, ids, "kpi", "t_a").low, layout.low)
})

test("a tie to every tab, or to none, is no tie", () => {
  assert.equal(withTies(f("c", ["t_a"]), [A, B], ["t_a", "t_b"]).tabs, undefined)
  assert.equal(withTies(f("c", ["t_a"]), [A, B], []).tabs, undefined)
  assert.deepEqual(withTies(f("c"), [A, B], ["t_b", "gone"]).tabs, ["t_b"])
})

test("ties survive the address: they are kept by normalise, encode and decode", () => {
  const tied = [f("c", ["t_b", "t_a"])]
  assert.deepEqual(normalizeFilters(tied)[0].tabs, ["t_a", "t_b"])
  assert.deepEqual(decodeFilters(encodeFilters(tied))?.[0].tabs, ["t_a", "t_b"])
  // A filter with no tie encodes exactly as before.
  assert.equal(encodeFilters([f("c")]), '[{"column":"c","values":["x"]}]')
})

test("the selected tab is the one in the address when it exists, else the first", () => {
  assert.equal(selectedTab([A, B], "t_b"), "t_b")
  assert.equal(selectedTab([A, B], "nope"), "t_a")
  assert.equal(selectedTab([A, B], null), "t_a")
  assert.equal(selectedTab([], "t_b"), undefined)
  assert.equal(withTabParam("f=1", [A, B], "t_b"), "f=1&tab=t_b")
  assert.equal(withTabParam("f=1&tab=t_b", [A, B], "t_a"), "f=1")
})

test("static kinds are the text tile and the three section kinds", () => {
  for (const k of ["text", "heading", "divider", "section"]) assert.equal(isStaticKind(k), true)
  for (const k of ["bar", "kpi", "table"]) assert.equal(isStaticKind(k), false)
})

test("the reading order is row then column, and a phone layout replaces it for the tiles it names", () => {
  const layout: LayoutMap = {
    a: { x: 6, y: 0, w: 6, h: 4 }, b: { x: 0, y: 0, w: 6, h: 3 }, c: { x: 0, y: 4, w: 12, h: 5 }, d: { x: 0, y: 9, w: 12, h: 2 },
  }
  assert.deepEqual(readingOrder(["a", "b", "c", "d"], layout), ["b", "a", "c", "d"])
  assert.deepEqual(phoneOrder(["a", "b", "c", "d"], layout, undefined), [{ id: "b", h: 3 }, { id: "a", h: 4 }, { id: "c", h: 5 }, { id: "d", h: 2 }])
  const saved = [{ id: "c", h: 8 }, { id: "gone", h: 3 }, { id: "a", h: 2 }]
  assert.deepEqual(phoneOrder(["a", "b", "c", "d"], layout, saved), [{ id: "c", h: 8 }, { id: "a", h: 2 }, { id: "b", h: 3 }, { id: "d", h: 2 }])
  const moved = movePhoneTile(phoneOrder(["a", "b"], layout, undefined), "a", -1)
  assert.deepEqual(moved.map((p) => p.id), ["a", "b"])
  assert.equal(resizePhoneTile(moved, "a", 999)[0].h, 60)
  assert.equal(resizePhoneTile(moved, "a", 0)[0].h, 1)
})

test("a section folds the tiles after it up to the next section, and the tiles below move up", () => {
  const layout: LayoutMap = {
    s1: { x: 0, y: 0, w: 12, h: 1 }, a: { x: 0, y: 1, w: 6, h: 4 }, b: { x: 6, y: 1, w: 6, h: 4 },
    s2: { x: 0, y: 5, w: 12, h: 1 }, c: { x: 0, y: 6, w: 12, h: 3 },
  }
  const items = Object.keys(layout).map((id) => ({ id, kind: id.startsWith("s") ? "section" : "bar" }))
  assert.deepEqual([...foldedTiles(items, layout, new Set())], [])
  assert.deepEqual([...foldedTiles(items, layout, new Set(["s1"]))].sort(), ["a", "b"])
  assert.deepEqual([...foldedTiles(items, layout, new Set(["s2"]))], ["c"])
  const hidden = foldedTiles(items, layout, new Set(["s1"]))
  const out = collapseLayout(layout, items, hidden)
  assert.deepEqual(Object.keys(out).sort(), ["c", "s1", "s2"])
  assert.equal(out.s2.y, 1)
  assert.equal(out.c.y, 2)
  assert.equal(out.s1.y, 0)
})

test("a heading, a section and a divider are created one row tall and a chart keeps its size (BI-18 part C review fix R6)", () => {
  for (const kind of ["heading", "section", "divider"]) assert.equal(defaultTileHeight(kind), 1)
  assert.equal(defaultTileHeight("bar"), 6)
  assert.equal(defaultTileHeight(undefined, 4), 4)
  assert.equal(defaultTileHeight("kpi", 3), 3)
})

// ── BI-18 part C review fix (BLOCKER) R13: charts with no stored box ──────────

type Case = {
  name: string
  tabs: string[]
  layout: LayoutMap
  charts: { id: string; span: 1 | 2; kind: string }[]
  placed: LayoutMap
}

test("placeUnboxed gives the place the server gives, on the cases both sides share", () => {
  const file = join(import.meta.dir, "..", "..", "ops", "fixtures", "place_unboxed.json")
  const { cases } = JSON.parse(readFileSync(file, "utf8")) as { cases: Case[] }
  assert.ok(cases.length >= 3)
  for (const c of cases) {
    const tabs = c.tabs.map((id) => ({ id, title: id }))
    const out = placeUnboxed(c.charts, c.layout, tabs)
    for (const [id, want] of Object.entries(c.placed)) assert.deepEqual(out[id], want, `${c.name}: ${id}`)
    for (const [id, box] of Object.entries(c.layout)) assert.deepEqual(out[id], box, `${c.name}: stored ${id} moved`)
    assert.equal(Object.keys(out).length, Object.keys(c.layout).length + Object.keys(c.placed).length, c.name)
  }
})

// Seven charts, two with a stored box, five without: the QA board in small.
const SHOWN_IDS = ["a", "b", "c", "d", "e", "f", "g"]
const SHOWN_ITEMS = SHOWN_IDS.map((id) => ({ id, span: 1 as const, kind: "bar" }))
const STORED: LayoutMap = { a: { x: 0, y: 0, w: 6, h: 6, tab: "t_a" }, b: { x: 6, y: 0, w: 6, h: 6, tab: "t_a" } }

test("Move to of a chart with no stored box does not crash and the space it left is closed", () => {
  const shown = placeUnboxed(SHOWN_ITEMS, STORED, [A, B])
  // Drawn: a,b (y 0, stored), c,d (y 6), e,f (y 12), g (y 18). Move c, which had no box.
  const out = moveTileToTab([A, B], shown, SHOWN_IDS, "c", "t_b")
  assert.equal(out.c.tab, "t_b")
  // Only the column of c closes: e and g follow it up, f (under d) and d stay.
  assert.deepEqual([out.e.y, out.f.y, out.g.y], [6, 12, 12])
  assert.equal(out.d.y, 6)
  assert.equal(tabOf([A, B], out, "d"), "t_a")
})

test("Move to of a stored chart while unstored ones exist closes the space they would have filled", () => {
  const shown = placeUnboxed(SHOWN_ITEMS, STORED, [A, B])
  const out = moveTileToTab([A, B], shown, SHOWN_IDS, "a", "t_b")
  // c (below a, over its columns) moves up into a's place; e under c follows c.
  assert.equal(out.c.y, 0)
  assert.equal(out.e.y, 6)
  assert.equal(out.g.y, 12)
  assert.equal(out.b.y, 0)
})

test("moveTileToTab and deleteTab never index a box that is missing", () => {
  const partial: LayoutMap = { a: { x: 0, y: 0, w: 6, h: 6, tab: "t_a" } }
  assert.doesNotThrow(() => moveTileToTab([A, B], partial, SHOWN_IDS, "a", "t_b"))
  assert.doesNotThrow(() => moveTileToTab([A, B], partial, SHOWN_IDS, "c", "t_b"))
  const out = deleteTab({ tabs: [A, B], layout: partial, filters: [] }, SHOWN_IDS, "t_a", true)
  assert.ok(!("error" in out))
})

test("deleting a tab moves every chart on it, stored or not, to the first remaining tab", () => {
  const shown = placeUnboxed(SHOWN_ITEMS, { ...STORED, b: { ...STORED.b, tab: "t_b" } }, [A, B])
  const out = deleteTab({ tabs: [A, B], layout: shown, filters: [] }, SHOWN_IDS, "t_a", true)
  assert.ok(!("error" in out))
  if ("error" in out) return
  for (const id of SHOWN_IDS) assert.equal(out.layout[id].tab, "t_b", id)
  const ys = SHOWN_IDS.map((id) => `${out.layout[id].x},${out.layout[id].y}`)
  assert.equal(new Set(ys).size, ys.length, "no two tiles share a place")
})

test("reordering the tabs leaves every chart, stored or not before, where it showed", () => {
  const shown = placeUnboxed(SHOWN_ITEMS, STORED, [A, B])
  // The page saves the layout as shown; the server stamps nothing more.
  for (const id of SHOWN_IDS) assert.equal(shown[id].tab ?? "t_a", "t_a", id)
  const reordered = moveTab([A, B], "t_b", -1)
  const tagged = Object.fromEntries(Object.entries(shown).map(([id, box]) => [id, { ...box, tab: box.tab ?? "t_a" }]))
  for (const id of SHOWN_IDS) assert.equal(tabOf(reordered, tagged, id), "t_a", id)
})

// ── BI-18 part C review fix (SHOULD-FIX) R15: a new block goes to the bottom as shown ──

test("a new block starts below the charts drawn at the bottom of the tab, stored or not", () => {
  const shown = placeUnboxed(SHOWN_ITEMS, STORED, [A, B])
  // Stored bottom is 6; the unstored charts are drawn down to 24.
  assert.equal(bottomOfTab([A, B], shown, SHOWN_IDS, "t_a"), 24)
  assert.equal(bottomOfTab([A, B], shown, SHOWN_IDS, "t_b"), 0)
  assert.equal(bottomOfTab([], shown, SHOWN_IDS, undefined), 24)
  // The stored map alone would have said 6, which is what put the block in the middle.
  assert.equal(bottomOfTab([A, B], STORED, SHOWN_IDS, "t_a"), 6)
})
