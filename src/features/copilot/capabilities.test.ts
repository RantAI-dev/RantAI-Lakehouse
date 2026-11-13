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

  it("offers set_cache_rule with the Dashboard Builder in build mode and not in ask mode (BI-4)", () => {
    expect(toolsFromCaps(new Set(["dashboard"]), "build")).toContain("set_cache_rule")
    expect(toolsFromCaps(new Set(["dashboard"]), "ask")).not.toContain("set_cache_rule")
  })

  // BI-37 D8: reading a dashboard's comments is a read, so Ask mode offers it
  // through Query Data, and the composer's allowlist must carry it in the
  // Dashboard Builder too (a tool missing here never reaches the model).
  it("offers list_dashboard_comments with Query Data in both modes and Dashboard Builder in build", () => {
    for (const mode of ["ask", "build"] as const) {
      expect(toolsFromCaps(new Set(["data"]), mode)).toContain("list_dashboard_comments")
    }
    expect(toolsFromCaps(new Set(["dashboard"]), "build")).toContain("list_dashboard_comments")
  })

  it("does not offer ask_user when Query Data is off", () => {
    expect(toolsFromCaps(new Set(["dashboard"]), "build")).not.toContain("ask_user")
  })

  // AI-4: explaining a calculated field is a read, so Ask mode offers it
  // through Query Data; the tools that change a formula stay in the
  // Dashboard Builder.
  it("offers list_calculated_fields with Query Data in both modes and keeps the formula writes in the builder (AI-4)", () => {
    for (const mode of ["ask", "build"] as const) {
      expect(toolsFromCaps(new Set(["data"]), mode)).toContain("list_calculated_fields")
      expect(toolsFromCaps(new Set(["data"]), mode)).not.toContain("create_calculated_field")
      expect(toolsFromCaps(new Set(["data"]), mode)).not.toContain("validate_formula")
    }
  })

  // AI-2 AC1: "show only 2026" changes the person's own view of the page and
  // nothing else, so Ask mode offers it through Query Data; saving the board's
  // default filters changes what every viewer sees and stays in the Builder.
  it("offers set_board_view_filters with Query Data in both modes and keeps save_board_filters in the builder (AI-2)", () => {
    for (const mode of ["ask", "build"] as const) {
      expect(toolsFromCaps(new Set(["data"]), mode)).toContain("set_board_view_filters")
      expect(toolsFromCaps(new Set(["data"]), mode)).not.toContain("save_board_filters")
    }
  })
})

// BI-26 slice 2 D10 (AI-11-AC3): the embed tools, named here or they never
// reach the model. The snippet is a read, so Ask mode offers it; the three
// writes belong to the Dashboard Builder's build mode.
describe("the signed-embed tools", () => {
  const fixture = JSON.parse(
    readFileSync(`${process.cwd()}/rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json`, "utf8")
  ) as { function: { name: string } }[]
  const serverTools = fixture.map((t) => t.function.name)
  const writers = ["enable_embed", "set_embed_filter_modes", "withdraw_embed_tokens"]

  it("are on the server", () => {
    for (const tool of [...writers, "get_embed_snippet"]) expect(serverTools).toContain(tool)
  })

  it("offers the snippet in both modes and the three writes in build mode only", () => {
    for (const mode of ["ask", "build"] as const) {
      expect(toolsFromCaps(new Set(["data"]), mode)).toContain("get_embed_snippet")
    }
    for (const tool of writers) {
      expect(toolsFromCaps(new Set(["dashboard"]), "build")).toContain(tool)
      expect(toolsFromCaps(new Set(["dashboard"]), "ask")).not.toContain(tool)
      expect(toolsFromCaps(new Set(["data"]), "build")).not.toContain(tool)
    }
    expect(toolsFromCaps(new Set(["dashboard"]), "build")).toContain("get_embed_snippet")
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

// BI-18 part D (T4): the report and template tools, named here or they never
// reach the model. All six need `dashboard:write`, so they belong to the
// Dashboard Builder's build mode only.
describe("the report and template tools", () => {
  const fixture = JSON.parse(
    readFileSync(`${process.cwd()}/rust/crates/lakehouse-api/tests/fixtures/tool_schemas.json`, "utf8")
  ) as { function: { name: string } }[]
  const serverTools = fixture.map((t) => t.function.name)
  const tools = [
    "create_report", "update_report", "delete_report",
    "list_templates", "create_dashboard_from_template", "delete_template",
  ]

  it("are on the server and offered with the Dashboard Builder in build mode only", () => {
    for (const tool of tools) {
      expect(serverTools).toContain(tool)
      expect(toolsFromCaps(new Set(["dashboard"]), "build")).toContain(tool)
      expect(toolsFromCaps(new Set(["dashboard"]), "ask")).not.toContain(tool)
      expect(toolsFromCaps(new Set(["data"]), "build")).not.toContain(tool)
    }
  })
})
