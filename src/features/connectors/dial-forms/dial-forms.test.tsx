import { fireEvent, render, screen } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"
import { SqlDialForm } from "./sql-dial-form"
import { RestDialForm } from "./rest-dial-form"
import { CdcDialForm } from "./cdc-dial-form"
import { CdcMongoDialForm } from "./cdc-mongo-dial-form"
import { DialFormFor, hostFromDial } from "../connector-form-parts"
import { connectionReviewItems } from "../connector-review"
import { finderKind } from "../table-finder"
import { FilesDialForm } from "./files-dial-form"
import { credentialProblems, credentialSlotsFor, driverForType } from "../connector-form-parts"
import type { RestDial } from "@/services/contracts/connectors"

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
  it("CdcDialForm shows the slot and publication for PostgreSQL only and sends no server id", () => {
    const onChange = mock()
    render(<CdcDialForm value={null} onChange={onChange} driver="postgres" />)
    expect(screen.getByLabelText("Replication slot name")).toBeDefined()
    expect(screen.getByLabelText("Publication name")).toBeDefined()
    expect(screen.queryByLabelText("Server id")).toBeNull()
    fireEvent.change(screen.getByLabelText("Host"), { target: { value: "h" } })
    const sent = onChange.mock.calls[0][0] as Record<string, unknown>
    expect(sent).toMatchObject({ slotName: "", publicationName: "" })
    expect("serverId" in sent).toBe(false)
  })
  it("CdcDialForm asks MySQL for a server id, empty by default, and sends no slot or publication", () => {
    const onChange = mock()
    render(<CdcDialForm value={null} onChange={onChange} driver="mysql" />)
    expect(screen.queryByLabelText("Replication slot name")).toBeNull()
    expect(screen.queryByLabelText("Publication name")).toBeNull()
    expect((screen.getByLabelText("Server id") as HTMLInputElement).value).toBe("")
    expect(screen.getByText(/no other MySQL replica uses/)).toBeDefined()
    fireEvent.change(screen.getByLabelText("Host"), { target: { value: "h" } })
    const empty = onChange.mock.calls[0][0] as Record<string, unknown>
    expect("serverId" in empty).toBe(false)
    expect("slotName" in empty).toBe(false)
    expect("publicationName" in empty).toBe(false)
    fireEvent.change(screen.getByLabelText("Server id"), { target: { value: "5400" } })
    expect(onChange.mock.calls[1][0]).toMatchObject({ serverId: 5400 })
  })
  it("CdcDialForm shows nothing of the three for SQL Server", () => {
    const onChange = mock()
    render(<CdcDialForm value={null} onChange={onChange} driver="mssql" />)
    for (const label of ["Replication slot name", "Publication name", "Server id"]) {
      expect(screen.queryByLabelText(label)).toBeNull()
    }
    fireEvent.change(screen.getByLabelText("Host"), { target: { value: "h" } })
    const sent = onChange.mock.calls[0][0] as Record<string, unknown>
    for (const key of ["slotName", "publicationName", "serverId"]) expect(key in sent).toBe(false)
  })
  it("CdcMongoDialForm asks for hosts, a replica set, a database and a user, with nothing invented", () => {
    const onChange = mock()
    render(<CdcMongoDialForm value={null} onChange={onChange} />)
    expect((screen.getByLabelText("Host 1") as HTMLInputElement).value).toBe("")
    expect((screen.getByLabelText("Replica set name") as HTMLInputElement).value).toBe("")
    expect(screen.getByText(/name in the server.s replica set configuration/)).toBeDefined()
    // None of the SQL or other change-capture fields.
    for (const label of ["Host", "Port", "Replication slot name", "Publication name", "Server id", "Driver"]) {
      expect(screen.queryByLabelText(label)).toBeNull()
    }
    fireEvent.change(screen.getByLabelText("Replica set name"), { target: { value: "rs0" } })
    const sent = onChange.mock.calls[0][0] as Record<string, unknown>
    expect(sent).toEqual({ driver: "mongodb", hosts: [""], database: "", user: "", replicaSet: "rs0" })
    // A dial with no TLS choice sends none; the server reads it as `require`.
    expect("sslMode" in sent).toBe(false)
  })
  it("CdcMongoDialForm adds and removes hosts and sends them as a list", () => {
    const onChange = mock()
    const value = { driver: "mongodb" as const, hosts: ["a:27017"], database: "d", user: "u", replicaSet: "rs0" }
    const { rerender } = render(<CdcMongoDialForm value={value} onChange={onChange} />)
    expect(screen.queryByText("Remove")).toBeNull()
    fireEvent.click(screen.getByText("Add host"))
    expect(onChange.mock.calls[0][0]).toMatchObject({ hosts: ["a:27017", ""] })
    const two = { ...value, hosts: ["a:27017", "b:27017"] }
    rerender(<CdcMongoDialForm value={two} onChange={onChange} />)
    fireEvent.change(screen.getByLabelText("Host 2"), { target: { value: "c:27018" } })
    expect(onChange.mock.calls[1][0]).toMatchObject({ hosts: ["a:27017", "c:27018"] })
    fireEvent.click(screen.getAllByText("Remove")[0])
    expect(onChange.mock.calls[2][0]).toMatchObject({ hosts: ["b:27017"] })
  })
  it("MongoDB CDC takes its own form, a host from its first seed, the collection finder and a review of its seeds", () => {
    const { container } = render(
      <DialFormFor adapter="cdc" typeName="MongoDB CDC" dial={null} onChange={mock()} />
    )
    expect(container.querySelector("#cdc-mongo-dial-replica-set")).not.toBeNull()
    expect(container.querySelector("#cdc-dial-host")).toBeNull()
    const dial = { driver: "mongodb", hosts: ["a:27017", "b:27017"], database: "d", user: "u", replicaSet: "rs0" }
    expect(hostFromDial("cdc", dial)).toBe("a:27017")
    expect(finderKind("cdc", dial)).toBe("mongodb")
    // The other change-capture drivers keep theirs.
    expect(finderKind("cdc", { driver: "postgres" } as never)).toBe("sql")
    expect(hostFromDial("cdc", { host: "h" })).toBe("h")
    const labels = connectionReviewItems("cdc", dial).map((i) => [i.label, i.value])
    expect(labels).toEqual([
      ["Hosts", "a:27017, b:27017"],
      ["Replica set", "rs0"],
      ["Database", "d"],
      ["User", "u"],
    ])
    // The relational types still get the relational form.
    const sql = render(<DialFormFor adapter="cdc" typeName="MySQL CDC" dial={null} onChange={mock()} />)
    expect(sql.container.querySelector("#cdc-dial-host")).not.toBeNull()
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
  it("FilesDialForm names the protocol text by its Protocol label (SRC-6 slice A review, SHOULD-FIX 1)", () => {
    render(<FilesDialForm value={null} onChange={mock()} />)
    expect(screen.getByLabelText("Protocol").textContent).toBe("S3-compatible")
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
    // SRC-12 part C: the client secret is typed; the refresh token comes from signing in.
    expect(slots("rest", "oauth2_authorization_code")).toEqual(["Client secret", "Refresh token"])
    expect(slots("sheets")).toEqual(["Service account JSON", null])
    // Two slots never share a kind: each kind names its own stored file.
    const pair = credentialSlotsFor("rest", { auth: { type: "basic" } })
    expect(pair.primary.kind).not.toBe(pair.secondary?.kind)
  })
  it("RestDialForm renders an auth-type selector with all five types", () => {
    render(<RestDialForm value={null} onChange={mock()} />)
    const select = screen.getByLabelText("Auth type") as HTMLSelectElement
    const options = Array.from(select.options).map((o) => o.value)
    expect(options).toEqual([
      "api_key",
      "bearer",
      "oauth2_client_credentials",
      "oauth2_authorization_code",
      "basic",
    ])
  })
  it("RestDialForm starts the authorization-code auth with empty addresses and no scopes (SRC-12 part C)", () => {
    const onChange = mock()
    render(<RestDialForm value={null} onChange={onChange} />)
    fireEvent.change(screen.getByLabelText("Auth type"), { target: { value: "oauth2_authorization_code" } })
    expect(onChange.mock.calls[0][0].auth).toEqual({
      type: "oauth2_authorization_code",
      clientId: "",
      authorizeUrl: "",
      tokenUrl: "",
      scopes: [],
    })
  })
  describe("OAuth authorization code (SRC-12 part C)", () => {
    const CODE_DIAL: RestDial = {
      baseUrl: "https://api.example.com",
      auth: {
        type: "oauth2_authorization_code",
        clientId: "app-1",
        authorizeUrl: "https://login.example.com/authorize",
        tokenUrl: "https://login.example.com/token",
        revocationUrl: "https://login.example.com/revoke",
        scopes: ["read", "offline_access"],
      },
      pagination: { type: "none" },
      endpoints: [{ path: "/items", recordsPath: null }],
    }

    it("shows the stored addresses, client id and scopes", () => {
      render(<RestDialForm value={CODE_DIAL} onChange={mock()} />)
      expect((screen.getByLabelText("Client ID") as HTMLInputElement).value).toBe("app-1")
      expect((screen.getByLabelText("Sign-in URL") as HTMLInputElement).value).toBe(
        "https://login.example.com/authorize"
      )
      expect((screen.getByLabelText("Token URL") as HTMLInputElement).value).toBe("https://login.example.com/token")
      expect((screen.getByLabelText("Revocation URL (optional)") as HTMLInputElement).value).toBe(
        "https://login.example.com/revoke"
      )
      expect((screen.getByLabelText("Scopes") as HTMLInputElement).value).toBe("read offline_access")
    })

    it("emits scopes as a list, keeps a trailing space while typing, and drops an emptied revocation address", () => {
      const onChange = mock()
      render(<RestDialForm value={CODE_DIAL} onChange={onChange} />)
      const scopes = screen.getByLabelText("Scopes") as HTMLInputElement
      fireEvent.change(scopes, { target: { value: "read " } })
      expect(onChange.mock.calls[0][0].auth.scopes).toEqual(["read"])
      expect(scopes.value).toBe("read ")
      fireEvent.change(screen.getByLabelText("Revocation URL (optional)"), { target: { value: "" } })
      expect("revocationUrl" in onChange.mock.calls[1][0].auth).toBe(false)
    })

    it("says what signing out cannot do when there is no revocation address", () => {
      render(<RestDialForm value={CODE_DIAL} onChange={mock()} />)
      expect(screen.getByText(/the token stays valid\s+there until it expires/)).toBeDefined()
    })

    it("shows the callback address to copy, or says the operator has not set it", () => {
      const { unmount } = render(
        <RestDialForm value={CODE_DIAL} onChange={mock()} oauthCallbackUrl="https://console.test/api/connectors/oauth/callback" />
      )
      expect(screen.getByText("https://console.test/api/connectors/oauth/callback")).toBeDefined()
      unmount()
      render(<RestDialForm value={CODE_DIAL} onChange={mock()} oauthCallbackUrl={null} />)
      expect(screen.getByText(/CONNECTOR_OAUTH_REDIRECT_URI/)).toBeDefined()
    })

    it("asks for the client secret only: the refresh token slot is never a typed field", () => {
      const slots = credentialSlotsFor("rest", CODE_DIAL as unknown as Record<string, unknown>)
      expect(slots.primary).toMatchObject({ kind: "secret_key", label: "Client secret" })
      expect(slots.secondary).toMatchObject({ kind: "token", signIn: true })
      // With the secret typed, nothing is wrong; with it blank, the secret is required.
      expect(
        credentialProblems({ noCredential: false, optional: false, slots, primaryValue: "s3cret", secondaryValue: "" })
      ).toEqual({ primary: null, secondary: null })
      const blank = credentialProblems({ noCredential: false, optional: false, slots, primaryValue: "", secondaryValue: "" })
      expect(blank.primary).not.toBeNull()
      expect(blank.secondary).toBeNull()
      // On the edit page a blank form keeps what is stored.
      expect(
        credentialProblems({ noCredential: false, optional: true, slots, primaryValue: "", secondaryValue: "" })
      ).toEqual({ primary: null, secondary: null })
    })
  })
})
