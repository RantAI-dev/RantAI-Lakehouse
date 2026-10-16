import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { ConnectorOAuthPanel, oauthSignIn, signOutWarning } from "./connector-oauth-panel"

const originalFetch = global.fetch
const originalOpen = window.open

afterEach(() => {
  global.fetch = originalFetch
  window.open = originalOpen
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string }

type Setup = {
  signedIn?: boolean
  callbackUrl?: string | null
  revocationConfigured?: boolean
  /** The newest run, and its attempts' error classes. */
  newestRun?: { status: string; errorClasses: (string | null)[] } | null
  signOut?: { revocation: string; message: string }
}

function stub(setup: Setup = {}) {
  const calls: Call[] = []
  let signedIn = setup.signedIn ?? false
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method })
    if (url.endsWith("/oauth/start") && method === "POST") {
      return json({ authorizeUrl: "https://login.example.com/authorize?state=abc", expiresInSeconds: 600 })
    }
    if (url.endsWith("/oauth/sign-out") && method === "POST") {
      signedIn = false
      return json({
        signedOut: true,
        revocation: setup.signOut?.revocation ?? "done",
        message: setup.signOut?.message ?? "The provider was told to revoke the token.",
      })
    }
    if (url.endsWith("/oauth")) {
      return json({
        signedIn,
        callbackUrl: setup.callbackUrl === undefined ? "https://console.test/api/connectors/oauth/callback" : setup.callbackUrl,
        revocationConfigured: setup.revocationConfigured ?? true,
      })
    }
    if (url.endsWith("/runs")) {
      const run = setup.newestRun
      return json({
        runs: run
          ? [{ runId: "run-1", trigger: "schedule", table: null, status: run.status, startedAt: "2026-10-15T01:00:00.000Z" }]
          : [],
      })
    }
    if (url.includes("/runs/run-1/log")) {
      return json({
        run: { runId: "run-1" },
        attempts: (setup.newestRun?.errorClasses ?? []).map((errorClass) => ({ errorClass })),
        lines: [],
      })
    }
    return json({ error: "unexpected" }, 500)
  }) as unknown as typeof fetch
  return { calls, setSignedIn: (v: boolean) => (signedIn = v) }
}

describe("ConnectorOAuthPanel (SRC-12 part C)", () => {
  it("offers Sign in to a connector nobody has signed in to, and shows the callback address", async () => {
    stub()
    render(<ConnectorOAuthPanel connectorId="c1" />)
    expect(await screen.findByText("Not signed in")).toBeDefined()
    expect(screen.getByRole("button", { name: "Sign in" })).toBeDefined()
    expect(screen.queryByRole("button", { name: "Sign out" })).toBeNull()
    expect(screen.getByText("https://console.test/api/connectors/oauth/callback")).toBeDefined()
  })

  it("opens the provider's page without an opener and without a referrer, from the address the API gave", async () => {
    const { calls } = stub()
    const opened: unknown[][] = []
    window.open = ((...args: unknown[]) => {
      opened.push(args)
      return null
    }) as unknown as typeof window.open
    render(<ConnectorOAuthPanel connectorId="c1" />)
    fireEvent.click(await screen.findByRole("button", { name: "Sign in" }))
    await waitFor(() => expect(opened).toHaveLength(1))
    expect(opened[0]).toEqual([
      "https://login.example.com/authorize?state=abc",
      "_blank",
      "noopener,noreferrer",
    ])
    expect(calls.some((c) => c.url.endsWith("/api/connectors/c1/oauth/start") && c.method === "POST")).toBe(true)
    expect(await screen.findByText(/Waiting for the provider/)).toBeDefined()
  })

  it("notices the sign-in when the connector has a token afterwards, and reports the change", async () => {
    const { setSignedIn } = stub()
    window.open = mock(() => null) as unknown as typeof window.open
    const onChanged = mock()
    render(<ConnectorOAuthPanel connectorId="c1" onChanged={onChanged} />)
    fireEvent.click(await screen.findByRole("button", { name: "Sign in" }))
    await screen.findByText(/Waiting for the provider/)
    setSignedIn(true)
    // "Check again" is the same ask the poll makes.
    fireEvent.click(screen.getByRole("button", { name: "Check again" }))
    expect(await screen.findByText("Signed in")).toBeDefined()
    await waitFor(() => expect(onChanged).toHaveBeenCalled())
    expect(screen.queryByText(/Waiting for the provider/)).toBeNull()
    expect(screen.getByRole("button", { name: "Sign in again" })).toBeDefined()
  })

  it("refuses to start a sign-in when the operator has not set the callback address", async () => {
    stub({ callbackUrl: null })
    render(<ConnectorOAuthPanel connectorId="c1" />)
    expect(await screen.findByText(/CONNECTOR_OAUTH_REDIRECT_URI/)).toBeDefined()
    expect((screen.getByRole("button", { name: "Sign in" }) as HTMLButtonElement).disabled).toBe(true)
  })

  it("says sign in again when the provider refused the newest run", async () => {
    stub({ signedIn: true, newestRun: { status: "failed", errorClasses: [null, "sign_in_refused"] } })
    render(<ConnectorOAuthPanel connectorId="c1" />)
    expect(await screen.findByText(/refused this connector's sign-in on its last run/)).toBeDefined()
    expect(screen.getByRole("button", { name: "Sign in again" })).toBeDefined()
  })

  it("does not say it for a run that failed for another reason, or that succeeded", async () => {
    stub({ signedIn: true, newestRun: { status: "failed", errorClasses: ["unreachable"] } })
    const { unmount } = render(<ConnectorOAuthPanel connectorId="c1" />)
    await screen.findByText("Signed in")
    expect(screen.queryByText(/refused this connector's sign-in/)).toBeNull()
    unmount()
    cleanup()
    stub({ signedIn: true, newestRun: { status: "succeeded", errorClasses: [] } })
    render(<ConnectorOAuthPanel connectorId="c1" />)
    await screen.findByText("Signed in")
    expect(screen.queryByText(/refused this connector's sign-in/)).toBeNull()
  })

  it("warns before signing out, differently with and without a revocation address, then shows what happened", async () => {
    stub({ signedIn: true, revocationConfigured: false, signOut: { revocation: "notConfigured", message: "The provider was not told: this connector has no revocation address, so the token stays valid at the provider until it expires." } })
    render(<ConnectorOAuthPanel connectorId="c1" />)
    fireEvent.click(await screen.findByRole("button", { name: "Sign out" }))
    expect(screen.getByText(signOutWarning(false))).toBeDefined()
    const confirm = screen.getByRole("group", { name: "Confirm sign out" })
    fireEvent.click(within(confirm).getByRole("button", { name: "Sign out" }))
    expect(await screen.findByText(/stays valid at the provider until it expires/)).toBeDefined()
    expect(await screen.findByText("Not signed in")).toBeDefined()
  })

  it("the sign-out warning names the revocation only when there is an address", () => {
    expect(signOutWarning(true)).toContain("tells the provider to revoke")
    expect(signOutWarning(false)).toContain("stays valid there until it expires")
  })
})

describe("oauthSignIn", () => {
  it("is set only for a REST connector that signs in with authorization code", () => {
    const code = (extra: object) => ({ adapter: "rest", dial: { auth: { type: "oauth2_authorization_code", ...extra } } })
    expect(oauthSignIn(code({ revocationUrl: "https://l.example.com/revoke" }))).toEqual({ revocationConfigured: true })
    expect(oauthSignIn(code({}))).toEqual({ revocationConfigured: false })
    expect(oauthSignIn(code({ revocationUrl: "" }))).toEqual({ revocationConfigured: false })
    expect(oauthSignIn({ adapter: "rest", dial: { auth: { type: "bearer" } } })).toBeNull()
    expect(oauthSignIn({ adapter: "sql", dial: { auth: { type: "oauth2_authorization_code" } } })).toBeNull()
    expect(oauthSignIn({ adapter: null, dial: null })).toBeNull()
  })
})
