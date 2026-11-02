import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorSetupChecks } from "./connector-setup-checks"

afterEach(cleanup)

describe("ConnectorSetupChecks", () => {
  it("renders nothing for no checks", () => {
    const { container } = render(<ConnectorSetupChecks checks={[]} />)
    expect(container.textContent).toBe("")
  })

  it("lists missing first, then unread, then passing, each with the API's words", () => {
    render(
      <ConnectorSetupChecks
        checks={[
          { setting: "wal_level", status: "ok", message: "wal_level is logical." },
          { setting: "SQL Server Agent", status: "unchecked", message: "Reading it needs VIEW SERVER STATE." },
          { setting: "publication", status: "missing", message: "The publication does not exist.", command: "CREATE PUBLICATION p;" },
        ]}
      />
    )
    const items = screen.getAllByRole("listitem")
    const settings = items.map((li) => li.querySelector("span.font-medium.text-sm")?.textContent)
    expect(settings).toEqual(["publication", "SQL Server Agent", "wal_level"])
    expect(screen.getByText("Missing")).toBeDefined()
    expect(screen.getByText("Not checked")).toBeDefined()
    expect(screen.getByText("OK")).toBeDefined()
    expect(screen.getByText(/1 setting missing/)).toBeDefined()
  })

  it("copies the command of a missing setting", () => {
    const writeText = mock(async () => {})
    Object.defineProperty(navigator, "clipboard", { value: { writeText }, configurable: true })
    render(
      <ConnectorSetupChecks
        checks={[{ setting: "wal_level", status: "missing", message: "m", command: "ALTER SYSTEM SET wal_level = 'logical';" }]}
      />
    )
    fireEvent.click(screen.getByRole("button", { name: "Copy the command for wal_level" }))
    expect(writeText).toHaveBeenCalledWith("ALTER SYSTEM SET wal_level = 'logical';")
  })

  it("says nothing is missing when every check passes", () => {
    render(<ConnectorSetupChecks checks={[{ setting: "a", status: "ok", message: "m" }]} />)
    expect(screen.getByText("Nothing is missing.")).toBeDefined()
  })
})
