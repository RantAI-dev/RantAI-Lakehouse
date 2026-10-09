// The Data Explorer's columns (DATA-11 T3): the reason a search returned a
// row sits under its name, and the Owner and Tags columns filter through a
// `filters` parameter the API accepts (`routes/catalog_query.rs`: `tags` is
// filterable and refused as a sort or group field).
import { render, screen } from "@testing-library/react"
import type { ColumnDef } from "@tanstack/react-table"
import { describe, expect, it } from "bun:test"
import { getDefaultFilterOperator } from "@/lib/data-table"
import { filterParam } from "@/lib/table-filter-link"
import type { Asset } from "@/services/contracts/assets"
import { getDataExplorerColumns } from "./data-explorer-columns"

const columns = getDataExplorerColumns({ onOpen: () => {} })
const column = (id: string): ColumnDef<Asset> => {
  const found = columns.find((c) => c.id === id)
  if (!found) throw new Error(`no column ${id}`)
  return found
}

function cellOf(id: string, asset: Partial<Asset>) {
  const render = column(id).cell as (ctx: { row: { original: Asset } }) => React.ReactNode
  return render({ row: { original: asset as Asset } })
}

describe("Data Explorer columns", () => {
  it("shows why a search returned the row under its name", () => {
    render(
      <>
        {cellOf("name", {
          name: "Sales",
          matchedOn: { field: "column", value: "revenue_amount", approximate: false },
        })}
      </>,
    )
    expect(screen.getByText("Sales")).toBeTruthy()
    expect(screen.getByText("column revenue_amount")).toBeTruthy()
  })

  it("shows no reason line when the row came without one", () => {
    render(<>{cellOf("name", { name: "Sales" })}</>)
    expect(screen.getByText("Sales")).toBeTruthy()
    expect(screen.queryByText(/column|tag|description|approximate/)).toBeNull()
  })

  it("truncates a long owner and keeps the full value as its title", () => {
    // DATA-11 review SHOULD-FIX 7: the Owner cell may not widen the table.
    const owner = "a-very-long-owner-address@example.invalid"
    render(<>{cellOf("owner", { owner })}</>)
    const cell = screen.getByText(owner)
    expect(cell.getAttribute("title")).toBe(owner)
    expect(cell.className).toContain("truncate")
    expect(cell.className).toContain("max-w-")
  })

  it("shows each tag as a pill, and a dash when there are none", () => {
    render(<>{cellOf("tags", { tags: ["finance", "pii"] })}</>)
    expect(screen.getByText("finance")).toBeTruthy()
    expect(screen.getByText("pii")).toBeTruthy()
    render(<>{cellOf("tags", {})}</>)
    expect(screen.getByText("—")).toBeTruthy()
  })

  it("filters Owner and Tags as text and offers no sort on Tags", () => {
    expect(column("owner").meta).toMatchObject({ variant: "text" })
    expect(column("tags").meta).toMatchObject({ variant: "text" })
    // The server answers a sort on `tags` with a 400.
    expect(column("tags").enableSorting).toBe(false)
    // Group-by is opted into per column; neither new column opts in.
    expect((column("tags").meta as { enableGrouping?: boolean }).enableGrouping).toBeUndefined()
  })

  it("builds the tags filter parameter with iLike, the operator the text filter starts with", () => {
    const operator = getDefaultFilterOperator("text")
    expect(operator).toBe("iLike")
    const param = JSON.parse(
      filterParam([{ id: "tags", value: "fin", variant: "text", operator }]),
    )
    expect(param).toEqual([
      { id: "tags", value: "fin", variant: "text", operator: "iLike", filterId: "link0" },
    ])
  })
})
