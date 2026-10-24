import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { DEFAULT_EXPIRY, PublicLinkPanel, expiryFor } from "./public-link-panel"

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

type Call = { url: string; method: string; body: unknown }

function stub(state: unknown, onWrite: (c: Call) => Response = () => json({ ok: true })): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call = {
      url: String(input),
      method: init?.method ?? "GET",
      body: init?.body ? JSON.parse(String(init.body)) : undefined,
    }
    calls.push(call)
    if (call.method === "GET") return json(state)
    return onWrite(call)
  }) as unknown as typeof fetch
  return calls
}

const LINK = {
  token: "p_abc",
  expiresAt: "2030-01-15T10:00:00.000Z",
  expired: false,
  hasPassword: true,
  createdAt: "2030-01-01T10:00:00.000Z",
}

describe("expiryFor (BI-19 part A, feature A1)", () => {
  const now = new Date("2030-01-01T00:00:00Z")
  it("30 days is the default and each choice is that many days from now", () => {
    expect(DEFAULT_EXPIRY).toBe("30")
    expect(expiryFor("30", "", now)).toEqual({ at: "2030-01-31T00:00:00.000Z" })
    expect(expiryFor("1", "", now)).toEqual({ at: "2030-01-02T00:00:00.000Z" })
  })
  it("a date must be within a year and not already past", () => {
    expect("error" in expiryFor("date", "2029-12-31", now)).toBe(true)
    expect("error" in expiryFor("date", "2031-06-01", now)).toBe(true)
    expect("error" in expiryFor("date", "", now)).toBe(true)
    expect("at" in expiryFor("date", "2030-06-01", now)).toBe(true)
  })
})

describe("PublicLinkPanel", () => {
  it("offers 30 days preselected and sends the expiry and the password when making a link", async () => {
    const calls = stub({ enabled: true, link: null }, () => json({ ok: true, link: LINK }))
    render(<PublicLinkPanel board="b_1" />)
    const select = (await screen.findByLabelText("Link expires in")) as HTMLSelectElement
    expect(select.value).toBe("30")
    fireEvent.change(screen.getByLabelText("Password (optional)"), { target: { value: "correct horse" } })
    fireEvent.click(screen.getByRole("button", { name: "Make a public link" }))
    await screen.findByText(/A password is needed/)
    const put = calls.find((c) => c.method === "PUT")
    expect(put?.url).toBe("/api/dashboard/public-link")
    const body = put?.body as { board: string; expiresAt: string; password: string }
    expect(body.board).toBe("b_1")
    expect(body.password).toBe("correct horse")
    const days = (Date.parse(body.expiresAt) - Date.now()) / 86_400_000
    expect(days).toBeGreaterThan(29.9)
    expect(days).toBeLessThan(30.1)
  })

  it("never shows a password back: it says one is set and offers to replace or remove it", async () => {
    stub({ enabled: true, link: LINK })
    render(<PublicLinkPanel board="b_1" />)
    await screen.findByText(/A password is needed/)
    expect(screen.getByText(/You cannot read it back/)).toBeDefined()
    expect((screen.getByLabelText("New password") as HTMLInputElement).value).toBe("")
    expect(screen.getByRole("button", { name: "Replace" })).toBeDefined()
    expect(document.body.textContent).not.toContain("argon2")
  })

  it("removes the password with null and keeps the link's own end", async () => {
    const calls = stub({ enabled: true, link: LINK }, () => json({ ok: true, link: { ...LINK, hasPassword: false } }))
    render(<PublicLinkPanel board="b_1" />)
    fireEvent.click(await screen.findByText("Remove the password"))
    await screen.findByText(/No password\./)
    const put = calls.find((c) => c.method === "PUT")
    expect(put?.body).toEqual({ board: "b_1", expiresAt: LINK.expiresAt, password: null })
  })

  it("switches the link off and then offers to make a new one", async () => {
    const calls = stub({ enabled: true, link: LINK })
    render(<PublicLinkPanel board="b_1" />)
    fireEvent.click(await screen.findByText("Switch off now"))
    await screen.findByRole("button", { name: "Make a public link" })
    expect(calls.some((c) => c.method === "DELETE" && c.url === "/api/dashboard/public-link?board=b_1")).toBe(true)
  })

  it("says an ended link no longer works, and says when public links are switched off", async () => {
    stub({ enabled: true, link: { ...LINK, expired: true } })
    render(<PublicLinkPanel board="b_1" />)
    await screen.findByText(/It no longer works/)
    cleanup()
    stub({ enabled: false, link: null })
    render(<PublicLinkPanel board="b_1" />)
    await screen.findByText(/switched public links off/)
    expect(screen.queryByRole("button", { name: "Make a public link" })).toBeNull()
  })

  it("shows the server's sentence when a save is refused", async () => {
    stub({ enabled: true, link: null }, () => json({ error: "A password needs 8 to 128 characters." }, 400))
    render(<PublicLinkPanel board="b_1" />)
    fireEvent.change(await screen.findByLabelText("Password (optional)"), { target: { value: "short" } })
    fireEvent.click(screen.getByRole("button", { name: "Make a public link" }))
    await waitFor(() => expect(screen.getByRole("alert").textContent).toContain("8 to 128"))
  })
})
