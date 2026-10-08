import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"
import { SqlDialForm } from "./sql-dial-form"
import { RestDialForm } from "./rest-dial-form"
import { CdcDialForm } from "./cdc-dial-form"
import { FilesDialForm } from "./files-dial-form"
import { credentialSlotsFor, driverForType } from "../connector-form-parts"

describe("dial forms", () => {
  it("SqlDialForm renders host/port/database/user as plain inputs, no secret picker for user", () => {
    render(<SqlDialForm value={null} onChange={mock()} />)
    expect(screen.getByLabelText("Host")).toBeDefined()
    expect(screen.getByLabelText("User")).toHaveProperty("type", "text")
  })
  it("SqlDialForm starts on the driver the type implies, with its port, and does not ask for it", () => {
    render(<SqlDialForm value={null} onChange={mock()} driver="mysql" />)
    expect(screen.queryByLabelText("Driver")).toBeNull()
    expect((screen.getByLabelText("Port") as HTMLInputElement).value).toBe("3306")
  })
  it("SqlDialForm shows the driver select again when a stored dial disagrees with the type", () => {
    render(
      <SqlDialForm
        value={{ driver: "postgres", host: "h", port: 5432, database: "d", user: "u", sslMode: null }}
        onChange={mock()}
        driver="mysql"
      />
    )
    expect((screen.getByLabelText("Driver") as HTMLSelectElement).value).toBe("postgres")
  })
  it("CdcDialForm starts on the implied driver's port", () => {
    render(<CdcDialForm value={null} onChange={mock()} driver="mssql" />)
    expect(screen.queryByLabelText("Driver")).toBeNull()
    expect((screen.getByLabelText("Port") as HTMLInputElement).value).toBe("1433")
  })
  it("FilesDialForm offers no SFTP choice and keeps emitting protocol s3 (SRC-6 F8)", () => {
    const onChange = mock()
    render(<FilesDialForm value={null} onChange={onChange} />)
    expect(screen.queryByText("SFTP")).toBeNull()
    expect(screen.queryByRole("option", { name: "SFTP" })).toBeNull()
    expect(screen.getByText("S3-compatible")).toBeDefined()
    fireEvent.change(screen.getByLabelText("Bucket / root"), { target: { value: "b" } })
    expect(onChange.mock.calls[0][0]).toMatchObject({ protocol: "s3", bucket: "b" })
  })
  it("FilesDialForm labels the endpoint and says why it is needed (SRC-6 D3)", () => {
    render(<FilesDialForm value={null} onChange={mock()} />)
    expect(screen.getByLabelText("Endpoint")).toBeDefined()
    expect(screen.getByText("Needed to test the connection. Leave empty for AWS S3.")).toBeDefined()
  })
  it("FilesDialForm still renders a stored dial whose protocol is sftp, as plain text", () => {
    render(
      <FilesDialForm
        value={{ protocol: "sftp", endpoint: null, bucket: "root", prefix: null, format: "csv", region: null }}
        onChange={mock()}
      />
    )
    expect(screen.getByText("sftp")).toBeDefined()
    expect((screen.getByLabelText("Bucket / root") as HTMLInputElement).value).toBe("root")
  })
  it("driverForType maps type names, CDC variants included", () => {
    expect(driverForType("MariaDB")).toBe("mysql")
    expect(driverForType("SQL Server CDC")).toBe("mssql")
    expect(driverForType("Oracle")).toBeUndefined()
  })
  it("credentialSlotsFor mirrors secret_field_names: one slot per field the adapter reads", () => {
    const slots = (adapter: string, auth?: string) => {
      const { primary, secondary } = credentialSlotsFor(adapter, auth ? { auth: { type: auth } } : null)
      return [primary.label, secondary?.label ?? null]
    }
    expect(slots("sql")).toEqual(["Password", null])
    expect(slots("files")).toEqual(["Access key ID", "Secret access key"])
    expect(slots("sftp", "public_key")).toEqual(["Private key", null])
    expect(slots("rest", "api_key")).toEqual(["API key", null])
    expect(slots("rest", "bearer")).toEqual(["Bearer token", null])
    expect(slots("rest", "basic")).toEqual(["Username", "Password"])
    expect(slots("rest", "oauth2_client_credentials")).toEqual(["Client ID", "Client secret"])
    expect(slots("sheets")).toEqual(["Service account JSON", null])
    // Two slots never share a kind: each kind names its own stored file.
    const pair = credentialSlotsFor("rest", { auth: { type: "basic" } })
    expect(pair.primary.kind).not.toBe(pair.secondary?.kind)
  })
  it("RestDialForm renders an auth-type selector with all four types", () => {
    render(<RestDialForm value={null} onChange={mock()} />)
    const select = screen.getByLabelText("Auth type") as HTMLSelectElement
    const options = Array.from(select.options).map((o) => o.value)
    expect(options).toEqual(["api_key", "bearer", "oauth2_client_credentials", "basic"])
  })
})
