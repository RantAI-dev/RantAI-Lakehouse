// The rows table and column chips shared by the dashboard's SQL source and
// the pipeline editor's preview (BLD-1 part 1 S3-T3 moved them here). The
// dashboard's own words are the defaults; the editor passes its own.
import { afterEach, describe, expect, it } from "bun:test"
import { cleanup, render, screen } from "@testing-library/react"
import { SqlColumnChips, SqlRowsTable } from "./sql-rows-table"

afterEach(() => cleanup())

const RESULT = {
  columns: [
    { name: "id", type: "Int64" },
    { name: "name", type: "Nullable(String)" },
  ],
  rows: [
    { id: 1, name: "a" },
    { id: 2, name: null },
  ],
}

describe("SqlRowsTable", () => {
  it("shows the rows under their columns and says how many", () => {
    render(<SqlRowsTable result={RESULT} />)
    expect(screen.getByText("First 2 rows.")).toBeTruthy()
    expect(screen.getAllByRole("columnheader").map((th) => th.textContent)).toEqual(["id", "name"])
    expect(screen.getAllByRole("row")).toHaveLength(3)
  })

  it("says the rows are of an earlier edit when stale, in the dashboard's words by default", () => {
    render(<SqlRowsTable result={RESULT} stale />)
    expect(screen.getByText("First 2 rows. These are the rows of the SQL before your last edit.")).toBeTruthy()
  })

  it("takes the caller's words for no rows and for stale", () => {
    render(
      <SqlRowsTable
        result={{ columns: RESULT.columns, rows: [] }}
        stale
        noRowsText="It returned no rows."
        staleText=" From before your last edit."
      />
    )
    expect(screen.getByText("It returned no rows. From before your last edit.")).toBeTruthy()
    expect(screen.queryByRole("table")).toBeNull()
  })
})

describe("SqlColumnChips", () => {
  it("lists each column with its type, and says when there are no rows", () => {
    render(<SqlColumnChips result={{ columns: RESULT.columns, rows: [] }} />)
    expect(screen.getByLabelText("Columns").textContent).toBe("id Int64name Nullable(String)")
    expect(screen.getByText("The SQL ran and returned no rows.")).toBeTruthy()
  })
})
