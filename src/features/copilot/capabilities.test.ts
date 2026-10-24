import { describe, expect, it } from "bun:test"
import { readFileSync } from "node:fs"

import { toolsFromCaps } from "./capabilities"

describe("toolsFromCaps", () => {
  it("offers ask_user with the Query Data capability in both modes", () => {
    expect(toolsFromCaps(new Set(["data"]), "ask")).toContain("ask_user")
    expect(toolsFromCaps(new Set(["data"]), "build")).toContain("ask_user")
  })

  it("offers the query-limit tools with Query Data in both modes (BI-5)", () => {
    for (const mode of ["ask", "build"] as const) {
      for (const tool of ["get_query_limits", "stop_query"]) expect(toolsFromCaps(new Set(["data"]), mode)).toContain(tool)
    }
  })

  it("offers the read-only metric tools with Query Data in both modes (AI-7)", () => {
    for (const mode of ["ask", "build"] as const) {
      for (const tool of ["list_metrics", "describe_metric", "query_metric"]) expect(toolsFromCaps(new Set(["data"]), mode)).toContain(tool)
      expect(toolsFromCaps(new Set(["data"]), mode)).not.toContain("create_metric")
    }
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

  // BI-2 / AI-7: the metric tools were invisible to the model for the same reason until they were named here.
  it("lists every metric tool the server has", () => {
    const wanted = serverTools.filter((n) => /^(list|describe|query|create|update|delete)_metrics?$/.test(n))
    expect(wanted).toContain("query_metric")
    expect(wanted).toHaveLength(6)
    const offered = toolsFromCaps(new Set(["dashboard"]), "build")
    for (const name of wanted) expect(offered).toContain(name)
  })

  // BI-18 part C / AI-9: the tab tools, named here or they never reach the model.
  it("lists every tab tool the server has", () => {
    const wanted = serverTools.filter((n) => /_tabs?(_|$)|_to_tab$/.test(n))
    expect(wanted.sort()).toEqual(["create_tab", "delete_tab", "move_charts_to_tab", "move_tab", "rename_tab"])
    const offered = toolsFromCaps(new Set(["dashboard"]), "build")
    for (const name of wanted) expect(offered).toContain(name)
    for (const name of wanted) expect(toolsFromCaps(new Set(["dashboard"]), "ask")).not.toContain(name)
  })

  // BI-17 part A / AI-8: the formatting tools.
  it("lists the formatting tools the server has, to build mode only", () => {
    const wanted = serverTools.filter((n) => n === "format_chart" || n === "set_dashboard_theme")
    expect(wanted.sort()).toEqual(["format_chart", "set_dashboard_theme"])
    const offered = toolsFromCaps(new Set(["dashboard"]), "build")
    for (const name of wanted) expect(offered).toContain(name)
    for (const name of wanted) expect(toolsFromCaps(new Set(["dashboard"]), "ask")).not.toContain(name)
  })

  it("names no tool the server does not have", () => {
    for (const name of toolsFromCaps(new Set(["dashboard"]), "build")) expect(serverTools).toContain(name)
  })
})

// BI-1 / AI-10: the export tools, named here or they never reach the model.
describe("the export tools", () => {
  const fixture = JSON.parse(
    readFileSync(`${process.cwd()}/rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json`, "utf8")
  ) as { function: { name: string } }[]
  const serverTools = fixture.map((t) => t.function.name)

  it("are on the server and offered with Query Data in both modes and with the Dashboard Builder", () => {
    expect(serverTools).toContain("export_dashboard")
    expect(serverTools).toContain("export_chart")
    for (const mode of ["ask", "build"] as const) {
      for (const tool of ["export_dashboard", "export_chart"]) expect(toolsFromCaps(new Set(["data"]), mode)).toContain(tool)
    }
    for (const tool of ["export_dashboard", "export_chart"]) expect(toolsFromCaps(new Set(["dashboard"]), "build")).toContain(tool)
  })
})

// RPT-1 / AI-10: the scheduled-report tools, named here or they never reach the model.
describe("the scheduled-report tools", () => {
  const fixture = JSON.parse(
    readFileSync(`${process.cwd()}/rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json`, "utf8")
  ) as { function: { name: string } }[]
  const serverTools = fixture.map((t) => t.function.name)
  const writers = ["create_report_subscription", "update_report_subscription", "delete_report_subscription"]

  it("are on the server", () => {
    for (const tool of [...writers, "list_report_subscriptions"]) expect(serverTools).toContain(tool)
  })

  it("lists in both modes, and creates, changes and deletes in build mode only", () => {
    for (const mode of ["ask", "build"] as const) {
      expect(toolsFromCaps(new Set(["data"]), mode)).toContain("list_report_subscriptions")
    }
    for (const tool of writers) {
      expect(toolsFromCaps(new Set(["dashboard"]), "build")).toContain(tool)
      expect(toolsFromCaps(new Set(["dashboard"]), "ask")).not.toContain(tool)
      expect(toolsFromCaps(new Set(["data"]), "build")).not.toContain(tool)
    }
  })
})
