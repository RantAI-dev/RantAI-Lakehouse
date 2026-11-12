// WEB-8 D8: the table picker and its columns. The forms that use it
// (`quality-rule-create-page.test.tsx`, the Data Quality tabs, the alert rule
// page) prove the wiring; this pins the picker's own contract — the options
// come from the catalog, a held table the catalog does not list is kept, and
// a failed read goes back to free text with the reason instead of an empty
// list that would read as "there are no tables".
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { assetService } from "@/services"
import type { Asset, AssetDetail } from "@/services/contracts/assets"
import { TablePicker, tableKey, useTableCatalog, useTableColumns } from "./table-picker"

afterEach(() => {
  cleanup()
  mock.restore()
})

// cmdk scrolls the selected item into view; jsdom/happy-dom has no such method.
Element.prototype.scrollIntoView = Element.prototype.scrollIntoView ?? (() => {})

const ASSETS = [
  { id: "a-orders", name: "orders", namespace: "silver" },
  { id: "a-raw", name: "raw_orders", namespace: "bronze" },
] as unknown as Asset[]

function detailOf(asset: Asset): AssetDetail {
  return {
    ...asset,
    schema: [{ name: "id", dataType: "bigint" }, { name: "amount", dataType: "double" }],
    sample: [],
    qualityChecks: [],
    policySummary: [],
    usage: null,
  } as unknown as AssetDetail
}

/** The hook's answer, rendered into a picker. */
function CatalogPicker({
  value = "",
  allowEveryTable = false,
  onChange = () => {},
}: {
  value?: string
  allowEveryTable?: boolean
  onChange?: (next: string) => void
}) {
  const catalog = useTableCatalog()
  return (
    <TablePicker
      id="table"
      label="Table"
      value={value}
      onChange={onChange}
      catalog={catalog}
      allowEveryTable={allowEveryTable}
    />
  )
}

/** A component that shows what `useTableColumns` answered. */
function ColumnsOf({ id }: { id: string | null }) {
  const columns = useTableColumns(id)
  return <p data-testid="columns">{columns === null ? "none" : columns.join(",")}</p>
}

describe("tableKey", () => {
  it("names a table the way every governance form sends it", () => {
    expect(tableKey({ namespace: "silver", name: "orders" })).toBe("silver.orders")
  })
})

describe("TablePicker", () => {
  it("offers the catalog's tables as namespace.table and picks one", async () => {
    spyOn(assetService, "listAssets").mockResolvedValue(ASSETS)
    const onChange = mock<(next: string) => void>(() => {})
    render(<CatalogPicker onChange={onChange} />)

    const trigger = await screen.findByRole("button", { name: "Table" })
    expect(trigger.textContent).toContain("Pick a table")
    fireEvent.click(trigger)
    const options = await screen.findAllByRole("option")
    expect(options.map((option) => option.textContent)).toEqual(["bronze.raw_orders", "silver.orders"])
    fireEvent.click(options[1] as HTMLElement)
    expect(onChange.mock.calls[0][0]).toBe("silver.orders")
  })

  it("keeps a table the catalog does not list, marked, rather than dropping it", async () => {
    spyOn(assetService, "listAssets").mockResolvedValue(ASSETS)
    render(<CatalogPicker value="serving.old_orders" />)

    const trigger = await screen.findByRole("button", { name: "Table" })
    expect(trigger.textContent).toContain("serving.old_orders (not in the catalog)")
    fireEvent.click(trigger)
    expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toContain(
      "serving.old_orders (not in the catalog)"
    )
  })

  it("offers * as Every table when the rule accepts every table", async () => {
    spyOn(assetService, "listAssets").mockResolvedValue(ASSETS)
    render(<CatalogPicker allowEveryTable />)
    fireEvent.click(await screen.findByRole("button", { name: "Table" }))
    expect((await screen.findAllByRole("option")).map((option) => option.textContent)).toEqual([
      "Every table",
      "bronze.raw_orders",
      "silver.orders",
    ])
  })

  it("is free text, with the reason, when the catalog cannot be read", async () => {
    spyOn(assetService, "listAssets").mockRejectedValue(new Error("catalog unavailable"))
    render(<CatalogPicker />)
    const input = (await screen.findByRole("textbox", { name: "Table" })) as HTMLInputElement
    expect(input.value).toBe("")
    expect(screen.getByText(/Could not read the catalog's tables/)).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Table" })).toBeNull()
  })
})

describe("useTableColumns", () => {
  it("reads the picked table's own schema, and answers none for a table the catalog does not list", async () => {
    spyOn(assetService, "getAsset").mockImplementation(async (id: string) =>
      id === "a-orders" ? detailOf(ASSETS[0] as Asset) : Promise.reject(new Error("not found"))
    )
    render(
      <>
        <ColumnsOf id="a-orders" />
        <ColumnsOf id={null} />
      </>
    )
    await waitFor(() => expect(screen.getAllByTestId("columns").map((p) => p.textContent)).toEqual(["id,amount", "none"]))
  })
})
