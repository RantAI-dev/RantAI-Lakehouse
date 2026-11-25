import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock, spyOn } from "bun:test"
import { lakehouseService } from "@/services"
import type { LakehouseTableDetail } from "@/services/contracts/lakehouse"
import { IcebergTimeTravelControls } from "./time-travel-controls"

// DATA-16 F5/F7/F8: the picker writes the ClickHouse pin, says the pin is
// query-wide, shows a version's time (not just its id), and is switched off
// with a reason on an engine that has no past versions. DATA-21 D5: Trino
// has them too — the pin is written on the table the user picked.
afterEach(() => {
  cleanup()
  mock.restore()
})

const SQL = "SELECT * FROM icecat_api.`bronze.orders`"
/** The Trino form of the same read: the catalog, the schema, the table. */
const TRINO_SQL = "SELECT * FROM iceberg.bronze.orders"

function stubLakehouse() {
  spyOn(lakehouseService, "listNamespaces").mockResolvedValue([{ name: "bronze" }] as never)
  spyOn(lakehouseService, "listTables").mockResolvedValue([{ name: "orders" }] as never)
  spyOn(lakehouseService, "getTableDetail").mockResolvedValue({
    snapshots: [
      {
        id: "7539123456789012345",
        parentId: null,
        timestampMs: Date.parse("2026-09-23T03:37:39Z"),
        operation: "append",
        summary: { addedRecords: 1, deletedRecords: null, totalRecords: 1, totalDataFiles: 1 },
      },
    ],
  } as unknown as LakehouseTableDetail)
}

/** Opens a Radix select by its placeholder and picks the option whose text matches. */
async function pick(placeholder: string, option: RegExp) {
  const trigger = (await screen.findByText(placeholder)).closest("button") as HTMLElement
  fireEvent.mouseDown(trigger, { button: 0 })
  fireEvent.mouseUp(trigger, { button: 0 })
  fireEvent.click(trigger)
  fireEvent.click(await screen.findByText(option))
}

describe("IcebergTimeTravelControls", () => {
  it("on ClickHouse lists a version by its time and operation and pins it through pinSnapshot", async () => {
    stubLakehouse()
    const onApply = mock<(next: string, pinned: unknown) => void>(() => {})
    render(<IcebergTimeTravelControls sql={SQL} engine="clickhouse" onApply={onApply} />)

    expect(screen.getByText(/One version applies to every raw table in the query\./)).toBeTruthy()
    await pick("Choose a namespace", /^bronze$/)
    await pick("Choose a table", /^orders$/)
    await pick("Choose a version", /2026.* · append/)
    fireEvent.click(screen.getByText("Query this version"))

    await waitFor(() => expect(onApply).toHaveBeenCalledTimes(1))
    expect(onApply.mock.calls[0][0]).toBe(`${SQL}\nSETTINGS iceberg_snapshot_id = 7539123456789012345`)
  })

  it("on Trino pins the version on the table the user picked", async () => {
    stubLakehouse()
    const onApply = mock<(next: string, pinned: unknown) => void>(() => {})
    render(<IcebergTimeTravelControls sql={TRINO_SQL} engine="trino" onApply={onApply} />)

    expect(screen.getByText(/The version applies to the table you picked\./)).toBeTruthy()
    await pick("Choose a namespace", /^bronze$/)
    await pick("Choose a table", /^orders$/)
    await pick("Choose a version", /2026.* · append/)
    fireEvent.click(screen.getByText("Query this version"))

    await waitFor(() => expect(onApply).toHaveBeenCalledTimes(1))
    expect(onApply.mock.calls[0][0]).toBe(
      "SELECT * FROM iceberg.bronze.orders FOR VERSION AS OF 7539123456789012345"
    )
  })

  it("says so when the engine has no past versions", () => {
    stubLakehouse()
    const onApply = mock<(next: string, pinned: unknown) => void>(() => {})
    render(<IcebergTimeTravelControls sql={SQL} engine="duckdb" onApply={onApply} />)

    expect(screen.getByText("Past versions can be queried on ClickHouse and Trino.")).toBeTruthy()
    expect((screen.getByText("Query this version") as HTMLButtonElement).disabled).toBe(true)
    expect((screen.getByText("Choose a namespace").closest("button") as HTMLButtonElement).disabled).toBe(true)
    expect(onApply).not.toHaveBeenCalled()
  })
})

// UX-9: the pin reports what it pinned, for the chip by the editor, and a
// Trino statement that does not read the picked table says so.
describe("IcebergTimeTravelControls in the Time travel dialog", () => {
  it("hands the chip the table and the version's time with the pinned SQL", async () => {
    stubLakehouse()
    const onApply = mock<(next: string, pinned: unknown) => void>(() => {})
    render(<IcebergTimeTravelControls sql={SQL} engine="clickhouse" onApply={onApply} />)

    await pick("Choose a namespace", /^bronze$/)
    await pick("Choose a table", /^orders$/)
    await pick("Choose a version", /2026.* · append/)
    fireEvent.click(screen.getByText("Query this version"))

    await waitFor(() => expect(onApply).toHaveBeenCalledTimes(1))
    expect(onApply.mock.calls[0][1]).toEqual({
      snapshotId: "7539123456789012345",
      table: "bronze.orders",
      at: "2026-09-23T03:37:39.000Z",
    })
  })

  it("says the Trino query does not read the picked table instead of doing nothing", async () => {
    stubLakehouse()
    const onApply = mock<(next: string, pinned: unknown) => void>(() => {})
    render(<IcebergTimeTravelControls sql="SELECT 1" engine="trino" onApply={onApply} />)

    await pick("Choose a namespace", /^bronze$/)
    await pick("Choose a table", /^orders$/)
    await pick("Choose a version", /2026.* · append/)
    fireEvent.click(screen.getByText("Query this version"))

    expect(await screen.findByRole("alert")).toBeTruthy()
    expect(screen.getByRole("alert").textContent).toContain("does not read bronze.orders")
    expect(onApply).not.toHaveBeenCalled()
  })
})
