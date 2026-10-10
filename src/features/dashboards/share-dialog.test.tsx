import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { ShareDialog } from "./share-dialog"

const originalFetch = global.fetch
const originalExec = document.execCommand
const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard")

beforeEach(() => {
  localStorage.clear()
})

afterEach(() => {
  global.fetch = originalFetch
  document.execCommand = originalExec
  if (originalClipboard) Object.defineProperty(navigator, "clipboard", originalClipboard)
  else Reflect.deleteProperty(navigator, "clipboard")
  localStorage.clear()
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string; body: unknown }

const INFO = {
  enabled: true,
  supported: true,
  sampleToken: "sample.token.value",
  maxLifetimeSeconds: 7200,
  revokedBefore: null as number | null,
  allowedOrigins: [] as string[],
}

/**
 * Serves the dialog's reads from `info` and answers each write with
 * `onWrite`, recording every call.
 */
function stub(
  info: Record<string, unknown> | "fail",
  onWrite: (call: Call) => Response = () => json({ ok: true }),
): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    const body = init?.body ? JSON.parse(String(init.body)) : undefined
    const call = { url, method, body }
    calls.push(call)
    if (url.startsWith("/api/dashboard/embed-info")) {
      return info === "fail" ? json({ error: "x" }, 500) : json(info)
    }
    if (url === "/api/dashboard/boards" && method === "GET") return json({ boards: [{ id: "b_1", publicToken: "" }] })
    return onWrite(call)
  }) as unknown as typeof fetch
  return calls
}

function open() {
  render(<ShareDialog board="b_1" dashName="Sales" open onOpenChange={() => {}} />)
}

const writes = (calls: Call[]) => calls.filter((c) => c.method !== "GET")

describe("ShareDialog: signed embedding (SEC-12)", () => {
  it("says signed embedding is not configured when the server has no secret, and offers no token or withdrawal", async () => {
    stub({ enabled: true, supported: false, reason: "embedding is not configured", maxLifetimeSeconds: 86400, revokedBefore: null, allowedOrigins: [] })
    open()
    await screen.findByText(/Signed embedding is not configured on this server/)
    expect(screen.queryByText("Sign a token (server-side)")).toBeNull()
    expect(screen.queryByText("Withdraw all embed tokens")).toBeNull()
    expect(screen.queryByText(/Preview signed embed/)).toBeNull()
  })

  it("a state that cannot be loaded is an error, not 'not configured'", async () => {
    stub("fail")
    open()
    await screen.findByText(/could not be loaded/)
    expect(screen.queryByText(/is not configured on this server/)).toBeNull()
  })

  it("states the token rules with the limit the server reported", async () => {
    stub(INFO)
    open()
    await screen.findByText(/may live at most 2 hours/)
    expect(screen.getByText(/at most 7200 \(2 hours\)/)).toBeDefined()
  })

  it("asks for confirmation before withdrawing all tokens, and cancelling sends nothing", async () => {
    const calls = stub(INFO)
    open()
    fireEvent.click(await screen.findByText("Withdraw all embed tokens"))
    await screen.findByText(/stops working, within a minute/)
    fireEvent.click(screen.getByText("Cancel"))
    expect(screen.queryByText(/stops working, within a minute/)).toBeNull()
    expect(writes(calls)).toHaveLength(0)
  })

  it("withdraws all tokens once confirmed and says the result", async () => {
    const calls = stub(INFO, () => json({ ok: true, embedRevokedBefore: 1_700_000_000 }))
    open()
    fireEvent.click(await screen.findByText("Withdraw all embed tokens"))
    fireEvent.click(await screen.findByText("Withdraw all"))
    await screen.findByText(/All embed tokens issued so far are withdrawn/)
    expect(writes(calls)).toEqual([{ url: "/api/dashboard/boards", method: "PUT", body: { id: "b_1", embedRevokeAll: true } }])
    expect(screen.getByText(/Tokens issued before .* are withdrawn\./)).toBeDefined()
  })

  it("shows the server's refusal when withdrawing is not allowed", async () => {
    stub(INFO, () => json({ error: "permission_denied: dashboard:write" }, 403))
    open()
    fireEvent.click(await screen.findByText("Withdraw all embed tokens"))
    fireEvent.click(await screen.findByText("Withdraw all"))
    await screen.findByText(/permission_denied: dashboard:write/)
  })

  it("withdraws one pasted token and reports its id", async () => {
    const calls = stub(INFO, () => json({ ok: true, withdrawn: "j-1" }))
    open()
    fireEvent.change(await screen.findByLabelText("Token to withdraw"), { target: { value: "  a.b.c  " } })
    fireEvent.click(screen.getByText("Withdraw this token"))
    await screen.findByText(/Token withdrawn \(id j-1\)/)
    expect(writes(calls)).toEqual([
      { url: "/api/dashboard/embed-revoke", method: "POST", body: { board: "b_1", token: "a.b.c" } },
    ])
  })

  it("tells the user a token without an id cannot be withdrawn on its own", async () => {
    stub(INFO, () => json({ error: "that token has no id (jti), so it cannot be withdrawn on its own; withdraw all the dashboard's embed tokens instead." }, 400))
    open()
    fireEvent.change(await screen.findByLabelText("Token to withdraw"), { target: { value: "a.b.c" } })
    fireEvent.click(screen.getByText("Withdraw this token"))
    await screen.findByText(/no id \(jti\)/)
  })
})

describe("ShareDialog: sites allowed to show embeds (SEC-12)", () => {
  it("says no site can show the embed while the list is empty", async () => {
    stub(INFO)
    open()
    await screen.findByText("No site is allowed yet, so the embed cannot be shown anywhere.")
  })

  it("lists the stored sites", async () => {
    stub({ ...INFO, allowedOrigins: ["https://app.example.com"] })
    open()
    await screen.findByText("https://app.example.com")
    expect(screen.queryByText(/No site is allowed yet/)).toBeNull()
  })

  it("adds a site, saves the list and shows what the server stored", async () => {
    const calls = stub(INFO, () => json({ ok: true, embedOrigins: ["https://app.example.com"] }))
    open()
    fireEvent.change(await screen.findByLabelText("Site to allow"), { target: { value: "https://App.example.com" } })
    fireEvent.click(screen.getByText("Add"))
    fireEvent.click(screen.getByText("Save sites"))
    await screen.findByText("Saved")
    expect(writes(calls)).toEqual([
      { url: "/api/dashboard/boards", method: "PUT", body: { id: "b_1", embedOrigins: ["https://app.example.com"] } },
    ])
    await screen.findByText("https://app.example.com")
  })

  it("an invalid site never becomes a chip and nothing is sent", async () => {
    const calls = stub(INFO)
    open()
    fireEvent.change(await screen.findByLabelText("Site to allow"), { target: { value: "*" } })
    fireEvent.click(screen.getByText("Add"))
    await screen.findByText(/Wildcards are not accepted/)
    expect(screen.queryByLabelText("Remove *")).toBeNull()
    expect(screen.queryByText("Save sites")).toBeNull()
    expect(writes(calls)).toHaveLength(0)
  })

  it("shows the server's message when it refuses a site the page accepted", async () => {
    stub(INFO, () => json({ error: "the server does not like that site" }, 400))
    open()
    fireEvent.change(await screen.findByLabelText("Site to allow"), { target: { value: "https://ok.example" } })
    fireEvent.click(screen.getByText("Add"))
    fireEvent.click(screen.getByText("Save sites"))
    await screen.findByText(/the server does not like that site/)
    expect(screen.getByLabelText("Remove https://ok.example")).toBeDefined()
  })

  it("shows Save sites only once the list differs from the saved one", async () => {
    stub({ ...INFO, allowedOrigins: ["https://a.example"] })
    open()
    await screen.findByText("https://a.example")
    expect(screen.queryByText("Save sites")).toBeNull()
    fireEvent.click(screen.getByLabelText("Remove https://a.example"))
    expect(screen.getByText("Save sites")).toBeDefined()
  })

  it("removing a site and saving sends the shorter list", async () => {
    const calls = stub({ ...INFO, allowedOrigins: ["https://a.example", "https://b.example"] }, () => json({ ok: true, embedOrigins: ["https://b.example"] }))
    open()
    fireEvent.click(await screen.findByLabelText("Remove https://a.example"))
    fireEvent.click(screen.getByText("Save sites"))
    await waitFor(() => expect(writes(calls)).toHaveLength(1))
    expect(writes(calls)[0].body).toEqual({ id: "b_1", embedOrigins: ["https://b.example"] })
  })
})

describe("ShareDialog: layout and copying", () => {
  it("shows the sample token in a read-only field with Copy token and Preview", async () => {
    stub(INFO)
    open()
    const field = (await screen.findByLabelText("Sample token")) as HTMLInputElement
    expect(field.value).toBe("sample.token.value")
    expect(field.readOnly).toBe(true)
    expect(screen.getByText("Copy token")).toBeDefined()
    expect(screen.getByText(/Preview/).getAttribute("href")).toContain("/embed/signed/sample.token.value")
  })

  it("keeps the signing code and the withdraw tools collapsed until opened", async () => {
    stub(INFO)
    open()
    await screen.findByLabelText("Sample token")
    for (const title of ["How to sign a token", "Withdraw tokens"]) {
      const details = screen.getByText(title).closest("details") as HTMLDetailsElement
      expect(details.open).toBe(false)
    }
  })

  it("says when the last withdrawal was, beside the closed Withdraw tokens title", async () => {
    stub({ ...INFO, revokedBefore: Math.floor(Date.now() / 1000) - 3 * 3600 })
    open()
    await screen.findByText("last withdrawn 3 hours ago")
  })

  it("with no secret the toggle is disabled and the section is one line", async () => {
    stub({ enabled: false, supported: false, reason: "embedding is not configured", maxLifetimeSeconds: 86400, revokedBefore: null, allowedOrigins: [] })
    open()
    await screen.findByText(/is not configured on this server/)
    expect((screen.getByLabelText("Signed embedding") as HTMLElement).getAttribute("aria-disabled") === "true" ||
      (screen.getByLabelText("Signed embedding") as HTMLButtonElement).disabled).toBe(true)
    expect(screen.queryByLabelText("Sample token")).toBeNull()
  })

  it("copies the sample token on an insecure origin through the textarea fallback", async () => {
    stub(INFO)
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true })
    let copied = ""
    document.execCommand = mock(() => {
      copied = (document.activeElement as HTMLTextAreaElement).value
      return true
    }) as unknown as typeof document.execCommand
    open()
    fireEvent.click(await screen.findByText("Copy token"))
    await screen.findByText("Copied")
    expect(copied).toBe("sample.token.value")
  })

  it("says so when copying did not work, and never shows Copied", async () => {
    stub(INFO)
    Object.defineProperty(navigator, "clipboard", { value: undefined, configurable: true })
    document.execCommand = mock(() => false) as unknown as typeof document.execCommand
    open()
    fireEvent.click(await screen.findByText("Copy token"))
    await screen.findByText(/Could not copy — select the text and copy it/)
    expect(screen.queryByText("Copied")).toBeNull()
  })
})
