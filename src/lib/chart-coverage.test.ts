import { strict as assert } from "node:assert"
import { readFileSync } from "node:fs"
import { join } from "node:path"
import { test } from "node:test"
import { KIND_GROUPS } from "@/features/dashboards/chart-builder"
import type { ChartKind } from "./dashboard-specs"

/**
 * The coverage check of `BI-16` part B (decision 9): every Metabase chart type
 * maps to a kind the console draws, or to a decision that drops it. The list is
 * in `docs/core/features/progress-histogram-bullet-cards.md` ("Coverage against
 * Metabase") and here; the test reads the page and fails when the two disagree,
 * so neither can drift on its own.
 */
type Coverage = { metabase: string; kinds: ChartKind[] } | { metabase: string; decision: string }

const COVERAGE: Coverage[] = [
  { metabase: "Table", kinds: ["table"] },
  { metabase: "Pivot table", kinds: ["pivot"] },
  { metabase: "Number", kinds: ["kpi"] },
  // A KPI with a comparison and a sparkline (`compare` on the definition, BI-16 A).
  { metabase: "Trend", kinds: ["kpi"] },
  { metabase: "Detail", decision: "Not a tile kind: the one-record view in the record list (BI-16 A)." },
  { metabase: "Progress", kinds: ["progress"] },
  { metabase: "Gauge", kinds: ["gauge"] },
  // Metabase's "row" chart is the horizontal bar.
  { metabase: "Line, bar, combo, area, row", kinds: ["line", "bar", "combo", "area", "hbar"] },
  { metabase: "Waterfall", kinds: ["waterfall"] },
  { metabase: "Scatter, bubble", kinds: ["scatter", "bubble"] },
  { metabase: "Pie, donut", kinds: ["pie"] },
  { metabase: "Funnel", kinds: ["funnel"] },
  { metabase: "Maps", kinds: ["geomap", "pointmap", "geoheat"] },
  { metabase: "Sankey", kinds: ["sankey"] },
  { metabase: "Box plot", kinds: ["boxplot"] },
]

/** The kinds the builder offers, which are the kinds a person can add. */
const OFFERED = new Set<string>(KIND_GROUPS.flatMap((g) => g.items.map((i) => i.value)))

/** The first column of the page's coverage table. */
function pageRows(): string[] {
  const page = readFileSync(join(process.cwd(), "docs/core/features/progress-histogram-bullet-cards.md"), "utf8")
  const section = page.split("## Coverage against Metabase")[1]?.split("\n## ")[0] ?? ""
  return section
    .split("\n")
    .filter((line) => line.startsWith("|"))
    .slice(2) // the header and its rule
    .map((line) => line.split("|")[1].trim())
}

test("every Metabase chart type maps to a kind the builder offers, or to a decision that says why not", () => {
  for (const row of COVERAGE) {
    if ("decision" in row) {
      assert.ok(row.decision.length > 20, `${row.metabase}: a dropped type needs its reason`)
      continue
    }
    assert.ok(row.kinds.length > 0, row.metabase)
    for (const kind of row.kinds) assert.ok(OFFERED.has(kind), `${row.metabase}: "${kind}" is not a kind the builder offers`)
  }
})

test("the list here and the table on the feature page name the same Metabase types", () => {
  assert.deepEqual(pageRows(), COVERAGE.map((c) => c.metabase))
})

test("the five kinds of BI-16 part B are offered", () => {
  for (const kind of ["progress", "histogram", "bullet", "image", "web"]) assert.ok(OFFERED.has(kind), kind)
})
