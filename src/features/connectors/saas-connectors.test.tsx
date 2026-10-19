import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ConnectorType, SaasDial } from "@/services/contracts/connectors"
import { SAAS_VENDORS } from "@/lib/connectors/saas-vendors"
import { targetChanged } from "@/lib/connectors/target-identity"
import { SaasDialForm } from "./dial-forms/saas-dial-form"
import { ConnectorTypePicker } from "./connector-type-picker"
import { connectionReviewItems } from "./connector-review"
import { oauthSignIn } from "./connector-oauth-panel"
import { credentialSlotsFor, hostFromDial } from "./connector-form-parts"
import { finderKind } from "./table-finder"
import { loadModeProblem, supportsColumnChoice } from "./connector-ingest-panel"

afterEach(cleanup)

const oauth = (vendor: string, account = "acme"): SaasDial => ({
  vendor,
  account,
  auth: { type: "oauth2_authorization_code", clientId: "app" },
})
const integration = (vendor: string): SaasDial => ({ vendor, account: "acme", auth: { type: "integration" } })

describe("SaasDialForm", () => {
  it("asks only for the account and the sign-in, labelled for the vendor, and shows no address field", () => {
    render(<SaasDialForm typeName="Zendesk" value={null} onChange={mock()} />)
    expect(screen.getByLabelText("Subdomain (the part before .zendesk.com)")).toBeDefined()
    expect(screen.getByLabelText("How it signs in")).toBeDefined()
    expect(screen.queryByLabelText(/url|host|base/i)).toBeNull()
  })

  it("emits the vendor id from the type with the typed account", () => {
    const onChange = mock()
    render(<SaasDialForm typeName="HubSpot" value={null} onChange={onChange} />)
    fireEvent.change(screen.getByLabelText("HubSpot account id (digits)"), { target: { value: "123" } })
    expect(onChange.mock.calls[0][0]).toMatchObject({ vendor: "hubspot", account: "123" })
  })

  it("offers the integration credential only where the vendor has one (not Salesforce)", () => {
    render(<SaasDialForm typeName="Salesforce" value={null} onChange={mock()} />)
    expect(screen.queryByRole("option", { name: "An integration credential" })).toBeNull()
    cleanup()
    render(<SaasDialForm typeName="Zendesk" value={null} onChange={mock()} />)
    expect(screen.getByRole("option", { name: "An integration credential" })).toBeDefined()
  })

  it("switches to an integration credential and back, keeping the account", () => {
    const onChange = mock()
    render(<SaasDialForm typeName="Zendesk" value={oauth("zendesk")} onChange={onChange} />)
    fireEvent.change(screen.getByLabelText("How it signs in"), { target: { value: "integration" } })
    expect(onChange.mock.calls[0][0]).toEqual(integration("zendesk"))
  })

  it("shows the callback address for the browser sign-in and says when the operator has not set one", () => {
    render(<SaasDialForm typeName="Zendesk" value={oauth("zendesk")} onChange={mock()} oauthCallbackUrl="https://console.example.invalid/cb" />)
    expect(screen.getByTestId("oauth-callback").textContent).toContain("https://console.example.invalid/cb")
    cleanup()
    render(<SaasDialForm typeName="Zendesk" value={oauth("zendesk")} onChange={mock()} oauthCallbackUrl={null} />)
    expect(screen.getByTestId("oauth-callback").textContent).toContain("has not set")
  })

  it("says so for a type that is not a business app", () => {
    render(<SaasDialForm typeName="PostgreSQL" value={null} onChange={mock()} />)
    expect(screen.getByText(/no connection form/)).toBeDefined()
  })
})

describe("credential slots of a saas connector", () => {
  it("a browser sign-in asks for the client secret and leaves the refresh token to signing in", () => {
    const slots = credentialSlotsFor("saas", oauth("zendesk") as unknown as Record<string, unknown>)
    expect(slots.primary.label).toBe("Client secret")
    expect(slots.secondary).toMatchObject({ label: "Refresh token", signIn: true })
  })

  it("an integration credential fills the slots the vendor table names, in order", () => {
    const zendesk = credentialSlotsFor("saas", integration("zendesk") as unknown as Record<string, unknown>)
    expect([zendesk.primary.label, zendesk.secondary?.label]).toEqual(["Email address", "API token"])
    const hubspot = credentialSlotsFor("saas", integration("hubspot") as unknown as Record<string, unknown>)
    expect(hubspot.primary.label).toBe("Access token")
    expect(hubspot.secondary).toBeNull()
    const ga4 = credentialSlotsFor("saas", integration("ga4") as unknown as Record<string, unknown>)
    expect(ga4.primary.multiline).toBe(true)
  })

  it("every integration vendor yields as many slots as its table row has fields", () => {
    for (const vendor of SAAS_VENDORS) {
      if (!vendor.integration) continue
      const slots = credentialSlotsFor("saas", integration(vendor.id) as unknown as Record<string, unknown>)
      expect(slots.secondary === null ? 1 : 2).toBe(vendor.integration.secretFields.length)
    }
  })
})

describe("the rest of the saas connector surfaces", () => {
  it("lists the ten types under Business apps, disabled, with their reason", () => {
    const types: ConnectorType[] = SAAS_VENDORS.map((v) => ({
      name: v.connectorType,
      adapter: "saas",
      supported: false,
      docsUrl: null,
      unsupportedReason: "Being built (SRC-3).",
    }))
    render(<ConnectorTypePicker types={types} value={null} onChange={() => {}} />)
    expect(screen.getByText("Business apps")).toBeDefined()
    expect(screen.queryByText("Coming soon")).toBeNull()
    const radios = screen.getAllByRole("radio")
    expect(radios).toHaveLength(10)
    for (const radio of radios) expect((radio as HTMLButtonElement).disabled).toBe(true)
    expect(screen.getAllByText("Being built (SRC-3).")).toHaveLength(10)
  })

  it("shows the account and the sign-in in the review, and uses the account as the host", () => {
    const dial = oauth("zendesk") as unknown as Record<string, unknown>
    expect(connectionReviewItems("saas", dial)).toEqual([
      { label: "Business app", value: "Zendesk" },
      { label: "Account", value: "acme" },
      { label: "Sign-in", value: "Browser sign-in" },
    ])
    expect(hostFromDial("saas", dial)).toBe("acme")
  })

  it("asks for the credentials again when the vendor or the account changes, not for anything else", () => {
    const stored = oauth("zendesk") as unknown as Record<string, unknown>
    expect(targetChanged({ storedAdapter: "saas", storedDial: stored, adapter: "saas", dial: { ...stored, account: "ACME" } })).toBe(false)
    expect(targetChanged({ storedAdapter: "saas", storedDial: stored, adapter: "saas", dial: { ...stored, auth: { type: "integration" } } })).toBe(false)
    expect(targetChanged({ storedAdapter: "saas", storedDial: stored, adapter: "saas", dial: { ...stored, account: "other" } })).toBe(true)
    expect(targetChanged({ storedAdapter: "saas", storedDial: stored, adapter: "saas", dial: { ...stored, vendor: "confluence" } })).toBe(true)
  })

  it("shows the Sign in panel for a browser sign-in, with the revocation the vendor table has", () => {
    expect(oauthSignIn({ adapter: "saas", dial: oauth("salesforce") })).toEqual({ revocationConfigured: true })
    expect(oauthSignIn({ adapter: "saas", dial: oauth("zendesk") })).toEqual({ revocationConfigured: false })
    expect(oauthSignIn({ adapter: "saas", dial: integration("zendesk") })).toBeNull()
  })

  it("finds objects through the orchestrator and offers the incremental modes and column choice", () => {
    expect(finderKind("saas", null)).toBe("saas")
    expect(supportsColumnChoice("saas", null)).toBe(true)
    expect(loadModeProblem({ loadMode: "incremental_dedup", incrementalKey: "updated_at", primaryKey: ["id"] })).toBeNull()
  })
})
