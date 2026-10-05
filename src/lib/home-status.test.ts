import { strict as assert } from "node:assert"
import { test } from "node:test"
import { homeStatus, type StatusCheck } from "./home-status"

const ok = (count = 0): StatusCheck => ({ state: "ok", count })
const failed: StatusCheck = { state: "error", count: 0 }
const loading: StatusCheck = { state: "loading", count: 0 }

test("all three read and nothing found is the only way to say all clear", () => {
  const s = homeStatus({ pipelines: ok(), alerts: ok(), sources: ok(), datasets: ok() })
  assert.equal(s.tone, "clear")
  assert.equal(s.headline, "All clear")
  assert.deepEqual(s.parts, ["no failing pipelines", "no open alerts", "all sources healthy", "every dataset has data"])
})

test("with a problem, only the problems are named and the headline counts them", () => {
  const s = homeStatus({ pipelines: ok(1), alerts: ok(), sources: ok(2), datasets: ok() })
  assert.equal(s.tone, "attention")
  assert.equal(s.headline, "3 things need your attention")
  assert.deepEqual(s.parts, ["1 pipeline failing", "2 sources unhealthy"])
})

test("a single problem reads in the singular", () => {
  const s = homeStatus({ pipelines: ok(1), alerts: ok(), sources: ok(), datasets: ok() })
  assert.equal(s.headline, "1 thing needs your attention")
  assert.equal(s.parts[0], "1 pipeline failing")
})

test("a read that failed is never reported as healthy", () => {
  const s = homeStatus({ pipelines: failed, alerts: ok(), sources: ok(), datasets: ok() })
  assert.equal(s.tone, "unknown")
  assert.equal(s.headline, "Status is incomplete")
  assert.deepEqual(s.parts, ["no open alerts", "all sources healthy", "every dataset has data", "could not check pipelines"])
})

test("a known problem outranks a failed read, and the failed read is still named", () => {
  const s = homeStatus({ pipelines: failed, alerts: ok(2), sources: failed, datasets: ok() })
  assert.equal(s.tone, "attention")
  assert.equal(s.headline, "2 things need your attention")
  assert.deepEqual(s.parts, ["2 alerts open", "could not check pipelines, sources"])
})

test("while any read is in flight there is no verdict", () => {
  const s = homeStatus({ pipelines: ok(3), alerts: loading, sources: ok(), datasets: ok() })
  assert.equal(s.tone, "loading")
  assert.deepEqual(s.parts, [])
})

test("datasets with no synced rows count as attention, worded as missing data, not stale", () => {
  const s = homeStatus({ pipelines: ok(), alerts: ok(), sources: ok(), datasets: ok(2) })
  assert.equal(s.tone, "attention")
  assert.equal(s.headline, "2 things need your attention")
  assert.equal(s.parts[0], "2 datasets have no data")
  assert.equal(homeStatus({ pipelines: ok(), alerts: ok(), sources: ok(), datasets: ok(1) }).parts[0], "1 dataset has no data")
})
