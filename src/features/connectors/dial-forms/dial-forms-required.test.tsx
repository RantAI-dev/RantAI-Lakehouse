import * as React from "react"
import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"

import type { RestDial } from "@/services/contracts/connectors"
import { CdcDialForm } from "./cdc-dial-form"
import { FilesDialForm } from "./files-dial-form"
import { OracleDialForm } from "./oracle-dial-form"
import { RestDialForm } from "./rest-dial-form"
import { SqlDialForm } from "./sql-dial-form"

/** Whether the field's visible label opted in to the shared marker (UX-1). */
function isMarked(name: string | RegExp): boolean {
  const control = screen.getByLabelText(name)
  const id = control.getAttribute("id")
  const label = id ? document.querySelector(`label[for="${id}"]`) : null
  return label?.getAttribute("data-required") === ""
}

function hasNoMarker(name: string | RegExp): boolean {
  const control = screen.getByLabelText(name)
  const id = control.getAttribute("id")
  const label = id ? document.querySelector(`label[for="${id}"]`) : null
  return !label?.hasAttribute("data-required")
}

describe("dial form required indicators (UX-1)", () => {
  it("marks the supplied SQL fields but leaves the defaulted driver and optional TLS mode unmarked", () => {
    render(<SqlDialForm value={null} onChange={mock()} />)
    expect(isMarked("Host")).toBe(true)
    expect(isMarked("Port")).toBe(true)
    expect(isMarked("Database")).toBe(true)
    expect(isMarked("User")).toBe(true)
    expect(screen.getByLabelText("Port").getAttribute("aria-required")).toBe("true")
    expect(hasNoMarker("SSL mode (optional)")).toBe(true)
    expect(screen.getByText(/Required/)).toBeDefined()
  })

  it("marks CDC slot and publication but not the optional server id", () => {
    render(<CdcDialForm value={null} onChange={mock()} />)
    expect(isMarked("Replication slot name")).toBe(true)
    expect(isMarked("Publication name")).toBe(true)
    expect(screen.getByLabelText("Replication slot name").getAttribute("aria-required")).toBe("true")
    expect(hasNoMarker("Server id (optional)")).toBe(true)
  })

  it("makes the Oracle TLS DN conditional on a TLS sslMode", () => {
    const props = (sslMode: string | null) => ({
      value: { driver: "oracle" as const, host: "h", port: 1521, database: "d", user: "u", sslMode, sslServerCertDn: null },
      onChange: mock(),
    })
    const { rerender } = render(<OracleDialForm {...props(null)} />)
    expect(hasNoMarker(/TLS server certificate DN/)).toBe(true)
    rerender(<OracleDialForm {...props("require")} />)
    expect(isMarked(/TLS server certificate DN/)).toBe(true)
    expect(screen.getByLabelText(/TLS server certificate DN/).getAttribute("aria-required")).toBe("true")
    rerender(<OracleDialForm {...props("disable")} />)
    expect(hasNoMarker(/TLS server certificate DN/)).toBe(true)
  })

  it("marks only the auth/pagination field selected by their modes", () => {
    function RestHarness() {
      const [value, setValue] = React.useState<RestDial | null>(null)
      return <RestDialForm value={value} onChange={setValue} />
    }
    render(<RestHarness />)
    // Defaults: api_key header and no pagination fields.
    expect(isMarked("Header name")).toBe(true)
    expect(screen.queryByLabelText("Token URL")).toBeNull()
    expect(screen.queryByLabelText("Page query parameter")).toBeNull()
    expect(isMarked("Path")).toBe(true)
    expect(hasNoMarker("Records path (optional)")).toBe(true)

    fireEvent.change(screen.getByLabelText("Auth type"), { target: { value: "oauth2_client_credentials" } })
    expect(screen.queryByLabelText("Header name")).toBeNull()
    expect(isMarked("Token URL")).toBe(true)

    fireEvent.change(screen.getByLabelText("Auth type"), { target: { value: "bearer" } })
    expect(screen.queryByLabelText("Token URL")).toBeNull()
    expect(screen.queryByLabelText("Header name")).toBeNull()

    fireEvent.change(screen.getByLabelText("Pagination"), { target: { value: "cursor" } })
    expect(isMarked("Cursor field (JSON path)")).toBe(true)
    fireEvent.change(screen.getByLabelText("Pagination"), { target: { value: "page" } })
    expect(screen.queryByLabelText("Cursor field (JSON path)")).toBeNull()
    expect(isMarked("Page query parameter")).toBe(true)
  })

  it("marks the files bucket but not the optional/defaulted or Test-only fields", () => {
    render(<FilesDialForm value={null} onChange={mock()} />)
    expect(isMarked("Bucket / root")).toBe(true)
    for (const optional of ["Prefix (optional)", "Endpoint", "Region (optional)", "Format"]) {
      expect(hasNoMarker(optional)).toBe(true)
    }
  })
})
