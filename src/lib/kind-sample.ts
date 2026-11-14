import type { ChartKind } from "./dashboard-specs"

/**
 * BI-58 review round 3, SHOULD-FIX 8 (owner; DESIGN.md §2, "Pilihan visual
 * ditampilkan sebagai preview, bukan daftar teks"): the chart-type gallery
 * shows a picture for every type, also before any data is picked. Where the
 * preview's own rows cannot be drawn as a type, the gallery draws this small
 * sample instead, shaped for that type: a few categories with values, a
 * short time series, a part of a whole, a KPI, a Gantt plan of four tasks, a
 * calendar of six weeks.
 *
 * A fixed fixture in the console: it is never requested from the server and
 * never saved; the gallery says the numbers are a sample until a source and
 * columns are picked. Maps, a picture and a web page need a file or an
 * address to draw, so they are a small drawing (`illustration`) instead.
 */

type Row = Record<string, unknown>

export type KindSample =
  | {
      kind: "rows"
      /** The render spec's fields the drawing reads (`x`, `y`, a split `series`, a gauge's `target`, a note's `text`). */
      spec: { x: string; y: string | string[]; series?: string; target?: number; text?: string }
      /** Definition fields some kinds read (a goal, a pivot's fields). */
      def: Record<string, unknown>
      columns: string[]
      rows: Row[]
    }
  | { kind: "illustration"; picture: "regions" | "points" | "density" | "image" | "web" }

const PLACES = ["Bali", "Java", "Sumatra", "Sulawesi", "Papua"]
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug"]

const rows = (columns: string[], data: unknown[][]): KindSample => ({
  kind: "rows", spec: { x: columns[0]!, y: columns.length > 2 ? columns.slice(1) : columns[1]! }, def: {}, columns,
  rows: data.map((d) => Object.fromEntries(columns.map((c, i) => [c, d[i]]))),
})

/** Five categories with one value, largest first. */
const CATEGORIES = rows(["place", "visits"], [[PLACES[0], 940], [PLACES[1], 760], [PLACES[2], 530], [PLACES[3], 380], [PLACES[4], 210]])
/** Eight months, rising with a dip. */
const SERIES = rows(["month", "visits"], MONTHS.map((m, i) => [m, [320, 410, 380, 520, 610, 580, 720, 810][i]]))
/** Five categories with two values. */
const TWO_VALUES = rows(["place", "visits", "stays"], PLACES.map((p, i) => [p, [940, 760, 530, 380, 210][i], [610, 520, 300, 260, 90][i]]))
/** Eight points with two (or three) values for the X, the Y and the size. */
const POINTS = rows(["place", "spend", "visits", "stays"], [
  ["A", 12, 30, 8], ["B", 18, 42, 14], ["C", 25, 38, 10], ["D", 31, 55, 20],
  ["E", 38, 61, 12], ["F", 44, 58, 24], ["G", 52, 74, 16], ["H", 60, 83, 28],
])
/** Three categories split three ways (heatmap, flow, rings). */
const SPLIT: KindSample = {
  kind: "rows", spec: { x: "place", y: "visits", series: "segment" }, def: {}, columns: ["place", "segment", "visits"],
  rows: PLACES.slice(0, 3).flatMap((p, i) => ["Domestic", "Asia", "Europe"].map((s, j) => ({ place: p, segment: s, visits: [[420, 260, 180], [350, 230, 120], [240, 160, 90]][i]![j] }))),
}

function calendar(): KindSample {
  const start = Date.UTC(2026, 0, 5)
  return {
    kind: "rows", spec: { x: "day", y: "visits" }, def: {}, columns: ["day", "visits"],
    // Six weeks, busier at the weekend, in a fixed pattern.
    rows: Array.from({ length: 42 }, (_, i) => ({
      day: new Date(start + i * 86_400_000).toISOString().slice(0, 10),
      visits: 40 + ((i * 37) % 50) + (i % 7 >= 5 ? 60 : 0),
    })),
  }
}

const SAMPLES: Partial<Record<ChartKind, () => KindSample>> = {
  bar: () => CATEGORIES, hbar: () => CATEGORIES, pie: () => CATEGORIES, rose: () => CATEGORIES,
  funnel: () => CATEGORIES, treemap: () => CATEGORIES, table: () => TWO_VALUES,
  line: () => SERIES, area: () => SERIES, waterfall: () => SERIES,
  stacked: () => TWO_VALUES, combo: () => TWO_VALUES, radar: () => TWO_VALUES,
  scatter: () => ({ ...POINTS, kind: "rows", spec: { x: "place", y: ["spend", "visits"] } }) as KindSample,
  bubble: () => POINTS,
  heatmap: () => SPLIT, sankey: () => SPLIT, sunburst: () => SPLIT,
  boxplot: () => ({
    kind: "rows", spec: { x: "place", y: "spread" }, def: {}, columns: ["place", "spread"],
    rows: PLACES.slice(0, 4).map((p, i) => ({ place: p, spread: [[12, 30, 42, 58, 80], [18, 34, 40, 52, 66], [8, 20, 31, 44, 70], [22, 38, 47, 60, 74]][i] })),
  }),
  calendar,
  histogram: () => ({
    kind: "rows", spec: { x: "lower", y: "count" }, def: {}, columns: ["lower", "count"],
    rows: [0, 10, 20, 30, 40, 50, 60, 70].map((lower, i) => ({ lower, count: [4, 11, 19, 27, 22, 14, 8, 3][i] })),
  }),
  gantt: () => ({
    kind: "rows", spec: { x: "task", y: "task" }, def: {}, columns: ["task", "group", "start", "end", "progress"],
    rows: [
      { task: "Plan", group: "Team A", start: "2026-01-05", end: "2026-01-16", progress: 100 },
      { task: "Build", group: "Team A", start: "2026-01-12", end: "2026-02-06", progress: 60 },
      { task: "Test", group: "Team B", start: "2026-01-29", end: "2026-02-13", progress: 20 },
      { task: "Launch", group: "Team B", start: "2026-02-13", end: "2026-02-20", progress: 0 },
    ],
  }),
  kpi: () => ({ kind: "rows", spec: { x: "", y: "v" }, def: {}, columns: ["v"], rows: [{ v: 1284 }] }),
  gauge: () => ({ kind: "rows", spec: { x: "", y: "v", target: 100 }, def: {}, columns: ["v"], rows: [{ v: 72 }] }),
  progress: () => ({ kind: "rows", spec: { x: "", y: "v" }, def: { goal: 100 }, columns: ["v"], rows: [{ v: 72 }] }),
  bullet: () => ({
    kind: "rows", spec: { x: "place", y: "v" }, def: { goal: 80 }, columns: ["place", "v"],
    rows: PLACES.slice(0, 3).map((p, i) => ({ place: p, v: [92, 68, 51][i] })),
  }),
  pivot: () => ({
    kind: "rows", spec: { x: "", y: "" }, columns: ["place", "year", "__v0"],
    def: { rows: ["place"], columns: ["year"], values: [{ column: "visits", aggregate: "sum" }], totals: "none" },
    rows: PLACES.slice(0, 3).flatMap((p, i) => ["2025", "2026"].map((y, j) => ({ place: p, year: y, __v0: [[940, 1020], [760, 790], [530, 610]][i]![j] }))),
  }),
  text: () => ({ kind: "rows", spec: { x: "", y: "", text: "### Notes\n- Visits rose in **July**\n- Bali leads every month" }, def: {}, columns: [], rows: [] }),
  geomap: () => ({ kind: "illustration", picture: "regions" }),
  pointmap: () => ({ kind: "illustration", picture: "points" }),
  geoheat: () => ({ kind: "illustration", picture: "density" }),
  image: () => ({ kind: "illustration", picture: "image" }),
  web: () => ({ kind: "illustration", picture: "web" }),
}

/** The sample a gallery card draws `kind` from (a drawing for the kinds that need a file or an address). */
export function kindSample(kind: ChartKind): KindSample {
  return (SAMPLES[kind] ?? (() => CATEGORIES))()
}
