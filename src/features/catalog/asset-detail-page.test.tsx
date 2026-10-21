// DATA-12: the certified / deprecated mark in the asset page's header. The
// page is rendered whole, with `fetch` stubbed: the mark must come from the
// asset the API sent, and an asset without one must show nothing.
mock.module("next/navigation", () => ({
  usePathname: () => "/data/assets/silver.orders",
  useParams: () => ({ assetId: "silver.orders" }),
  useSearchParams: () => new URLSearchParams(""),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { AssetDetail } from "@/services/contracts/assets"
import { AssetDetailPage } from "./asset-detail-page"

afterEach(() => {
  cleanup()
  mock.restore()
})

const ASSET: AssetDetail = {
  id: "silver.orders",
  name: "Orders Silver",
  namespace: "silver",
  type: "table",
  layer: "silver",
  tier: "hot",
  classification: "internal",
  owner: "Data Platform",
  domain: "sales",
  description: "One row per order.",
  format: "MergeTree",
  engine: "hot-store",
  rows: 10,
  sizeBytes: 100,
  columnCount: 1,
  freshnessLagSeconds: 60,
  lastUpdated: "2026-10-01T00:00:00Z",
  health: "healthy",
  residency: "id-jakarta",
  schema: [{ name: "id", dataType: "Int64" }],
  sample: [],
  qualityChecks: [],
  policySummary: [],
  usage: null,
  recentQueries: [],
  dependents: [],
  changeHistory: [],
  snapshots: [],
  schemaVersions: [],
  upstream: [],
  downstream: [],
} as unknown as AssetDetail

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function stubApi(
  asset: Partial<AssetDetail>,
  permissions = ["catalog:read"],
  certification: () => Response = () => json({ ok: true })
) {
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL, init?: RequestInit) => {
    const path = String(input)
    if (init?.method === "PUT" && path.endsWith("/certification")) return certification()
    if (path.includes("/api/auth/me")) {
      return json({ id: "u1", name: "Reader", email: null, roles: [], permissions, tenants: [] })
    }
    if (path === "/api/catalog/silver.orders") return json({ ...ASSET, ...asset })
    return json({ error: "not stubbed" }, 404)
  }) as unknown as typeof fetch)
}

function renderPage() {
  return render(
    <AuthProvider>
      <AssetDetailPage />
    </AuthProvider>
  )
}

describe("AssetDetailPage certification", () => {
  it.each([
    ["certified", "Certified"],
    ["deprecated", "Deprecated"],
  ])("shows the %s mark in the header", async (certification, label) => {
    stubApi({ certification })
    renderPage()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
    expect(screen.getByText(label)).toBeTruthy()
  })

  it("shows no mark for an asset that has none", async () => {
    stubApi({})
    renderPage()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
    await waitFor(() => expect(screen.queryByText("Certified")).toBeNull())
    expect(screen.queryByText("Deprecated")).toBeNull()
  })
})

/** The `[method, path, body]` of every write the page sent. */
function writes(fetchSpy: ReturnType<typeof stubApi>) {
  return fetchSpy.mock.calls
    .filter((c) => (c[1]?.method ?? "GET") !== "GET")
    .map((c) => [c[1]?.method, String(c[0]), c[1]?.body ? JSON.parse(String(c[1].body)) : undefined])
}

const GOVERNOR = ["catalog:read", "governance:write"]

describe("AssetDetailPage deprecation notice", () => {
  it("says the table is deprecated, with the note and a link to the replacement", async () => {
    stubApi({
      certification: "deprecated",
      certificationNote: "Superseded by the monthly mart.",
      replacementAssetId: "serving.monthly_orders",
    })
    renderPage()
    const notice = await screen.findByRole("note")
    expect(notice.textContent).toContain("This table is deprecated.")
    expect(notice.textContent).toContain("Superseded by the monthly mart.")
    const link = screen.getByRole("link", { name: "serving.monthly_orders" })
    expect(link.getAttribute("href")).toBe("/data/assets/serving.monthly_orders")
  })

  // DATA-12 review SHOULD-FIX 1: the note is its own sentence.
  it("adds a full stop to a note that has none, so it does not run into the next sentence", async () => {
    stubApi({
      certification: "deprecated",
      certificationNote: "Old load, use the customers table",
      replacementAssetId: "serving.monthly_orders",
    })
    renderPage()
    const notice = await screen.findByRole("note")
    expect(notice.textContent).toBe(
      "This table is deprecated. Old load, use the customers table. Use serving.monthly_orders instead.",
    )
  })

  it("does not double the full stop of a note that ends a sentence", async () => {
    stubApi({
      certification: "deprecated",
      certificationNote: "Superseded!",
      replacementAssetId: "serving.monthly_orders",
    })
    renderPage()
    const notice = await screen.findByRole("note")
    expect(notice.textContent).toContain("Superseded! Use")
    expect(notice.textContent).not.toContain("!.")
  })

  it("names the replacement when the API sent its name, with the id as the link title", async () => {
    stubApi({
      certification: "deprecated",
      replacementAssetId: "serving.monthly_orders",
      replacementName: "Monthly orders",
    })
    renderPage()
    const link = await screen.findByRole("link", { name: "Monthly orders" })
    expect(link.getAttribute("href")).toBe("/data/assets/serving.monthly_orders")
    expect(link.getAttribute("title")).toBe("serving.monthly_orders")
  })

  it("shows no notice for a certified table or one without a mark", async () => {
    stubApi({ certification: "certified" })
    renderPage()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
    expect(screen.queryByRole("note")).toBeNull()
  })
})

describe("AssetDetailPage certification control", () => {
  it("offers no control without governance:write", async () => {
    stubApi({}, ["catalog:read", "catalog:write"])
    renderPage()
    expect(await screen.findByText("Orders Silver")).toBeTruthy()
    // The permission list arrives after the asset; wait for it to settle.
    await waitFor(() => expect(screen.queryByRole("button", { name: "Certification" })).toBeNull())
  })

  it("sends the note and the replacement when the mark is deprecated, then reloads the asset", async () => {
    const fetchSpy = stubApi({}, GOVERNOR)
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Certification" }))
    fireEvent.click(screen.getByLabelText("Deprecated"))
    fireEvent.change(screen.getByLabelText("Note"), { target: { value: " Superseded " } })
    fireEvent.change(screen.getByLabelText("Replacement asset id"), {
      target: { value: "serving.monthly_orders" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => expect(writes(fetchSpy)).toHaveLength(1))
    expect(writes(fetchSpy)).toEqual([
      [
        "PUT",
        "/api/catalog/silver.orders/certification",
        { status: "deprecated", note: "Superseded", replacementAssetId: "serving.monthly_orders" },
      ],
    ])
    const loads = () => fetchSpy.mock.calls.filter((c) => String(c[0]) === "/api/catalog/silver.orders")
    await waitFor(() => expect(loads().length).toBeGreaterThan(1))
  })

  it("sends status null when the choice is no mark", async () => {
    const fetchSpy = stubApi({ certification: "certified" }, GOVERNOR)
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Certification" }))
    // The dialog opens on the saved mark.
    expect((screen.getByLabelText("Certified") as HTMLInputElement).checked).toBe(true)
    fireEvent.click(screen.getByLabelText("No mark"))
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(writes(fetchSpy)).toHaveLength(1))
    expect(writes(fetchSpy)).toEqual([["PUT", "/api/catalog/silver.orders/certification", { status: null }]])
  })

  it("shows the API's sentence and keeps the dialog open on a refusal", async () => {
    stubApi({}, GOVERNOR, () => json({ error: "the replacement is itself deprecated" }, 400))
    renderPage()
    fireEvent.click(await screen.findByRole("button", { name: "Certification" }))
    fireEvent.click(screen.getByLabelText("Deprecated"))
    fireEvent.change(screen.getByLabelText("Replacement asset id"), { target: { value: "serving.old" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    expect(await screen.findByText("the replacement is itself deprecated")).toBeTruthy()
    expect(screen.getByLabelText("Replacement asset id")).toBeTruthy()
  })
})

