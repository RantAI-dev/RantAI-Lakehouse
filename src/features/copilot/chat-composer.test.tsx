// The chat box is back to its pre-#568 presentation (UX-4 decision 4): no
// visible "Message" label, no "* Required" line, no tooltip, no hidden
// requirement text. What must survive is behaviour: the textarea/input keep
// `aria-required` (UX-4 decision 3), the animated example still sits inside
// a `relative` containing block with only the textarea and the overlay, and
// the send button stays disabled until something is typed.
import { fireEvent, render, screen, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"

import { ChatComposer, MAX_TABLE_MENTIONS } from "./chat-composer"

const base = {
  mode: "ask" as const,
  setMode: mock(),
  onSend: mock(),
  busy: false,
}

describe("ChatComposer after the required marks were removed (UX-4)", () => {
  it("keeps the animated example inside a relative wrapper with the textarea", () => {
    const { container } = render(<ChatComposer {...base} examples={["one example"]} />)
    const view = within(container)
    const textarea = view.getByLabelText("Message AI Copilot")
    expect(textarea.getAttribute("aria-required")).toBe("true")
    const wrapper = textarea.parentElement as HTMLElement

    // The absolutely positioned overlay needs a positioned ancestor, and
    // that wrapper contains only the textarea and the overlay.
    expect(wrapper.className).toContain("relative")
    const overlay = wrapper.querySelector(".absolute")
    expect(overlay).not.toBeNull()
    expect(overlay?.parentElement).toBe(wrapper)
    expect(wrapper.querySelector("label")).toBeNull()
    // No visible label, no legend.
    expect(container.querySelector('label[for="copilot-message"]')).toBeNull()
    expect(view.queryByText(/Required/)).toBeNull()
  })

  it("keeps the compact pill label-free but still required", () => {
    const { container } = render(<ChatComposer {...base} compact />)
    const view = within(container)
    const input = view.getByLabelText("Message AI Copilot")
    expect(input.getAttribute("aria-required")).toBe("true")
    // No visible "Message" label, no tooltip, no hidden requirement text.
    expect(container.querySelector('label[for="copilot-compact-message"]')).toBeNull()
    expect(input.getAttribute("aria-describedby")).toBeNull()
    expect(view.queryByText(/message is required/i)).toBeNull()
    expect(view.queryByText(/Required/)).toBeNull()
  })
})

afterEach(() => mock.restore())

/** Six tables the picker can offer, so the sixth pick can be refused. */
const TABLES = Array.from({ length: 6 }, (_, i) => ({
  id: `t${i + 1}`,
  name: `Table ${i + 1}`,
  type: "table",
  layer: "gold",
  tableKey: `serving.table_${i + 1}`,
}))

function stubCatalog(body: unknown, status = 200) {
  return spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
    if (String(input).startsWith("/api/catalog")) {
      return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
    }
    return new Response("not stubbed", { status: 404 })
  }) as unknown as typeof fetch)
}

async function openPicker() {
  fireEvent.change(screen.getByLabelText("Message AI Copilot"), { target: { value: "@" } })
  return screen.findByRole("dialog", { name: "Mention a table" })
}

describe("ChatComposer table mentions (AI-26)", () => {
  it("opens the catalog picker when the person types @", async () => {
    stubCatalog({ assets: TABLES.slice(0, 1), namespaces: [] })
    render(<ChatComposer {...base} />)
    expect(screen.queryByRole("dialog", { name: "Mention a table" })).toBeNull()

    const picker = await openPicker()
    expect(within(picker).getByLabelText("Search tables")).toBeTruthy()
  })

  it("refuses a sixth pick and sends only the first five mentions", async () => {
    stubCatalog({ assets: TABLES, namespaces: [] })
    const onSend = mock()
    render(<ChatComposer {...base} onSend={onSend} />)
    const picker = await openPicker()
    fireEvent.change(within(picker).getByLabelText("Search tables"), { target: { value: "table" } })
    for (const table of TABLES) {
      fireEvent.click(await within(picker).findByRole("button", { name: `Mention ${table.name}` }))
    }

    expect(screen.getAllByRole("button", { name: /^Remove mention of / })).toHaveLength(MAX_TABLE_MENTIONS)
    fireEvent.change(screen.getByLabelText("Message AI Copilot"), { target: { value: "What columns do they have?" } })
    fireEvent.click(screen.getByRole("button", { name: "Send" }))
    expect(onSend).toHaveBeenCalledWith("What columns do they have?", TABLES.slice(0, MAX_TABLE_MENTIONS).map((t) => ({
      kind: "table",
      key: t.tableKey,
    })))
  })

  it("does not offer a Bronze asset in the catalog response", async () => {
    const bronze = {
      id: "bronze.raw_orders",
      name: "Bronze raw orders",
      type: "iceberg-table",
      layer: "bronze",
      tableKey: "bronze.raw_orders",
    }
    stubCatalog({ assets: [bronze], namespaces: [] })
    render(<ChatComposer {...base} />)
    const picker = await openPicker()
    fireEvent.change(within(picker).getByLabelText("Search tables"), { target: { value: "raw_orders" } })

    expect(await within(picker).findByText(/No tables matched/)).toBeTruthy()
    expect(within(picker).queryByRole("button", { name: /^Mention / })).toBeNull()
  })

  it("offers a Silver and a Gold asset and sends the picked key unchanged", async () => {
    const silver = {
      id: "silver.orders",
      name: "Silver orders",
      type: "iceberg-table",
      layer: "silver",
    }
    const gold = {
      id: "gold.daily",
      name: "Gold daily",
      type: "table",
      layer: "gold",
    }
    stubCatalog({ assets: [silver, gold], namespaces: [] })
    const onSend = mock()
    render(<ChatComposer {...base} onSend={onSend} />)
    const picker = await openPicker()
    fireEvent.change(within(picker).getByLabelText("Search tables"), { target: { value: "orders" } })

    // Neither asset carries a tableKey, so the sent key is the id itself.
    fireEvent.click(await within(picker).findByRole("button", { name: "Mention Silver orders" }))
    fireEvent.click(await within(picker).findByRole("button", { name: "Mention Gold daily" }))
    fireEvent.change(screen.getByLabelText("Message AI Copilot"), { target: { value: "What columns do they have?" } })
    fireEvent.click(screen.getByRole("button", { name: "Send" }))
    expect(onSend).toHaveBeenCalledWith("What columns do they have?", [
      { kind: "table", key: "silver.orders" },
      { kind: "table", key: "gold.daily" },
    ])
  })

  it("says so and offers no tables when the catalog search is refused", async () => {
    stubCatalog({ supported: false, reason: "The tenant search index is not available." })
    render(<ChatComposer {...base} />)
    const picker = await openPicker()
    fireEvent.change(within(picker).getByLabelText("Search tables"), { target: { value: "orders" } })

    expect(await within(picker).findByRole("alert")).toBeTruthy()
    expect(within(picker).getByText(/tenant search index is not available/)).toBeTruthy()
    expect(within(picker).queryByRole("button", { name: /^Mention / })).toBeNull()
  })
})
