import * as React from "react"
import { fireEvent, render, screen, within } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"

import type { KafkaDial, MongoDial, RestDial, SheetsDial } from "@/services/contracts/connectors"
import { CdcDialForm } from "./cdc-dial-form"
import { FilesDialForm } from "./files-dial-form"
import { KafkaDialForm } from "./kafka-dial-form"
import { MongoDialForm } from "./mongo-dial-form"
import { OracleDialForm } from "./oracle-dial-form"
import { RestDialForm } from "./rest-dial-form"
import { SheetsDialForm } from "./sheets-dial-form"
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

  // UX-1-R5: the store validates EVERY Mongo host / Kafka broker, and the
  // Sheets/REST adapters read EVERY range / endpoint at runtime. A second
  // rendered row must therefore be just as required as the first, with a
  // visible label rather than an sr-only or first-row-only marker.
  it("marks every rendered repeated row required with a visible associated label", () => {
    const labelFor = (container: HTMLElement, input: HTMLElement) =>
      container.querySelector(`label[for="${input.id}"]`)

    const mongo: MongoDial = {
      hosts: ["mongo-a.internal:27017", "mongo-b.internal:27017"],
      database: "d",
      username: "u",
      directConnection: true,
    }
    const mongoView = render(<MongoDialForm value={mongo} onChange={mock()} />)
    const mongoForm = within(mongoView.container)
    for (const n of [1, 2]) {
      const input = mongoForm.getByLabelText(`Seed host ${n}`)
      expect(input.getAttribute("aria-required")).toBe("true")
      expect(labelFor(mongoView.container, input)?.getAttribute("data-required")).toBe("")
    }
    expect(mongoForm.getByText(/use Remove for a row you do not need/i)).toBeDefined()

    const kafka: KafkaDial = {
      bootstrapServers: ["kafka-a.internal:9092", "kafka-b.internal:9092"],
      topic: "t",
      auth: { type: "sasl_plain", username: "u" },
      groupId: "g",
      microBatchSeconds: 30,
    }
    const kafkaView = render(<KafkaDialForm value={kafka} onChange={mock()} />)
    const kafkaForm = within(kafkaView.container)
    for (const n of [1, 2]) {
      const input = kafkaForm.getByLabelText(`Bootstrap server ${n}`)
      expect(input.getAttribute("aria-required")).toBe("true")
      expect(labelFor(kafkaView.container, input)?.getAttribute("data-required")).toBe("")
    }
    expect(kafkaForm.getByText(/use Remove for a row you do not need/i)).toBeDefined()

    const sheets: SheetsDial = { spreadsheetId: "sheet", ranges: ["Sheet1!A1:D", "Sheet2!A1:D"] }
    const sheetsView = render(<SheetsDialForm value={sheets} onChange={mock()} />)
    const sheetsForm = within(sheetsView.container)
    for (const n of [1, 2]) {
      const input = sheetsForm.getByLabelText(`Range ${n}`)
      expect(input.getAttribute("aria-required")).toBe("true")
      const label = labelFor(sheetsView.container, input)
      expect(label?.getAttribute("data-required")).toBe("")
      expect(label?.className).not.toContain("sr-only")
    }

    const rest: RestDial = {
      baseUrl: "https://api.example.com",
      auth: { type: "api_key", header: "X-API-Key" },
      pagination: { type: "none" },
      endpoints: [
        { path: "/v1/a", recordsPath: null },
        { path: "/v1/b", recordsPath: null },
      ],
    }
    const restView = render(<RestDialForm value={rest} onChange={mock()} />)
    const restForm = within(restView.container)
    for (const n of [0, 1]) {
      const input = restView.container.querySelector(`#rest-dial-endpoint-${n}-path`)
      expect(input?.getAttribute("aria-required")).toBe("true")
      expect(labelFor(restView.container, input as HTMLElement)?.getAttribute("data-required")).toBe("")
    }
    expect(restForm.getByText(/every listed endpoint is polled/i)).toBeDefined()
  })

  // UX-1-R7: composed connector steps show one legend; the dial's own is
  // opt-out, so a step is not left with three copies of the same explanation.
  it("lets a composed parent opt a dial form out of its standalone legend", () => {
    const standalone = render(<SqlDialForm value={null} onChange={mock()} />)
    expect(within(standalone.container).getByText(/Required/)).toBeDefined()
    const composed = render(<SqlDialForm value={null} onChange={mock()} showLegend={false} />)
    expect(within(composed.container).queryByText(/Required/)).toBeNull()
  })
})
