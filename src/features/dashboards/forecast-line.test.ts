import assert from "node:assert/strict"
import test from "node:test"
import reference from "@/lib/forecast-reference.json"
import { bucketLabel } from "@/lib/time-grain"
import { buildOption, forecastNote, withReferenceLines, type LineContext } from "./chart-option"

// BI-17 part B, T4 (plan D4): the forecast is drawn from the rows on screen.
type Obj = Record<string, unknown>
const pad = (n: number) => String(n).padStart(2, "0")
/** `n` monthly buckets ending at `last` (year, month), as the server writes them. */
function months(n: number, lastYear: number, lastMonth: number): string[] {
  return Array.from({ length: n }, (_, i) => {
    const k = lastYear * 12 + lastMonth - 1 - (n - 1 - i)
    return `${Math.floor(k / 12)}-${pad((k % 12) + 1)}-01`
  })
}
const AIR = reference.airline.values.slice(0, 48)
const NOW = Date.UTC(2027, 5, 20) // 20 June 2027, well after the data ends

function chart(opts: { kind?: "line" | "bar" | "area"; values?: number[]; buckets?: string[]; grain?: string; now?: number; breakdown?: boolean; line?: Obj } = {}) {
  const values = opts.values ?? AIR
  const buckets = opts.buckets ?? months(values.length, 2026, 12)
  const labels = buckets.map((b) => bucketLabel((opts.grain ?? "month") as never, b))
  const rows = labels.map((x, i) => ({ x, v: values[i] }))
  const built = buildOption({ kind: opts.kind ?? "line", x: "x", y: "v" }, rows, false)
  const line = { kind: "forecast", horizon: 6, ...(opts.line ?? {}) } as never
  const ctx: LineContext = {
    values: () => values,
    buckets: { grain: (opts.grain ?? "month") as never, raw: buckets, timeZone: "Asia/Jakarta", now: opts.now ?? NOW, breakdown: opts.breakdown },
  }
  return {
    option: withReferenceLines(built, [line], false, false, ctx, ["v"]) as unknown as { series: Obj[]; xAxis: { data: string[] }; legend?: Obj },
    note: forecastNote([line], ctx, false),
    built,
  }
}

test("a monthly line gets six future months, a dashed line and a stacked band", () => {
  const { option, note } = chart()
  assert.equal(option.series.length, 4)
  const [actual, lower, width, point] = option.series
  assert.equal((actual.data as unknown[]).length, 48)
  assert.deepEqual(option.xAxis.data.slice(-8), ["Nov 2026", "Dec 2026", "Jan 2027", "Feb 2027", "Mar 2027", "Apr 2027", "May 2027", "Jun 2027"])
  assert.equal(option.xAxis.data.length, 54)
  // The line starts on the last actual point and runs six periods; the band is stacked under it.
  const data = point.data as (number | null)[]
  assert.equal(data.length, 54)
  assert.equal(data[46], null)
  assert.equal(data[47], AIR[47])
  assert.ok(data.slice(48).every((v) => typeof v === "number"))
  assert.equal(lower.stack, "forecast-band")
  assert.equal(width.stack, "forecast-band")
  assert.equal((point.lineStyle as Obj).type, "dashed")
  assert.ok(note?.startsWith("Forecast: level"))
  assert.ok(note?.includes("95 % band"))
  // The helper series stay out of the tooltip and the legend.
  assert.deepEqual((lower.tooltip as Obj).show, false)
  assert.deepEqual((width.tooltip as Obj).show, false)
})

test("the band is wider the further ahead, and the lower edge sits below the line", () => {
  const { option } = chart()
  const lower = option.series[1].data as (number | null)[]
  const width = option.series[2].data as (number | null)[]
  const point = option.series[3].data as (number | null)[]
  for (let i = 49; i < 54; i++) {
    assert.ok((width[i] as number) >= (width[i - 1] as number))
    assert.ok((lower[i] as number) < (point[i] as number))
  }
})

test("the tooltip of the forecast line writes the band", () => {
  const { option } = chart()
  const tip = (option.series[3].tooltip as { valueFormatter: (v: unknown, i: number) => string }).valueFormatter
  assert.match(tip(100, 50), /\(95 % band .+ to .+\)/)
  assert.equal(tip(100, 3).includes("band"), false)
})

test("a bar chart gets the forecast as a line over the bars", () => {
  const { option } = chart({ kind: "bar" })
  assert.equal(option.series[0].type, "bar")
  assert.equal(option.series[3].type, "line")
  assert.equal(option.xAxis.data.length, 54)
})

test("quarters, weeks and days continue their own grain in the labels", () => {
  const quarters = Array.from({ length: 24 }, (_, i) => `${2021 + Math.floor(i / 4)}-${pad(((i % 4) * 3) + 1)}-01`)
  const q = chart({ values: AIR.slice(0, 24), buckets: quarters, grain: "quarter", now: Date.UTC(2027, 5, 1) })
  assert.deepEqual(q.option.xAxis.data.slice(-2), ["Q1 2028", "Q2 2028"])
  const day = (n: number) => new Date(Date.UTC(2026, 0, 1 + n)).toISOString().slice(0, 10)
  const days = chart({ values: AIR.slice(0, 40), buckets: Array.from({ length: 40 }, (_, i) => day(i)), grain: "day", now: Date.UTC(2026, 5, 1), line: { horizon: 3 } })
  assert.deepEqual(days.option.xAxis.data.slice(-3), ["2026-02-10", "2026-02-11", "2026-02-12"])
})

test("the unfinished current month is left out and the forecast starts on it", () => {
  const values = AIR.slice(0, 30)
  const buckets = months(30, 2027, 6)
  const { option, note } = chart({ values, buckets, now: Date.UTC(2027, 5, 20, 12) }) // 20 June: June is open
  const point = option.series[3].data as (number | null)[]
  // 29 finished months; the first forecast point sits on the category of the open June.
  assert.equal(point[28], values[28])
  assert.equal(typeof point[29], "number")
  assert.equal(option.xAxis.data.length, 30 + 5)
  assert.ok(note?.includes("current period is not finished"))
})

test("a refusal draws no line and puts the sentence in the note", () => {
  const { option, note, built } = chart({ values: [1, 2, 3], buckets: months(3, 2026, 12) })
  assert.equal(option.series.length, (built.series as unknown[]).length)
  assert.equal(note, "A forecast needs at least 5 periods; this chart has 3.")
})

test("a chart that cannot be forecast says why, in the note, and draws nothing", () => {
  assert.equal(chart({ breakdown: true }).option.series.length, 1)
  assert.match(chart({ breakdown: true }).note ?? "", /remove the breakdown/)
  assert.match(chart({ grain: "month_of_year" }).note ?? "", /dates grouped by/)
  assert.match(chart({ buckets: AIR.map((_, i) => `north ${i}`) }).note ?? "", /could not be read as a run of periods/)
  const ctx = chart().built
  assert.equal(withReferenceLines(ctx, [{ kind: "forecast", horizon: 3 } as never], true, false, { values: () => AIR, buckets: { grain: "month", raw: months(48, 2026, 12), timeZone: "UTC" } }, ["v"]), ctx)
  assert.equal(forecastNote([{ kind: "forecast", horizon: 3 } as never], { values: () => AIR, buckets: { grain: "month", raw: months(48, 2026, 12), timeZone: "UTC" } }, true), "A forecast is not drawn on horizontal bars.")
})

test("a chart with no forecast line gets no forecast series, labels or note", () => {
  const built = chart().built
  const out = withReferenceLines(built, [{ kind: "average" } as never], false, false, { values: () => AIR }) as unknown as { series: Obj[]; xAxis: { data: string[] } }
  assert.equal(out.series.length, 1)
  assert.equal(out.xAxis.data.length, 48)
  assert.equal(forecastNote(undefined, undefined, false), undefined)
  assert.equal(forecastNote([{ kind: "average" } as never], { values: () => AIR }, false), undefined)
})

// BI-17 B QA fix F1/F2: "Visitors over time (BI-9 QA)" is a daily line chart of
// 153 rows over about 14 months whose last row (2026-11-12) is after today
// (2026-10-24). Most days have no row, so the honest answer is the gap rule;
// it used to say "The latest period has no value" because the last row was
// dropped as unfinished and the day before it had no row.
function sparseDaily(): { buckets: string[]; values: number[] } {
  const buckets: string[] = []
  const end = Date.UTC(2026, 10, 12)
  // 153 distinct days, roughly every 2.8 days back from 12 Nov 2026.
  for (let i = 0; i < 153; i++) buckets.unshift(new Date(end - Math.round(i * 2.8) * 86_400_000).toISOString().slice(0, 10))
  const unique = [...new Set(buckets)]
  return { buckets: unique, values: unique.map((_, i) => 8000 + (i % 7) * 100 + i * 5) }
}
const TODAY = Date.UTC(2026, 9, 24, 9, 0)

test("a sparse daily chart that ends in the future says the days are missing, not that the latest period is empty", () => {
  const { buckets, values } = sparseDaily()
  const { note, option, built } = chart({ values, buckets, grain: "day", now: TODAY, line: { horizon: 30 } })
  assert.match(note ?? "", /of the \d+ periods from the first to the last have no data, more than 10 %/)
  assert.match(note ?? "", /coarser period \(for example month\)/)
  assert.doesNotMatch(note ?? "", /latest period/)
  assert.equal(option.series.length, (built.series as unknown[]).length)
})

test("the same data grouped by month forecasts", () => {
  const { buckets, values } = sparseDaily()
  const byMonth = new Map<string, number>()
  buckets.forEach((b, i) => byMonth.set(`${b.slice(0, 7)}-01`, (byMonth.get(`${b.slice(0, 7)}-01`) ?? 0) + values[i]))
  const months = [...byMonth.keys()]
  const { note, option } = chart({ values: [...byMonth.values()], buckets: months, grain: "month", now: TODAY, line: { horizon: 6 } })
  assert.ok(note?.startsWith("Forecast: "))
  assert.equal(option.series.length, 4)
  assert.ok(note?.includes("current period is not finished"))
})

test("a dense daily chart that ends in the future and on a day with no row continues from its last day with data", () => {
  // 100 consecutive days, then the last row two days after a one-day gap, dated after today.
  const start = Date.UTC(2026, 6, 1)
  const buckets = Array.from({ length: 100 }, (_, i) => new Date(start + i * 86_400_000).toISOString().slice(0, 10))
  const values = buckets.map((_, i) => 5000 + 20 * i + (i % 7) * 50)
  buckets.push(new Date(start + 102 * 86_400_000).toISOString().slice(0, 10)) // 2026-10-11 + 2 days
  values.push(7000)
  const { note, option } = chart({ values, buckets, grain: "day", now: Date.UTC(2026, 9, 11, 9), line: { horizon: 5 } })
  assert.ok(note?.startsWith("Forecast: "))
  assert.equal(option.series.length, 4)
  // History ended on the last day with data before the unfinished one; the forecast follows that day.
  assert.match(note ?? "", /current period is not finished/)
  assert.match(note ?? "", /2 empty periods at the end are left out/)
  // The last day with data is 8 Oct, so the first day ahead is 9 Oct (the axis keeps the unfinished 11 Oct row as drawn).
  const labels = option.xAxis.data
  assert.equal(labels[labels.length - 5], "2026-10-09")
})
