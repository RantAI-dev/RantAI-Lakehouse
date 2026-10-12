import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "bun:test"
import type { AlertRule } from "@/services/contracts/alerts"
import {
  RUN_ALERTS_OFF,
  RunAlertsNotice,
  alertRuleFormFields,
  alertRuleSaveInput,
  connectorForKind,
  seesEveryTenant,
} from "./alerts-page"
import { ruleKindLabel, ruleScope } from "./rules-columns"

afterEach(cleanup)

// SRC-7 task 8: the three assertions below gained the keys the helper returns
// for the connector and upload kinds (`connector`, `allConnectors`, `note`),
// all false/null for these three kinds; `toEqual` compares every key, so the
// expected objects had to name them. Nothing else about them changed.
const NO_LOAD_FIELDS = { connector: false, allConnectors: false, note: null }

// Pure logic, no DOM (matching src/lib/*.test.ts's convention): which form
// fields the rule editor shows for each `AlertRule.type`. `freshness`
// targets a `dataset_sla.table_name` via the reused `mart` field and sends
// no board — the backend clears measure/agg/threshold/board for this kind
// (WS5 item E3).
describe("alertRuleFormFields", () => {
  it("shows the freshness target field, not mart/measure or board, for a freshness rule", () => {
    expect(alertRuleFormFields("freshness")).toEqual({
      martMeasure: false,
      board: false,
      freshnessTarget: true,
      ...NO_LOAD_FIELDS,
    })
  })

  it("shows mart/measure, not board or the freshness target, for an alert rule", () => {
    expect(alertRuleFormFields("alert")).toEqual({
      martMeasure: true,
      board: false,
      freshnessTarget: false,
      ...NO_LOAD_FIELDS,
    })
  })

  it("shows the board picker, not mart/measure or the freshness target, for a digest rule", () => {
    expect(alertRuleFormFields("digest")).toEqual({
      martMeasure: false,
      board: true,
      freshnessTarget: false,
      ...NO_LOAD_FIELDS,
    })
  })
})

const NONE = { martMeasure: false, board: false, freshnessTarget: false }
const NOT_RAISED_SRC11 =
  "Nothing raises this yet. It starts working when automatic pausing (SRC-11) is installed."
const NOT_RAISED_SRC8 =
  "Nothing raises this yet. It starts working when schema change detection (SRC-8) is installed."

describe("alertRuleFormFields for connector and upload rules", () => {
  it("shows a connector select and no mart, measure, board or table for each connector kind", () => {
    for (const kind of ["connector_failure", "connector_repeated_failure", "connector_success"]) {
      const fields = alertRuleFormFields(kind)
      expect(fields).toMatchObject({ ...NONE, connector: true, note: null })
    }
  })

  it("offers All connectors only to an administrator who sees every tenant, and never for a success rule", () => {
    expect(alertRuleFormFields("connector_failure", false).allConnectors).toBe(false)
    expect(alertRuleFormFields("connector_failure", true).allConnectors).toBe(true)
    expect(alertRuleFormFields("connector_repeated_failure", true).allConnectors).toBe(true)
    expect(alertRuleFormFields("connector_success", true).allConnectors).toBe(false)
  })

  it("shows no connector select for an upload rule, whoever is signed in", () => {
    expect(alertRuleFormFields("upload_failure", true)).toEqual({
      ...NONE,
      connector: false,
      allConnectors: false,
      note: null,
    })
  })

  it("says under the disabled and schema change kinds which later work raises them", () => {
    expect(alertRuleFormFields("connector_disabled").note).toBe(NOT_RAISED_SRC11)
    expect(alertRuleFormFields("connector_schema_change").note).toBe(NOT_RAISED_SRC8)
  })

  it("keeps All connectors only for a kind that can take it when the kind changes", () => {
    expect(connectorForKind("connector_success", "*", true)).toBeUndefined()
    expect(connectorForKind("connector_failure", "*", true)).toBe("*")
    expect(connectorForKind("connector_failure", "*", false)).toBeUndefined()
    expect(connectorForKind("connector_failure", "conn-a", false)).toBe("conn-a")
    expect(connectorForKind("upload_failure", "conn-a", true)).toBeUndefined()
    expect(connectorForKind("alert", "conn-a", true)).toBeUndefined()
  })

  it("sees every tenant only with the literal *:* grant the API checks", () => {
    expect(seesEveryTenant(["*:*"])).toBe(true)
    expect(seesEveryTenant(["alert:write", "connector:manage"])).toBe(false)
    expect(seesEveryTenant(undefined)).toBe(false)
  })
})

const FORM: AlertRule = {
  id: "",
  name: "  Load failed ",
  type: "connector_failure",
  mart: "mart_x",
  measure: "m",
  board: "b",
  agg: "sum",
  op: ">",
  threshold: 3,
  channel: "webhook",
  target: " https://hooks.example.com/x ",
  enabled: true,
  connector: "conn-a",
}

describe("alertRuleSaveInput", () => {
  it("sends a connector rule with only the fields that apply to it", () => {
    expect(alertRuleSaveInput(FORM)).toEqual({
      name: "Load failed",
      type: "connector_failure",
      channel: "webhook",
      target: "https://hooks.example.com/x",
      enabled: true,
      connector: "conn-a",
    })
  })

  it("sends the severity of a load rule only when one is set", () => {
    expect(alertRuleSaveInput({ ...FORM, severity: "high" })).toHaveProperty("severity", "high")
    expect(alertRuleSaveInput(FORM)).not.toHaveProperty("severity")
  })

  it("sends no connector for an upload rule", () => {
    const body = alertRuleSaveInput({ ...FORM, type: "upload_failure", connector: "conn-a" })
    expect(body).not.toHaveProperty("connector")
    expect(body.type).toBe("upload_failure")
  })

  it("sends the form unchanged for the older kinds", () => {
    const alert = { ...FORM, type: "alert", connector: undefined }
    expect(alertRuleSaveInput(alert)).toBe(alert)
  })
})

describe("the rules table's labels and scope", () => {
  const connectors = [{ id: "conn-a", name: "northwind" }]

  it("names the six kinds in plain words", () => {
    expect(ruleKindLabel("connector_failure")).toBe("Connector run failed")
    expect(ruleKindLabel("connector_repeated_failure")).toBe("Connector failed 3 runs in a row")
    expect(ruleKindLabel("connector_disabled")).toBe("Connector disabled")
    expect(ruleKindLabel("connector_schema_change")).toBe("Connector schema changed")
    expect(ruleKindLabel("connector_success")).toBe("Connector run succeeded")
    expect(ruleKindLabel("upload_failure")).toBe("Upload failed")
  })

  it("shows the connector's name when known, its id otherwise, All connectors for *, Uploads for uploads", () => {
    expect(ruleScope({ type: "connector_failure", connector: "conn-a" }, connectors)).toBe("northwind")
    expect(ruleScope({ type: "connector_failure", connector: "conn-z" }, connectors)).toBe("conn-z")
    expect(ruleScope({ type: "connector_failure", connector: "*" }, connectors)).toBe("All connectors")
    expect(ruleScope({ type: "upload_failure", connector: "*" }, connectors)).toBe("Uploads")
    expect(ruleScope({ type: "alert" }, connectors)).toBeNull()
  })
})

describe("RunAlertsNotice", () => {
  it("says run alerts are not reported when the API says the token is not set", () => {
    render(<RunAlertsNotice runEventsConfigured={false} />)
    expect(screen.getByRole("status").textContent).toBe(RUN_ALERTS_OFF)
    expect(RUN_ALERTS_OFF).toBe(
      "Run alerts are not being reported: PIPELINE_RUN_TOKEN is not set on the API."
    )
  })

  it("shows nothing when reports can arrive or the status could not be read", () => {
    const { container } = render(
      <>
        <RunAlertsNotice runEventsConfigured={true} />
        <RunAlertsNotice runEventsConfigured={null} />
      </>
    )
    expect(container.textContent).toBe("")
  })
})
