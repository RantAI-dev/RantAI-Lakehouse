import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { readTicket } from "./public-link-access"

// happy-dom has no base URL for `next/image`'s logo, which is not under test.
mock.module("next/image", () => ({ default: () => null }))
const { PublicDashboard } = await import("./public-dashboard")

const originalFetch = global.fetch
beforeEach(() => window.sessionStorage.clear())
afterEach(() => {
  global.fetch = originalFetch
  window.sessionStorage.clear()
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const PAYLOAD = { board: { id: "b_1", name: "Sales" }, layout: {}, charts: [], results: {} }

function stub(handler: (url: string, init?: RequestInit) => Response) {
  const calls: { url: string; headers: Record<string, string> }[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    calls.push({ url: String(input), headers: (init?.headers ?? {}) as Record<string, string> })
    return handler(String(input), init)
  }) as unknown as typeof fetch
  return calls
}

describe("a public link in the browser (BI-19 part A)", () => {
  it("shows one 'does not work' page for a 404", async () => {
    stub(() => json({ error: "not_found" }, 404))
    render(<PublicDashboard token="p_gone" />)
    await screen.findByText("This link does not work")
  })

  it("asks for the password, keeps the ticket for the session and sends it", async () => {
    const calls = stub((url) => {
      if (url.endsWith("/unlock")) return json({ ticket: "t-123", expiresAt: "2030-01-01T00:00:00Z" })
      return calls.some((c) => c.headers["X-Public-Link-Ticket"] === "t-123")
        ? json(PAYLOAD)
        : json({ error: "password_required" }, 401)
    })
    render(<PublicDashboard token="p_pw" />)
    fireEvent.change(await screen.findByLabelText("Password"), { target: { value: "correct horse" } })
    fireEvent.click(screen.getByRole("button", { name: "Open" }))
    await screen.findByText("Sales")
    expect(readTicket("p_pw")).toBe("t-123")
    expect(calls.some((c) => c.headers["X-Public-Link-Ticket"] === "t-123")).toBe(true)
  })

  it("says a wrong password did not work and stays on the password page", async () => {
    stub((url) => (url.endsWith("/unlock") ? json({ error: "wrong_password" }, 401) : json({ error: "password_required" }, 401)))
    render(<PublicDashboard token="p_pw" />)
    fireEvent.change(await screen.findByLabelText("Password"), { target: { value: "nope nope" } })
    fireEvent.click(screen.getByRole("button", { name: "Open" }))
    await waitFor(() => expect(screen.getByRole("alert").textContent).toBe("That password did not work."))
    expect(readTicket("p_pw")).toBeNull()
  })

  it("works when the browser has no session storage", async () => {
    const spy = Object.getOwnPropertyDescriptor(window, "sessionStorage")
    Object.defineProperty(window, "sessionStorage", { configurable: true, get() { throw new Error("blocked") } })
    try {
      stub(() => json(PAYLOAD))
      render(<PublicDashboard token="p_open" />)
      await screen.findByText("Sales")
    } finally {
      if (spy) Object.defineProperty(window, "sessionStorage", spy)
    }
  })

  // BI-19 part A QA fix P1.
  it("lets the visitor see what they typed, with a labelled toggle", async () => {
    stub(() => json({ error: "password_required" }, 401))
    render(<PublicDashboard token="p_pw" />)
    const field = (await screen.findByLabelText("Password")) as HTMLInputElement
    expect(field.type).toBe("password")
    fireEvent.click(screen.getByRole("button", { name: "Show password" }))
    expect(field.type).toBe("text")
    fireEvent.click(screen.getByRole("button", { name: "Hide password" }))
    expect(field.type).toBe("password")
  })
})
