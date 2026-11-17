import { strict as assert } from "node:assert"
import { test } from "node:test"
import {
  CONTENT_SITE_REFUSED, MAX_BULLETS, MAX_DRAWN_RANGES, cutBullets, emptyTypeDraft, fillRanges, goalMeasureColumn, httpsOrigin, isTypeBKind,
  progressView, siteAllowed, typeDraftFromDef, typePayload, typeProblems, urlProblem, type TypeDefFields,
} from "./type-b"
import { isStaticKind } from "./dashboard-tabs"
import { builderRules } from "./builder-rules"
import { kindHasClickValue } from "./chart-click"
import { measureLimits, takesSort } from "./chart-shape"
import { formatRules } from "./style-draft"
import type { ChartKind } from "./dashboard-specs"

// The same cases as the Rust test `the_origin_of_an_address_ignores_the_path_and_refuses_user_info`
// (`lakehouse_bi::embed_access`): the server decides at save, the console at render, and they must agree.
test("the origin of an address is its https scheme, host and port in the server's normal form", () => {
  assert.equal(httpsOrigin("https://Img.Example:8443/a/b.png?x=1#f"), "https://img.example:8443")
  assert.equal(httpsOrigin("https://img.example"), "https://img.example")
  assert.equal(httpsOrigin("  https://img.example/a.png  "), "https://img.example")
  assert.equal(httpsOrigin("https://img.example:00443/a"), "https://img.example:443")
  for (const bad of [
    "http://img.example/a.png", "https://ok.example@evil.example/", "https://[::1]/", "//img.example/a.png", "https:///a.png",
    "HTTPS://img.example/", "https://img.example:0/", "https://img.example:70000/", "https://img.example:abc/", "https://-bad.example/",
    "https://bad..example/", "https://bad_host.example/", "javascript:alert(1)", "data:image/png;base64,AAAA", "", "img.example",
  ]) {
    assert.equal(httpsOrigin(bad), null, bad)
  }
})

test("an address is allowed only when its origin is on the list, whatever its path", () => {
  const sites = ["https://img.example", "https://cdn.example:8443"]
  assert.equal(siteAllowed("https://img.example/logo.png", sites), true)
  assert.equal(siteAllowed("https://IMG.example/logo.png", sites), true)
  assert.equal(siteAllowed("https://cdn.example:8443/x", sites), true)
  for (const bad of [
    "https://cdn.example/x", "https://img.example.evil.example/x", "https://img.example@evil.example/x", "http://img.example/x",
    "https://other.example/x",
  ]) {
    assert.equal(siteAllowed(bad, sites), false, bad)
  }
  // A site removed from the list since the card was saved is no longer allowed; an empty list allows nothing.
  assert.equal(siteAllowed("https://img.example/logo.png", ["https://cdn.example:8443"]), false)
  assert.equal(siteAllowed("https://img.example/logo.png", []), false)
  assert.equal(siteAllowed("https://img.example/logo.png", undefined), false)
  assert.equal(siteAllowed(undefined, sites), false)
  assert.equal(siteAllowed(`https://img.example/${"a".repeat(2000)}`, sites), false)
})

test("the builder refuses an address with the server's sentence and says nothing of a blank one", () => {
  assert.equal(urlProblem("", ["https://a.example"]), null)
  assert.equal(urlProblem("https://a.example/x", ["https://a.example"]), null)
  assert.equal(urlProblem("https://b.example/x", ["https://a.example"]), CONTENT_SITE_REFUSED)
  assert.equal(urlProblem("https://a.example/ x", ["https://a.example"]), CONTENT_SITE_REFUSED)
  assert.equal(CONTENT_SITE_REFUSED, "Images and web pages may only come from the sites an administrator allows (Admin > Settings).")
})

test("a progress bar is full above its goal and keeps the real ratio, and a goal of zero or less has nothing to compare with", () => {
  assert.deepEqual(progressView(50, 200), { kind: "ok", value: 50, goal: 200, ratio: 0.25, fill: 0.25 })
  const over = progressView(1120, 1000)
  assert.equal(over.kind, "ok")
  if (over.kind === "ok") {
    assert.equal(over.fill, 1)
    assert.equal(over.ratio, 1.12)
  }
  for (const goal of [0, -5, null, Number.NaN]) assert.deepEqual(progressView(10, goal), { kind: "no-goal" }, String(goal))
  assert.deepEqual(progressView(null, 10), { kind: "no-goal" })
  const negative = progressView(-5, 10)
  assert.equal(negative.kind === "ok" && negative.fill, 0)
})

test("empty ranges between two filled ones are shown as zero, and a stretch that would be a stripe is refused", () => {
  const filled = fillRanges([{ lower: 0, count: 3 }, { lower: 2000, count: 7 }], 500)
  assert.deepEqual(filled, [
    { lower: 0, count: 3 }, { lower: 500, count: 0 }, { lower: 1000, count: 0 }, { lower: 1500, count: 0 }, { lower: 2000, count: 7 },
  ])
  // Rows in any order, and a width that is not a whole number.
  assert.deepEqual(fillRanges([{ lower: 0.5, count: 1 }, { lower: 0, count: 2 }], 0.25), [
    { lower: 0, count: 2 }, { lower: 0.25, count: 0 }, { lower: 0.5, count: 1 },
  ])
  assert.equal(fillRanges([{ lower: 0, count: 1 }, { lower: MAX_DRAWN_RANGES, count: 1 }], 1), null)
  assert.equal(fillRanges([{ lower: 0, count: 1 }, { lower: MAX_DRAWN_RANGES - 1, count: 1 }], 1)?.length, MAX_DRAWN_RANGES)
})

test("a bullet tile draws the first twenty and counts the rest", () => {
  const rows = Array.from({ length: 23 }, (_, i) => i)
  const cut = cutBullets(rows)
  assert.equal(cut.shown.length, MAX_BULLETS)
  assert.equal(cut.more, 3)
  assert.deepEqual(cutBullets([1, 2]), { shown: [1, 2], more: 0 })
})

test("each kind saves only the fields it takes, and reads them back", () => {
  const draft = { ...emptyTypeDraft(), goalNumber: "1000000", bands: ["50", "", "100"], histMode: "count" as const, histCount: "10", url: " https://a.example/x.png ", alt: " A logo " }
  assert.deepEqual(typePayload("progress", draft), { goal: 1000000 })
  assert.deepEqual(typePayload("bullet", draft), { goal: 1000000, bands: [50, 100] })
  assert.deepEqual(typePayload("histogram", draft), { histogram: { count: 10 } })
  assert.deepEqual(typePayload("histogram", { ...draft, histMode: "width", histWidth: "2.5" }), { histogram: { width: 2.5 } })
  assert.deepEqual(typePayload("histogram", { ...draft, histMode: "auto" }), {})
  assert.deepEqual(typePayload("image", draft), { url: "https://a.example/x.png", alt: "A logo" })
  assert.deepEqual(typePayload("web", draft), { url: "https://a.example/x.png" })
  assert.deepEqual(typePayload("bar", draft), {})
  // A goal that is a second value.
  const valued = { ...draft, goalMode: "measure" as const, goalMeasure: "target" }
  assert.deepEqual(typePayload("progress", valued), { goal: { measure: "target" } })
  assert.equal(goalMeasureColumn("progress", valued), "target")
  assert.equal(goalMeasureColumn("bar", valued), undefined)
  assert.equal(goalMeasureColumn("progress", draft), undefined)
  // A bullet's blank target is no target.
  assert.deepEqual(typePayload("bullet", { ...draft, goalNumber: "" }), { bands: [50, 100] })

  const saved: TypeDefFields[] = [
    { goal: 5 }, { goal: { measure: "target" }, bands: [1, 2, 3] }, { histogram: { count: 7 } }, { histogram: { width: 0.5 } }, { url: "https://a.example/x", alt: "x" },
  ]
  for (const def of saved) {
    const back = typeDraftFromDef(def)
    const kind = def.url ? "image" : def.histogram ? "histogram" : def.bands ? "bullet" : "progress"
    assert.deepEqual(typePayload(kind, back), def, JSON.stringify(def))
  }
})

test("what is still needed is named in the builder's own words", () => {
  const d = emptyTypeDraft()
  assert.deepEqual(typeProblems("progress", d), ["Goal number"])
  assert.deepEqual(typeProblems("progress", { ...d, goalNumber: "10" }), [])
  assert.deepEqual(typeProblems("progress", { ...d, goalMode: "measure" }), ["Goal column"])
  assert.deepEqual(typeProblems("bullet", d), [])
  assert.deepEqual(typeProblems("bullet", { ...d, goalNumber: "abc" }), ["Target number"])
  assert.deepEqual(typeProblems("bullet", { ...d, bands: ["30", "20"] }), ["Band limits, rising"])
  assert.deepEqual(typeProblems("bullet", { ...d, bands: ["10", "", "20"] }), [])
  assert.deepEqual(typeProblems("histogram", d), [])
  assert.deepEqual(typeProblems("histogram", { ...d, histMode: "count", histCount: "101" }), ["Number of ranges"])
  assert.deepEqual(typeProblems("histogram", { ...d, histMode: "count", histCount: "2.5" }), ["Number of ranges"])
  assert.deepEqual(typeProblems("histogram", { ...d, histMode: "width", histWidth: "0" }), ["Range width"])
  assert.deepEqual(typeProblems("image", d), ["Address", "Description"])
  assert.deepEqual(typeProblems("image", { ...d, url: "https://a.example/x", alt: "x".repeat(201) }), ["Shorter description"])
  assert.deepEqual(typeProblems("web", d), ["Address"])
  assert.deepEqual(typeProblems("bar", d), [])
})

test("the new kinds are registered where each rule reads its kind", () => {
  const kinds: ChartKind[] = ["progress", "histogram", "bullet", "image", "web"]
  for (const kind of kinds) assert.equal(isTypeBKind(kind), true, kind)
  assert.equal(isTypeBKind("bar"), false)
  // Image and web are static like a note: no data, no filter, no sections.
  assert.deepEqual(kinds.filter(isStaticKind), ["image", "web"])
  for (const kind of ["image", "web"]) {
    const rules = builderRules({ kind, rowsMode: false, grained: false, breakdown: false, other: false, steps: 0, layers: 0, multiGrouped: false, limitHasEffect: true, hasSource: true })
    assert.deepEqual(rules.sections, { data: false, show: false, sortAndFilter: false, moreOptions: false }, kind)
    assert.equal(rules.filters, false)
  }
  // A progress bar and a histogram neither sort nor limit; a bullet keeps its order and has a limit only with a category.
  const rules = (kind: string, hasDimension = false) =>
    builderRules({ kind, rowsMode: false, grained: false, breakdown: false, other: false, steps: 0, layers: 0, multiGrouped: false, limitHasEffect: true, hasSource: true, hasDimension })
  assert.equal(rules("progress").sort, "none")
  assert.equal(rules("progress").limit, false)
  assert.equal(rules("histogram").limit, false)
  assert.equal(rules("bullet").sort, "none")
  assert.equal(rules("bullet").limit, false)
  assert.equal(rules("bullet", true).limit, true)
  for (const kind of ["progress", "histogram", "bullet"]) {
    assert.equal(rules(kind, true).filters, true, kind)
    assert.equal(rules(kind, true).steps, false, kind)
    assert.equal(rules(kind, true).other.shown, false, kind)
    assert.equal(rules(kind, true).bins, false, kind)
    assert.equal(rules(kind, true).referenceLines, false, kind)
    assert.equal(takesSort(kind), false, kind)
  }
  assert.deepEqual(measureLimits("histogram"), { min: 0, max: 0, own: false })
  assert.deepEqual(measureLimits("progress"), { min: 1, max: 1, own: false })
  assert.deepEqual(measureLimits("bullet"), { min: 1, max: 1, own: false })
  // None has a mark that names a value to click.
  for (const kind of kinds) assert.equal(kindHasClickValue(kind), false, kind)
  // BI-17: a progress bar takes number formats and no palette, as a KPI does; a histogram and a bullet are charts.
  assert.equal(formatRules({ kind: "progress", breakdown: false, measures: 1 }).colors, false)
  assert.equal(formatRules({ kind: "progress", breakdown: false, measures: 1 }).values, true)
  assert.equal(formatRules({ kind: "histogram", breakdown: false, measures: 0 }).colors, true)
  assert.equal(formatRules({ kind: "bullet", breakdown: false, measures: 1 }).axes, false)
  assert.equal(formatRules({ kind: "image", breakdown: false, measures: 0 }).values, false)
})
