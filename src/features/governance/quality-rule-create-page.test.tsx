// WEB-7 T3 (D2): the add-quality-rule form is its own page. These tests pin
// what it sends — the threshold the check picker builds, and the typed
// threshold when that is chosen — and DATA-14 (AC1): the new rule runs once.
let pushed: string[] = []
mock.module("next/navigation", () => ({
  usePathname: () => "/governance/data-quality/rules/new",
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

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { AuthProvider } from "@/features/auth/auth-provider"
import { QualityRuleCreatePage } from "./quality-rule-create-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

beforeEach(() => {
  pushed = []
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

// WEB-8 D8: the table is picked from the catalog, and its columns come from
// the asset's own detail.
// Shaped like the API's list rows: a Silver row's `id` is its table key and
// its `name` is the display name.
const CATALOG = [
  { id: "silver.orders", name: "Orders", namespace: "silver" },
  { id: "silver.customers", name: "Customers", namespace: "silver" },
]
const ORDERS_DETAIL = {
  ...CATALOG[0],
  schema: [
    { name: "email", dataType: "string" },
    { name: "amount", dataType: "double" },
  ],
  sample: [],
  qualityChecks: [],
  policySummary: [],
  usage: null,
}

/** Answers the session; records every write. */
function stubApi(permissions: string[] = ["governance:write", "query:read"]) {
  const writes: { method: string; path: string; body?: unknown }[] = []
  spyOn(globalThis, "fetch").mockImplementation((async (
    input: RequestInfo | URL,
    init?: RequestInit
  ) => {
    const path = String(input)
    const method = init?.method ?? "GET"
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Admin", email: null, roles: [], permissions, tenants: [] })
    }
    // Reads the form makes before it can be filled: the catalog's tables and
    // the picked table's columns. Not writes, so they stay out of `writes`.
    if (path.startsWith("/api/catalog/")) {
      return path.endsWith("/silver.orders")
        ? json(ORDERS_DETAIL)
        : json({ error: "asset not found" }, 404)
    }
    if (path.startsWith("/api/catalog")) return json({ assets: CATALOG, namespaces: [] })
    writes.push({
      method,
      path,
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    })
    if (method === "POST" && path.endsWith("/api/governance/quality")) {
      return json({
        id: "r1",
        name: "email_filled",
        asset: "silver.orders",
        dimension: "completeness",
        threshold: "email not null",
        severity: "medium",
        lastStatus: null,
        lastRunAt: null,
        lastValue: null,
        evaluable: true,
        hint: null,
      })
    }
    if (path.endsWith("/run")) {
      return json({
        ruleId: "r1",
        status: "passed",
        value: "100.0% not null",
        ranAt: "2026-11-11T00:00:00Z",
      })
    }
    return json({})
  }) as unknown as typeof fetch)
  return writes
}

function renderPage() {
  return render(
    <AuthProvider>
      <QualityRuleCreatePage />
    </AuthProvider>
  )
}

/** Picks `table` from the catalog combobox the Asset field is. */
async function pickTable(table = "silver.orders") {
  fireEvent.click(await screen.findByRole("button", { name: "Asset" }))
  fireEvent.click(within(await screen.findByRole("listbox")).getByText(table))
}

describe("QualityRuleCreatePage (WEB-7 D2/T3)", () => {
  it("builds the threshold from the picked check and goes back to Data Quality", async () => {
    const writes = stubApi()
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "email_filled" } })
    await pickTable()
    // WEB-8 D8: the column is a list of that table's own columns now.
    chooseOption(await screen.findByRole("combobox", { name: "Column" }), "email")
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    await waitFor(() =>
      expect(writes[0]).toEqual({
        method: "POST",
        path: "/api/governance/quality",
        body: {
          name: "email_filled",
          asset: "silver.orders",
          dimension: "completeness",
          threshold: "email not null",
          severity: "medium",
        },
      })
    )
    // DATA-14 (AC1): the rule is run once, right after it is created.
    await waitFor(() =>
      expect(writes.map((w) => `${w.method} ${w.path}`)).toContain(
        "POST /api/governance/quality/r1/run"
      )
    )
    await waitFor(() => expect(pushed).toEqual(["/governance/data-quality"]))
  })

  it("sends the threshold the person typed when they choose that", async () => {
    const writes = stubApi(["governance:write"])
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "custom" } })
    await pickTable()
    fireEvent.click(screen.getByRole("button", { name: "Write the threshold myself" }))
    fireEvent.change(screen.getByLabelText("Threshold"), {
      target: { value: "rows >= 10" },
    })
    fireEvent.change(screen.getByLabelText("Dimension"), {
      target: { value: "volume" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    await waitFor(() =>
      expect(writes[0]).toEqual({
        method: "POST",
        path: "/api/governance/quality",
        body: {
          name: "custom",
          asset: "silver.orders",
          dimension: "volume",
          threshold: "rows >= 10",
          severity: "medium",
        },
      })
    )
    // QA LH-DATA-14-04 (GOV-10, DEC-17): the first run needs
    // `governance:write`, which this person has — so it is tried. This line
    // asserted "no run" while the guard asked for `query:read`, which is the
    // read permission the run stopped taking.
    await waitFor(() =>
      expect(writes.map((w) => `${w.method} ${w.path}`)).toContain(
        "POST /api/governance/quality/r1/run"
      )
    )
  })

  it("does not try the first run for a person without governance:write", async () => {
    // QA LH-DATA-14-04 (GOV-10, DEC-17): `query:read` alone may read the
    // table, not run a rule by hand.
    const writes = stubApi(["query:read"])
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "custom" } })
    await pickTable()
    fireEvent.click(screen.getByRole("button", { name: "Write the threshold myself" }))
    fireEvent.change(screen.getByLabelText("Threshold"), {
      target: { value: "rows >= 10" },
    })
    fireEvent.change(screen.getByLabelText("Dimension"), {
      target: { value: "volume" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    await waitFor(() =>
      expect(writes.map((w) => `${w.method} ${w.path}`)).toContain(
        "POST /api/governance/quality"
      )
    )
    expect(writes.some((w) => w.path.endsWith("/run"))).toBe(false)
  })

  it("keeps the API's sentence on the page when the rule is refused", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Admin", email: null, roles: [], permissions: ["governance:write"], tenants: [] })
      }
      if (path.startsWith("/api/catalog/")) {
        return path.endsWith("/silver.orders")
          ? json(ORDERS_DETAIL)
          : json({ error: "asset not found" }, 404)
      }
      if (path.startsWith("/api/catalog")) return json({ assets: CATALOG, namespaces: [] })
      if (init?.method === "POST") {
        return json({ error: "threshold: silver.orders is not a table this deployment has" }, 400)
      }
      return json({})
    }) as unknown as typeof fetch)
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "email_filled" } })
    await pickTable()
    // WEB-8 D8: the column is a list of that table's own columns now.
    chooseOption(await screen.findByRole("combobox", { name: "Column" }), "email")
    fireEvent.click(screen.getByRole("button", { name: "Add rule" }))

    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(pushed).toEqual([])
  })

  it("cancel goes back to Data Quality without creating anything", async () => {
    const writes = stubApi()
    renderPage()
    await screen.findByLabelText("Name")
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    expect(pushed).toEqual(["/governance/data-quality"])
    expect(writes).toEqual([])
  })
})

describe("QualityRuleCreatePage table and column pickers (WEB-8 D8)", () => {
  it("offers the catalog's tables as namespace.table", async () => {
    stubApi()
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Asset" }))
    const list = await screen.findByRole("listbox")
    expect(within(list).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "silver.customers",
      "silver.orders",
    ])
  })

  it("offers the picked table's own columns, and keeps the field free text while the table is unknown", async () => {
    stubApi()
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "email_filled" } })
    // No table yet: the check's column is typed, as it always was.
    expect(screen.getByLabelText("Column")).toHaveProperty("tagName", "INPUT")
    fireEvent.change(screen.getByLabelText("Column"), { target: { value: "typed_by_hand" } })
    // Customers has no detail in this stub: the field stays free text rather
    // than offering an empty list.
    await pickTable("silver.customers")
    expect(screen.getByLabelText("Column")).toHaveProperty("tagName", "INPUT")
    expect((screen.getByLabelText("Column") as HTMLInputElement).value).toBe("typed_by_hand")
    // Orders is known: the column is now the table's own list, with the typed
    // value kept and marked (it is not one of them).
    await pickTable("silver.orders")
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Column" })).toBeTruthy()
    )
    const column = screen.getByRole("combobox", { name: "Column" })
    // WEB-9 D2: the trigger shows the option's label; the stored value is
    // still the typed column, kept and marked because this table lacks it.
    expect(column.querySelector('[data-slot="select-value"]')?.textContent).toBe(
      "typed_by_hand (not in this table)"
    )
    fireEvent.click(column)
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "typed_by_hand (not in this table)",
      "email",
      "amount",
    ])
  })

  it("says what to pick before a column is chosen, instead of an empty box", async () => {
    stubApi()
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "email_filled" } })
    await pickTable()
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Column" })).toBeTruthy()
    )
    // WEB-9 S1 review R1-S1 (D2a): the form holds "" until a column is picked,
    // so the trigger names the choice instead of drawing an empty box.
    expect(
      screen
        .getByRole("combobox", { name: "Column" })
        .querySelector('[data-slot="select-value"]')?.textContent
    ).toBe("Pick a column")
    // Choosing one replaces the prompt with the column's own name.
    chooseOption(screen.getByRole("combobox", { name: "Column" }), "amount")
    expect(
      screen
        .getByRole("combobox", { name: "Column" })
        .querySelector('[data-slot="select-value"]')?.textContent
    ).toBe("amount")
  })

  it("keeps a typed column that the table does not have, marked, instead of dropping it", async () => {
    stubApi()
    renderPage()
    fireEvent.change(await screen.findByLabelText("Name"), { target: { value: "email_filled" } })
    fireEvent.change(screen.getByLabelText("Column"), { target: { value: "old_name" } })
    await pickTable("silver.orders")
    await waitFor(() =>
      expect(screen.getByRole("combobox", { name: "Column" })).toBeTruthy()
    )
    const column = screen.getByRole("combobox", { name: "Column" })
    expect(column.querySelector('[data-slot="select-value"]')?.textContent).toBe(
      "old_name (not in this table)"
    )
    fireEvent.click(column)
    expect(screen.getByRole("option", { name: "old_name (not in this table)" })).toBeTruthy()
  })

  it("is free text when the catalog cannot be read, and says why", async () => {
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
      const path = String(input)
      if (path.includes("/api/auth/me")) {
        return json({ id: "u1", name: "Admin", email: null, roles: [], permissions: ["governance:write"], tenants: [] })
      }
      if (path.startsWith("/api/catalog")) return json({ error: "catalog unavailable" }, 503)
      return json({})
    }) as unknown as typeof fetch)
    renderPage()
    expect((await screen.findByLabelText("Asset")) as HTMLInputElement).toHaveProperty("tagName", "INPUT")
    expect(screen.getByText(/Could not read the catalog's tables/)).toBeTruthy()
    fireEvent.change(screen.getByLabelText("Asset"), { target: { value: "silver.orders" } })
    expect((screen.getByLabelText("Asset") as HTMLInputElement).value).toBe("silver.orders")
  })
})
