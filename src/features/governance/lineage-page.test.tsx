// A link names what to focus on (`/lineage?focus=<id>`, from an asset or a
// pipeline run); the page used to ignore it for a fixed demo id. The graph
// is drawn from what the API recorded, column by column, and a node traces
// again from itself.
const url = { search: "focus=pl-orders-clean-abc" }
mock.module("next/navigation", () => ({
  usePathname: () => "/lineage",
  useSearchParams: () => new URLSearchParams(url.search),
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
import { TableProviders } from "@/components/app-shell/table-providers"
import { LineagePage } from "./lineage-page"

afterEach(() => {
  cleanup()
  url.search = "focus=pl-orders-clean-abc"
})

const GRAPH = {
  focus: "pl-orders-clean-abc",
  focusIds: ["pl-orders-clean-abc"],
  nodes: [
    { id: "conn-nw:public.orders", label: "public.orders", kind: "source", sublabel: "Northwind", ref: "conn-nw", depth: 0 },
    { id: "bronze.orders", label: "bronze.orders", kind: "bronze", sublabel: null, ref: "orders", depth: 1 },
    { id: "pl-orders-clean-abc", label: "orders_clean", kind: "pipeline", sublabel: "ready", ref: "pl-orders-clean-abc", depth: 2 },
    { id: "silver.orders_clean", label: "silver.orders_clean", kind: "silver", sublabel: null, ref: "silver.orders_clean", depth: 3 },
  ],
  edges: [
    { id: "a", from: "conn-nw:public.orders", to: "bronze.orders", kind: "ingest" },
    { id: "b", from: "bronze.orders", to: "pl-orders-clean-abc", kind: "read" },
    { id: "c", from: "pl-orders-clean-abc", to: "silver.orders_clean", kind: "write" },
  ],
  columnMappings: [
    { source: "bronze.orders.amount", target: "silver.orders_clean.amount_usd", transform: "renamed" },
  ],
  supported: true,
  note: "Traced from connector ingest specs and pipelines built in the console.",
}

function json(body: unknown) {
  return new Response(JSON.stringify(body), { headers: { "Content-Type": "application/json" } })
}

function page() {
  return (
    <TableProviders>
      <LineagePage />
    </TableProviders>
  )
}

function renderPage() {
  return render(page())
}

describe("LineagePage", () => {
  it("asks for the lineage of the id the link names", async () => {
    const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(json(GRAPH))
    renderPage()

    await waitFor(() => expect(fetchSpy).toHaveBeenCalled())
    expect(String(fetchSpy.mock.calls[0]![0])).toContain("/api/governance/lineage?focus=pl-orders-clean-abc")
    expect((screen.getByLabelText("Focus asset id") as HTMLInputElement).value).toBe("pl-orders-clean-abc")
    fetchSpy.mockRestore()
  })

  it("draws the recorded graph, links each node, and traces again from a clicked node", async () => {
    // A fresh Response per call: a body can only be read once.
    const fetchSpy = spyOn(globalThis, "fetch").mockImplementation((async () => json(GRAPH)) as unknown as typeof fetch)
    renderPage()

    expect(await screen.findByRole("button", { name: "public.orders" })).toBeTruthy()
    expect(screen.getByText(GRAPH.note)).toBeTruthy()
    expect(screen.getByText(/silver\.orders_clean\.amount_usd/)).toBeTruthy()
    expect(screen.getByRole("link", { name: "Open orders_clean" }).getAttribute("href")).toBe(
      "/pipelines/pl-orders-clean-abc"
    )
    expect(screen.getByRole("link", { name: "Open public.orders" }).getAttribute("href")).toBe(
      "/connectors/conn-nw/edit"
    )
    expect(screen.getByRole("link", { name: "Open bronze.orders" }).getAttribute("href")).toBe("/data/assets/orders")

    fireEvent.click(screen.getByRole("button", { name: "bronze.orders" }))
    await waitFor(() =>
      expect(fetchSpy.mock.calls.some((c) => String(c[0]).endsWith("?focus=bronze.orders"))).toBe(true)
    )
    fetchSpy.mockRestore()
  })

  it("follows the URL when a link to this page drops the focus, e.g. the sidebar entry", async () => {
    const fetchSpy = spyOn(globalThis, "fetch").mockImplementation((async () => json(GRAPH)) as unknown as typeof fetch)
    const { rerender } = renderPage()
    await waitFor(() => expect(fetchSpy).toHaveBeenCalled())

    url.search = ""
    rerender(page())
    await waitFor(() => expect((screen.getByLabelText("Focus asset id") as HTMLInputElement).value).toBe(""))
    await waitFor(() =>
      expect(fetchSpy.mock.calls.some((c) => String(c[0]).endsWith("/api/governance/lineage?focus="))).toBe(true)
    )
    fetchSpy.mockRestore()
  })

  it("says nothing is recorded for an id instead of drawing an empty canvas", async () => {
    const fetchSpy = spyOn(globalThis, "fetch").mockResolvedValue(
      json({ ...GRAPH, focusIds: [], nodes: [], edges: [], columnMappings: [] })
    )
    renderPage()

    expect(await screen.findByText("No lineage recorded for pl-orders-clean-abc")).toBeTruthy()
    fetchSpy.mockRestore()
  })
})
