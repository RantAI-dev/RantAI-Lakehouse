// The schema browser beside the editor: what a person sees when they expand a
// table, what a click puts in the buffer, and what the panel says when the
// catalog refuses. It renders the browser together with the real editor and
// the real hook, because "click inserts at the cursor" is a claim about the
// editor's buffer, not about a callback. The services are stubbed; nothing
// here reaches the network.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { EditorView } from "@codemirror/view"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { assetService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import { SchemaBrowser } from "./schema-browser"
import { SqlPanel } from "./sql-panel"
import { useQueryStudio } from "./use-query-studio"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
})

const ASSETS = [
  { id: "silver.orders", name: "Orders", namespace: "silver", layer: "silver" },
  { id: "serving.mart_wisman", name: "Mart Wisman", namespace: "serving", layer: "gold" },
  { id: "demo_orders", name: "Demo Orders", namespace: "bronze", layer: "raw" },
] as never[]

/** The page's two halves that matter here: the editor with its toolbar, and the browser. */
function Harness() {
  const studio = useQueryStudio()
  return (
    <>
      <SqlPanel studio={studio} onSave={() => {}} />
      <SchemaBrowser studio={studio} onClose={() => {}} />
    </>
  )
}

function stubServices(options: { assets?: "ok" | "forbidden" } = {}) {
  spyOn(queryService, "listSaved").mockResolvedValue([])
  spyOn(queryService, "listHistory").mockResolvedValue([])
  spyOn(queryService, "estimate").mockRejectedValue(new ServiceError("unavailable", "no estimate"))
  if (options.assets === "forbidden") {
    spyOn(assetService, "listAssets").mockRejectedValue(
      new ServiceError("permission_denied", "catalog:read", 403)
    )
  } else {
    spyOn(assetService, "listAssets").mockResolvedValue(ASSETS)
  }
  return spyOn(assetService, "getAsset").mockImplementation((async (id: string) => {
    if (id === "silver.orders") {
      return {
        schema: [
          { name: "amount", dataType: "Int64" },
          { name: "order total", dataType: "Decimal(10, 2)" },
        ],
      }
    }
    return { schema: [] }
  }) as never)
}

/** The editor's view, found the way the editor's own DOM exposes it. */
function editorView(container: HTMLElement): EditorView {
  const content = container.querySelector(".cm-content")
  const view = content ? EditorView.findFromDOM(content as HTMLElement) : null
  if (!view) throw new Error("the editor did not render")
  return view
}

/** Replaces the buffer and puts the cursor at `cursor`, as typing and a click would. */
function setBuffer(view: EditorView, text: string, cursor: number) {
  act(() => {
    view.dispatch({
      changes: { from: 0, to: view.state.doc.length, insert: text },
      selection: { anchor: cursor },
    })
  })
}

describe("SchemaBrowser", () => {
  it("shows the layers, and the columns with their types once a table is expanded", async () => {
    stubServices()
    render(<Harness />)

    expect(await screen.findByText("Silver")).toBeTruthy()
    expect(screen.getByText("Gold")).toBeTruthy()
    expect(screen.getByText("Bronze (raw)")).toBeTruthy()

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Orders" }))

    expect(await screen.findByText("amount")).toBeTruthy()
    expect(screen.getByText("Int64")).toBeTruthy()
    expect(screen.getByText("order total")).toBeTruthy()
    expect(screen.getByText("Decimal(10, 2)")).toBeTruthy()
  })

  it("fetches a table's columns once, however often it is collapsed and expanded", async () => {
    const getAsset = stubServices()
    render(<Harness />)

    const toggle = await screen.findByRole("button", { name: "Columns of Orders" })
    fireEvent.click(toggle)
    await screen.findByText("amount")
    fireEvent.click(toggle)
    fireEvent.click(toggle)

    expect(getAsset).toHaveBeenCalledTimes(1)
  })

  it("inserts a column's name at the cursor and leaves the cursor after it", async () => {
    stubServices()
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT  FROM tail", "SELECT ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Orders" }))
    fireEvent.click(await screen.findByRole("button", { name: "Insert column amount" }))

    expect(view.state.doc.toString()).toBe("SELECT amount FROM tail")
    expect(view.state.selection.main.head).toBe("SELECT amount".length)
  })

  it("quotes a column's name when it is not a plain identifier", async () => {
    stubServices()
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT  FROM tail", "SELECT ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Orders" }))
    fireEvent.click(await screen.findByRole("button", { name: 'Insert column "order total"' }))

    expect(view.state.doc.toString()).toBe('SELECT "order total" FROM tail')
  })

  it("inserts a table's SQL name at the cursor", async () => {
    const getAsset = stubServices()
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Insert serving.mart_wisman" }))

    expect(view.state.doc.toString()).toBe("SELECT * FROM serving.mart_wisman")
    // The statement now names the table, so its columns are fetched; let
    // that settle inside the test rather than after it.
    await waitFor(() => expect(getAsset).toHaveBeenCalledWith("serving.mart_wisman"))
    await act(async () => {})
  })

  it("does not insert a Bronze registry slug as if it were a table name", async () => {
    stubServices()
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    const row = await screen.findByRole("button", { name: "Demo Orders" })
    fireEvent.click(row)

    expect(view.state.doc.toString()).toBe("SELECT * FROM ")
    expect(row.getAttribute("title") ?? "").toContain("SQL name")
  })

  it("shows the catalog's reason when a table's columns were refused, and retries on request", async () => {
    const getAsset = stubServices()
    getAsset.mockResolvedValueOnce({
      supported: false,
      reason: "This tenant has no catalog configured",
    } as never)
    render(<Harness />)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Orders" }))
    expect(await screen.findByText(/This tenant has no catalog configured/)).toBeTruthy()

    fireEvent.click(screen.getByRole("button", { name: "Retry columns of Orders" }))

    expect(await screen.findByText("amount")).toBeTruthy()
    expect(getAsset).toHaveBeenCalledTimes(2)
  })

  it("says the browser needs catalog access when the catalog list is forbidden, and leaves the editor usable", async () => {
    stubServices({ assets: "forbidden" })
    const { container } = render(<Harness />)

    expect(await screen.findByText(/needs catalog access/i)).toBeTruthy()
    expect(screen.queryByText("Silver")).toBeNull()

    const view = editorView(container)
    setBuffer(view, "SELECT 1", "SELECT 1".length)
    expect(view.state.doc.toString()).toBe("SELECT 1")
    await waitFor(() => {
      const run = screen.getByRole("button", { name: "Run query" }) as HTMLButtonElement
      expect(run.disabled).toBe(false)
    })
  })
})
