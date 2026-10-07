import { strict as assert } from "node:assert"
import { test } from "node:test"
import { homePrompts } from "./home-prompts"

test("with nothing known, only the generic starters for the mode are offered", () => {
  const ask = homePrompts({ mode: "ask" })
  assert.equal(ask[0], "What data do we have?")
  assert.ok(ask.every((p) => !p.includes('"')), "no name is invented")
  assert.equal(homePrompts({ mode: "build" })[0], "Suggest a dashboard for the data we have")
})

test("what is wrong right now leads the Ask prompts, by name", () => {
  const out = homePrompts({ mode: "ask", failedPipeline: "gold_export_job", unhealthySource: "crm", lastDashboard: "Sales" })
  assert.deepEqual(out.slice(0, 3), [
    "Why did gold_export_job fail?",
    'What is wrong with the source "crm"?',
    'Explain the charts on "Sales"',
  ])
})

test("Build prompts act on the same things instead of asking about them", () => {
  const out = homePrompts({ mode: "build", failedPipeline: "gold_export_job", lastDashboard: "Sales" })
  assert.deepEqual(out.slice(0, 2), ['Add a chart to "Sales"', 'Retry the failed pipeline "gold_export_job"'])
})
