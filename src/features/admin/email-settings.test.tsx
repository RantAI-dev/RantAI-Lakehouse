import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { EmailSettingsForm } from "./email-settings"

const originalFetch = global.fetch

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
})

function jsonResponse(body: unknown, status = 200): Response {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

type Call = { method: string; path: string; body: string | null }

/** What the server answers for the saved settings. */
const SAVED = {
  host: { value: null as string | null, source: "default" },
  port: { value: 587 as number | null, source: "default" },
  security: { value: "starttls" as "tls" | "starttls" | null, source: "default" },
  user: { value: null as string | null, source: "default" },
  password: { set: false, source: "default" },
  from: { value: null as string | null, source: "default" },
  consolePublicUrl: { value: null as string | null, source: "default" },
}

/**
 * Answers the settings read with `current`, the save with the same, and the
 * test email with `testResult`.
 */
function mockApi(current: unknown, testResult: unknown = { sent: true, message: "Sent to admin@example.com." }) {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ method, path: url, body: typeof init?.body === "string" ? init.body : null })
    if (url.includes("/api/settings/email/test")) return jsonResponse(testResult)
    return jsonResponse(current)
  }) as unknown as typeof fetch
  return calls
}

describe("EmailSettingsForm (ADM-7)", () => {
  it("shows a setting the server owns as read-only, and says why", async () => {
    mockApi({
      ...SAVED,
      host: { value: "smtp.example.com", source: "server" },
    })
    render(<EmailSettingsForm canWrite />)
    const host = (await screen.findByLabelText("Mail server")) as HTMLInputElement
    await waitFor(() => expect(host.value).toBe("smtp.example.com"))
    expect(host.disabled).toBe(true)
    expect(screen.getAllByText("Set by the server").length).toBeGreaterThan(0)
  })

  it("never pre-fills the password, and only asks for it when it is replaced", async () => {
    mockApi({ ...SAVED, password: { set: true, source: "console" } })
    render(<EmailSettingsForm canWrite />)
    await screen.findByText("Set")
    expect(screen.queryByLabelText("Password")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Replace" }))
    const password = (await screen.findByLabelText("Password")) as HTMLInputElement
    expect(password.value).toBe("")
  })

  it("sends the fields that changed and leaves the server's own alone", async () => {
    const calls = mockApi({
      ...SAVED,
      host: { value: "smtp.example.com", source: "server" },
      user: { value: "reports", source: "console" },
    })
    render(<EmailSettingsForm canWrite />)
    const user = (await screen.findByLabelText("User name")) as HTMLInputElement
    await waitFor(() => expect(user.value).toBe("reports"))
    fireEvent.change(user, { target: { value: "alerts" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    const body = JSON.parse(put?.body ?? "{}")
    expect(body.user).toBe("alerts")
    expect("host" in body).toBe(false)
  })

  it("shows the server's sentence after a test email", async () => {
    mockApi(SAVED, { sent: false, message: "Could not reach the mail server." })
    render(<EmailSettingsForm canWrite />)
    await screen.findByText("Not set")
    fireEvent.click(screen.getByRole("button", { name: "Send test email" }))
    await waitFor(() => expect(screen.getByText("Could not reach the mail server.")).toBeTruthy())
  })

  it("offers no Save to a caller without settings:write", async () => {
    mockApi(SAVED)
    render(<EmailSettingsForm canWrite={false} />)
    await screen.findByText("Not set")
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()
    expect(screen.getByText(/Only an administrator/i)).toBeTruthy()
  })
})
