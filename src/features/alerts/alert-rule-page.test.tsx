// `WEB-7` T7 (D2/D4/D7): the alert rule form is its own page. These render
// the page and read the body it sends, so the moved form is exercised through
// the same service calls the list used to make.
//
// `next/navigation` is stubbed before any import resolves (`mock.module` is
// hoisted), and every service read is spied, so nothing leaves the test.

let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/alerts/rules/new",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: (href: string) => {
      pushed.push(href)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import * as realAuth from "@/features/auth/auth-provider"
const real = { ...realAuth }
let permissions: string[] = []
mock.module("@/features/auth/auth-provider", () => ({
  ...real,
  useAuth: () => ({
    user: { id: "u1", permissions },
    hasPermission: (perm: string) => permissions.includes(perm),
    signOut: () => {},
    refresh: () => {},
    loading: false,
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { alertRuleService, assetService, connectorService, dashboardService, metricService } from "@/services"
import type { AlertRule } from "@/services/contracts/alerts"
import { AlertRulePage } from "./alert-rule-page"

afterAll(() => {
  mock.module("@/features/auth/auth-provider", () => real)
})

afterEach(() => {
  cleanup()
  mock.restore()
  pushed = []
  permissions = []
})

beforeEach(() => {
  pushed = []
  permissions = []
})

const RULE: AlertRule = {
  id: "al_1234",
  name: "Visitors in Bali",
  type: "alert",
  mart: "mart_wisman",
  measure: "jumlah",
  agg: "sum",
  op: ">",
  threshold: 100,
  channel: "email",
  target: "sari@meridian.example",
  enabled: true,
}

/** Every read the page makes, answered without a network. */
function stubReads(rules: AlertRule[] = []) {
  spyOn(alertRuleService, "listRules").mockResolvedValue(rules)
  spyOn(dashboardService, "listBoards").mockResolvedValue([{ id: "default", name: "Main" }])
  spyOn(dashboardService, "listMarts").mockResolvedValue([{ name: "mart_wisman" }])
  spyOn(dashboardService, "listMartMeasures").mockResolvedValue(["jumlah", "nights"])
  spyOn(connectorService, "listConnectors").mockResolvedValue([])
  spyOn(metricService, "list").mockResolvedValue([])
  // WEB-8 D8: the table pickers read the catalog.
  spyOn(assetService, "listAssets").mockResolvedValue([
    { id: "a-orders", name: "orders", namespace: "silver" },
    { id: "a-raw", name: "raw_orders", namespace: "bronze" },
  ] as never)
}

/** base-ui's Select commits on the pointer sequence, not on a bare click. */
function pickOption(option: HTMLElement) {
  fireEvent.pointerDown(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseDown(option, { button: 0 })
  fireEvent.pointerUp(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseUp(option, { button: 0 })
  fireEvent.click(option)
}

describe("the alert rule page (WEB-7 T7)", () => {
  it("creates a rule from its own page and goes back to the list", async () => {
    stubReads()
    const create = spyOn(alertRuleService, "createRule").mockResolvedValue(RULE)
    render(<AlertRulePage />)

    fireEvent.change(await screen.findByPlaceholderText("e.g. Foreign visitors dropped"), {
      target: { value: "  Visitors in Bali " },
    })
    fireEvent.change(screen.getByPlaceholderText("https://hooks.slack.com/services/…"), {
      target: { value: " https://hooks.example.com/x " },
    })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    await waitFor(() => expect(create).toHaveBeenCalledTimes(1))
    // A threshold rule goes out as the form holds it (`alertRuleSaveInput`
    // returns the older kinds unchanged; only the load rules are trimmed).
    expect(create.mock.calls[0]?.[0]).toEqual({
      id: "",
      name: "  Visitors in Bali ",
      type: "alert",
      agg: "sum",
      op: ">",
      threshold: 0,
      channel: "webhook",
      target: " https://hooks.example.com/x ",
      enabled: true,
    })
    await waitFor(() => expect(pushed).toEqual(["/alerts"]))
  })

  it("loads a saved rule by id and writes the change back with its id", async () => {
    stubReads([RULE])
    const update = spyOn(alertRuleService, "updateRule").mockResolvedValue(RULE)
    render(<AlertRulePage ruleId="al_1234" />)

    const name = (await screen.findByDisplayValue("Visitors in Bali")) as HTMLInputElement
    expect((screen.getByRole("heading", { level: 1 }) as HTMLElement).textContent).toBe(
      "Edit rule"
    )
    expect((screen.getByDisplayValue("sari@meridian.example") as HTMLInputElement).value).toBe(
      "sari@meridian.example"
    )
    fireEvent.change(name, { target: { value: "Visitors in Jakarta" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(update).toHaveBeenCalledTimes(1))
    expect(update.mock.calls[0]?.[0]).toMatchObject({
      id: "al_1234",
      name: "Visitors in Jakarta",
      channel: "email",
    })
    await waitFor(() => expect(pushed).toEqual(["/alerts"]))
  })

  it("shows the not-found state for an id nothing has", async () => {
    stubReads()
    render(<AlertRulePage ruleId="gone" />)

    expect(await screen.findByText("Rule not found")).toBeDefined()
    expect(screen.getByText("Back to Alerts").closest("a")?.getAttribute("href")).toBe("/alerts")
  })

  // DEC-16 (QA LH-SRC-7-04, SRC-42): the API sends a rule without its target to
  // a caller who may not manage it, and the page says so instead of rendering a
  // form whose Save the API would refuse.
  it("says a rule whose target is hidden cannot be changed", async () => {
    stubReads([{ ...RULE, target: undefined, targetHidden: true }])
    render(<AlertRulePage ruleId="al_1234" />)

    expect(await screen.findByText("You cannot change this rule")).toBeDefined()
    expect(
      screen.getByText(
        "Seeing where a rule sends, or changing it, needs the alert:write permission."
      )
    ).toBeDefined()
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()
    expect(screen.getByText("Back to Alerts").closest("a")?.getAttribute("href")).toBe("/alerts")
  })

  it("still renders the form for a rule whose target is returned", async () => {
    stubReads([RULE])
    render(<AlertRulePage ruleId="al_1234" />)

    expect(await screen.findByRole("button", { name: "Save" })).toBeDefined()
    expect(screen.queryByText("You cannot change this rule")).toBeNull()
  })

  it("says which field is missing instead of saving", async () => {
    stubReads()
    const create = spyOn(alertRuleService, "createRule").mockResolvedValue(RULE)
    render(<AlertRulePage />)

    fireEvent.click(await screen.findByRole("button", { name: "Create" }))

    expect((await screen.findByRole("alert")).textContent).toBe("Name required.")
    expect(create).not.toHaveBeenCalled()
    expect(pushed).toEqual([])
  })

  it("warns that a rule saved before owners will read as the person who saves it (BI-39 D4)", async () => {
    stubReads([{ ...RULE, ownerId: undefined }])
    render(<AlertRulePage ruleId="al_1234" />)

    expect(
      await screen.findByText(
        "This alert was set before alerts had owners and reads as the Dashboard Viewer role. Saving it makes it read as you."
      )
    ).toBeDefined()
  })

  it("shows the server's refusal in the form's own alert", async () => {
    stubReads()
    spyOn(alertRuleService, "createRule").mockRejectedValue(
      new Error("That webhook address is not allowed.")
    )
    render(<AlertRulePage />)

    fireEvent.change(await screen.findByPlaceholderText("e.g. Foreign visitors dropped"), {
      target: { value: "R" },
    })
    fireEvent.change(screen.getByPlaceholderText("https://hooks.slack.com/services/…"), {
      target: { value: "https://hooks.example.com/x" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))

    expect((await screen.findByRole("alert")).textContent).toBe(
      "That webhook address is not allowed."
    )
    expect(pushed).toEqual([])
  })
})

// WEB-8 D8: the tables a quality-incident or freshness rule watches are
// picked from the catalog instead of typed.
describe("the table pickers on the alert rule page (WEB-8 D8)", () => {
  async function chooseType(label: string) {
    fireEvent.click(screen.getByRole("combobox", { name: "Type" }))
    pickOption(await screen.findByRole("option", { name: label }))
  }

  it("picks a quality alert's table from the catalog, with Every table still offered", async () => {
    // Only a user who sees every tenant is offered a quality rule (the API
    // enforces it too), which is the kind whose table is picked here.
    permissions = ["*:*"]
    stubReads()
    const create = spyOn(alertRuleService, "createRule").mockResolvedValue(RULE)
    render(<AlertRulePage />)
    fireEvent.change(await screen.findByPlaceholderText("e.g. Foreign visitors dropped"), {
      target: { value: "Quality" },
    })
    await chooseType("Quality incident opened")

    const field = await screen.findByRole("button", {
      name: "Table (zone.table, or * for every table)",
    })
    fireEvent.click(field)
    const options = await screen.findAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual([
      "Every table",
      "bronze.raw_orders",
      "silver.orders",
    ])
    fireEvent.click(options[2] as HTMLElement)

    fireEvent.change(screen.getByPlaceholderText("https://hooks.slack.com/services/…"), {
      target: { value: "https://hooks.example.com/x" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    await waitFor(() => expect(create).toHaveBeenCalledTimes(1))
    expect(create.mock.calls[0]?.[0]).toMatchObject({ type: "quality_incident", asset: "silver.orders" })
  })

  it("picks a freshness rule's table from the catalog", async () => {
    stubReads()
    const create = spyOn(alertRuleService, "createRule").mockResolvedValue(RULE)
    render(<AlertRulePage />)
    fireEvent.change(await screen.findByPlaceholderText("e.g. Foreign visitors dropped"), {
      target: { value: "Fresh" },
    })
    await chooseType("Dataset freshness")

    const field = await screen.findByRole("button", { name: "Table (namespace.table)" })
    fireEvent.click(field)
    const options = await screen.findAllByRole("option")
    expect(options.map((o) => o.textContent)).toEqual(["bronze.raw_orders", "silver.orders"])
    fireEvent.click(options[0] as HTMLElement)

    fireEvent.change(screen.getByPlaceholderText("https://hooks.slack.com/services/…"), {
      target: { value: "https://hooks.example.com/x" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Create" }))
    await waitFor(() => expect(create).toHaveBeenCalledTimes(1))
    expect(create.mock.calls[0]?.[0]).toMatchObject({ type: "freshness", mart: "bronze.raw_orders" })
  })
})
