import { describe, expect, it } from "bun:test"
import { readFileSync } from "node:fs"

import { toolsFromCaps } from "./capabilities"

describe("toolsFromCaps", () => {
  it("offers ask_user with the Query Data capability in both modes", () => {
    expect(toolsFromCaps(new Set(["data"]), "ask")).toContain("ask_user")
    expect(toolsFromCaps(new Set(["data"]), "build")).toContain("ask_user")
  })

  it("does not offer ask_user when Query Data is off", () => {
    expect(toolsFromCaps(new Set(["dashboard"]), "build")).not.toContain("ask_user")
  })
})

describe("the dashboard capability against the server's tool list", () => {
  // BI-8 review fix (BLOCKER) R1: the composer sends these names as the turn's
  // allowlist, so a dashboard tool the server has but this list lacks is never
  // offered to the model (the assistant said it had no calculated-field tools).
  const fixture = JSON.parse(
    readFileSync(`${process.cwd()}/rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json`, "utf8")
  ) as { function: { name: string } }[]
  const serverTools = fixture.map((t) => t.function.name)

  it("lists every chart, board, SQL source and calculated-field tool the server has", () => {
    const wanted = serverTools.filter((n) =>
      /(chart|board|sql_source|calculated_field|formula|describe_mart|suggest_dashboard)/.test(n)
    )
    expect(wanted).toContain("create_calculated_field")
    const offered = toolsFromCaps(new Set(["dashboard"]), "build")
    for (const name of wanted) expect(offered).toContain(name)
  })

  it("names no tool the server does not have", () => {
    for (const name of toolsFromCaps(new Set(["dashboard"]), "build")) expect(serverTools).toContain(name)
  })
})
