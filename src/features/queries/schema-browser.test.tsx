// The schema browser beside the editor: what a person sees when they expand a
// table, what a click puts in the buffer, and what the panel says when the
// catalog refuses. It renders the browser together with the real editor and
// the real hook, because "click inserts at the cursor" is a claim about the
// editor's buffer, not about a callback. The services are stubbed; nothing
// here reaches the network.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { EditorView } from "@codemirror/view"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { NO_READABLE_TABLE } from "@/lib/asset-query"
import { assetService, lakehouseService, queryService } from "@/services"
import { ServiceError } from "@/services/errors"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
import { SchemaBrowser } from "./schema-browser"
import { SqlPanel } from "./sql-panel"
import { useQueryStudio } from "./use-query-studio"

afterEach(() => {
  cleanup()
  mock.restore()
  window.localStorage.clear()
  visit("/query-studio")
})

/** Points the browser at a Query Studio address, the way an href does. */
function visit(path: string) {
  const url = new URL(path, "http://localhost:3000").href
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(url)
}

/** Opens the studio on a Trino tab, the way an asset's "Open in Query Studio" link does. */
function visitTrinoTab() {
  visit("/query-studio?sql=SELECT%20*%20FROM%20&engine=trino")
}

const BRONZE_NEEDS_TRINO = "Bronze tables are read with Trino: switch this tab to Trino to insert it."

/**
 * What the browser says about a Bronze table whose Iceberg table read
 * failed: the read failed, not the table.
 */
const TABLE_PROOF_FAILED = "The table could not be read."

/**
 * A Bronze table's detail as the API sends it: the registry slug `demo_orders`
 * is its id, `queryTarget` is the ClickHouse `DataLakeCatalog` name (never a
 * Trino one), and `tableName` is what the Trino name is built from.
 */
function bronzeDetail(tableName = "orders") {
  return {
    schema: [{ name: "oid", dataType: "String" }],
    layer: "raw",
    type: "iceberg-table",
    tableName,
    namespace: "bronze",
    name: "Demo Orders",
    queryTarget: { engine: "clickhouse", table: `icecat_api.\`bronze.${tableName}\`` },
  } as never
}

const ASSETS = [
  { id: "silver.orders", name: "Orders", namespace: "silver", layer: "silver" },
  { id: "serving.mart_wisman", name: "Mart Wisman", namespace: "serving", layer: "gold" },
  { id: "demo_orders", name: "Demo Orders", namespace: "bronze", layer: "raw" },
] as never[]

/** The Iceberg table the studio reads to prove a Bronze table is there. */
const ICEBERG_TABLE = {
  schema: [],
  partitionSpec: [],
  properties: {},
  snapshots: [],
  stats: {
    fileCount: 1,
    smallFileCount: null,
    smallFileThresholdBytes: null,
    recordCount: 1,
    totalBytes: 1,
    snapshotCount: 0,
    metadataLogCount: 0,
  },
} as unknown as LakehouseTableDetail

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
  const getAsset = spyOn(assetService, "getAsset").mockImplementation((async (id: string) => {
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
  // The Iceberg table the studio reads to learn whether a Bronze table is
  // there at all (QS-4 D3): the same read the catalog page makes.
  const getTableDetail = spyOn(lakehouseService, "getTableDetail").mockResolvedValue(ICEBERG_TABLE)
  return { getAsset, getTableDetail }
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
    const { getAsset } = stubServices()
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
    const { getAsset } = stubServices()
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

  it("says a Bronze table is read with Trino, and inserts nothing, on a ClickHouse tab before its detail loads", async () => {
    stubServices()
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    const row = await screen.findByRole("button", { name: "Demo Orders" })
    fireEvent.click(row)

    expect(view.state.doc.toString()).toBe("SELECT * FROM ")
    expect(row.getAttribute("title")).toBe(BRONZE_NEEDS_TRINO)
  })

  it("keeps saying so on a ClickHouse tab once the detail has loaded, and offers no ClickHouse name", async () => {
    const { getAsset } = stubServices()
    getAsset.mockResolvedValueOnce(bronzeDetail())
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))

    // The detail landed: its column is listed.
    expect(await screen.findByText("oid")).toBeTruthy()
    const row = screen.getByRole("button", { name: "Demo Orders" })
    fireEvent.click(row)
    expect(row.getAttribute("title")).toBe(BRONZE_NEEDS_TRINO)
    expect(screen.queryByRole("button", { name: /^Insert (iceberg|icecat_api)/ })).toBeNull()
    expect(view.state.doc.toString()).toBe("SELECT * FROM ")
  })

  it("inserts a Bronze table's Trino name on a Trino tab once its detail names the table", async () => {
    visitTrinoTab()
    const { getAsset } = stubServices()
    getAsset.mockResolvedValueOnce(bronzeDetail())
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))

    const insert = await screen.findByRole("button", { name: "Insert iceberg.bronze.orders" })
    fireEvent.click(insert)

    expect(view.state.doc.toString()).toBe("SELECT * FROM iceberg.bronze.orders")
    await act(async () => {})
  })

  it("keeps a Bronze table out of the editor until its Iceberg table is proven", async () => {
    visitTrinoTab()
    const { getAsset, getTableDetail } = stubServices()
    getAsset.mockResolvedValueOnce(bronzeDetail())
    // The read that proves the table is there never comes back.
    getTableDetail.mockReturnValue(new Promise<never>(() => {}))
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))

    // The detail is there, but the table itself is not proven: the entry
    // offers nothing.
    expect(await screen.findByText("oid")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /^Insert iceberg/ }) === null).toBe(true)
    const row = screen.getByRole("button", { name: "Demo Orders" })
    expect(row.getAttribute("aria-disabled")).toBe("true")
    expect(view.state.doc.toString()).toBe("SELECT * FROM ")
  })

  it("disables a Bronze table with the sentence when its Iceberg table is not there", async () => {
    visitTrinoTab()
    const { getAsset, getTableDetail } = stubServices()
    getAsset.mockResolvedValueOnce(bronzeDetail())
    getTableDetail.mockRejectedValue(new ServiceError("not_found", "table not found", 404))
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))

    await waitFor(() => {
      const row = screen.getByRole("button", { name: "Demo Orders" })
      expect(row.getAttribute("title")).toBe(NO_READABLE_TABLE)
    })
    expect(screen.queryByRole("button", { name: /^Insert iceberg/ }) === null).toBe(true)
    expect(view.state.doc.toString()).toBe("SELECT * FROM ")
  })

  it("says the table could not be read, with retry, when the proof read fails, and reads it again on the next open", async () => {
    visitTrinoTab()
    const { getAsset, getTableDetail } = stubServices()
    getAsset.mockResolvedValue(bronzeDetail())
    // A 5xx says nothing about whether the table is there; the read itself
    // failed (#448 review SHOULD-FIX).
    getTableDetail.mockRejectedValue(new ServiceError("unavailable", "warehouse down", 503))
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))

    // The entry holds the load-failed sentence, never the no-table one,
    // with a Retry affordance.
    expect(await screen.findByText(TABLE_PROOF_FAILED)).toBeTruthy()
    expect(screen.queryByText(NO_READABLE_TABLE)).toBeNull()
    const row = screen.getByRole("button", { name: "Demo Orders" })
    expect(row.getAttribute("title")).toBe(TABLE_PROOF_FAILED)
    const retry = screen.getByRole("button", {
      name: "Retry reading the table of Demo Orders",
    })

    // Retry reads the table again.
    const afterFirstRead = getTableDetail.mock.calls.length
    fireEvent.click(retry)
    await waitFor(() => expect(getTableDetail.mock.calls.length).toBeGreaterThan(afterFirstRead))

    // And so does the next open of the entry.
    const afterRetry = getTableDetail.mock.calls.length
    fireEvent.click(screen.getByRole("button", { name: "Columns of Demo Orders" }))
    fireEvent.click(screen.getByRole("button", { name: "Columns of Demo Orders" }))
    await waitFor(() => expect(getTableDetail.mock.calls.length).toBeGreaterThan(afterRetry))
  })

  it("inserts a quoted Trino name as it is formed, without quoting its parts again", async () => {
    visitTrinoTab()
    const { getAsset } = stubServices()
    getAsset.mockResolvedValueOnce(bronzeDetail("order lines"))
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))
    fireEvent.click(await screen.findByRole("button", { name: 'Insert iceberg.bronze."order lines"' }))

    expect(view.state.doc.toString()).toBe('SELECT * FROM iceberg.bronze."order lines"')
    await act(async () => {})
  })

  it("keeps the table's name unknown on a Trino tab until its detail loads", async () => {
    visitTrinoTab()
    stubServices()
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    const row = await screen.findByRole("button", { name: "Demo Orders" })
    fireEvent.click(row)

    expect(view.state.doc.toString()).toBe("SELECT * FROM ")
    expect(row.getAttribute("title") ?? "").toContain("SQL name")
  })

  // LH-QS-4-08: a Bronze row is proven when the list
  // loads, and its columns follow the row.
  it("offers a Bronze table's Trino name on a Trino tab without expanding it", async () => {
    visitTrinoTab()
    const { getAsset, getTableDetail } = stubServices()
    getAsset.mockImplementation((async (id: string) =>
      id === "demo_orders" ? bronzeDetail() : { schema: [] }) as never)
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Insert iceberg.bronze.orders" }))

    expect(view.state.doc.toString()).toBe("SELECT * FROM iceberg.bronze.orders")
    expect(getTableDetail).toHaveBeenCalledTimes(1)
    await act(async () => {})
  })

  it("disables a Bronze table's column buttons on a Trino tab while its Iceberg table is being proven", async () => {
    visitTrinoTab()
    const { getAsset, getTableDetail } = stubServices()
    getAsset.mockResolvedValue(bronzeDetail())
    // The read that proves the table is there never comes back.
    getTableDetail.mockReturnValue(new Promise<never>(() => {}))
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT * FROM ", "SELECT * FROM ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))

    const column = await screen.findByRole("button", { name: "Insert column oid" })
    expect((column as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(column)
    expect(view.state.doc.toString()).toBe("SELECT * FROM ")
  })

  it("lets a proven Bronze table's column be inserted on a Trino tab", async () => {
    visitTrinoTab()
    const { getAsset } = stubServices()
    getAsset.mockResolvedValue(bronzeDetail())
    const { container } = render(<Harness />)
    const view = editorView(container)
    setBuffer(view, "SELECT ", "SELECT ".length)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))
    const column = await screen.findByRole("button", { name: "Insert column oid" })
    await waitFor(() => expect((column as HTMLButtonElement).disabled).toBe(false))
    fireEvent.click(column)

    expect(view.state.doc.toString()).toBe("SELECT oid")
    await act(async () => {})
  })

  it("shows the no-table sentence and no columns for a Bronze table whose Iceberg table is not there", async () => {
    visitTrinoTab()
    const { getAsset, getTableDetail } = stubServices()
    getAsset.mockResolvedValue(bronzeDetail())
    getTableDetail.mockRejectedValue(new ServiceError("not_found", "table not found", 404))
    render(<Harness />)

    fireEvent.click(await screen.findByRole("button", { name: "Columns of Demo Orders" }))

    expect(await screen.findByText(NO_READABLE_TABLE)).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Insert column oid" })).toBeNull()
    expect(screen.queryByText("oid")).toBeNull()
  })

  it("keeps a Bronze table's columns insertable on a ClickHouse tab, and reads no Iceberg table for the list", async () => {
    const { getAsset, getTableDetail } = stubServices()
    getAsset.mockResolvedValue(bronzeDetail())
    render(<Harness />)

    await screen.findByRole("button", { name: "Demo Orders" })
    await act(async () => {})
    expect(getTableDetail).toHaveBeenCalledTimes(0)
    expect(getAsset).toHaveBeenCalledTimes(0)

    fireEvent.click(screen.getByRole("button", { name: "Columns of Demo Orders" }))
    const column = await screen.findByRole("button", { name: "Insert column oid" })
    expect((column as HTMLButtonElement).disabled).toBe(false)
  })

  it("shows the catalog's reason when a table's columns were refused, and retries on request", async () => {
    const { getAsset } = stubServices()
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
