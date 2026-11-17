import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { SignInSettingsForm } from "./sign-in-settings"

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

/** What the server answers for the saved limits. */
const SAVED = {
  maxFailures: { value: 5 as number | null, source: "default" },
  failureWindowSecs: { value: 900 as number | null, source: "default" },
  lockoutSecs: { value: 300 as number | null, source: "default" },
  retentionDays: { value: 30 as number | null, source: "default" },
  embedMaxLifetimeSecs: { value: 86400 as number | null, source: "default" },
}

/** Answers the limits read and the save with `current`. */
function mockApi(current: unknown) {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ method, path: url, body: typeof init?.body === "string" ? init.body : null })
    return jsonResponse(current)
  }) as unknown as typeof fetch
  return calls
}

describe("SignInSettingsForm (ADM-8)", () => {
  it("shows a limit the server owns as read-only, and says why", async () => {
    mockApi({
      ...SAVED,
      maxFailures: { value: 7, source: "server" },
    })
    render(<SignInSettingsForm canWrite />)
    const maxFailures = (await screen.findByLabelText(
      "Failed sign-ins before the lock"
    )) as HTMLInputElement
    await waitFor(() => expect(maxFailures.value).toBe("7"))
    expect(maxFailures.disabled).toBe(true)
    expect(screen.getAllByText("Set by the server").length).toBeGreaterThan(0)
  })

  it("sends the limits that changed and leaves the server's own alone", async () => {
    const calls = mockApi({
      ...SAVED,
      maxFailures: { value: 7, source: "server" },
      retentionDays: { value: 14, source: "console" },
    })
    render(<SignInSettingsForm canWrite />)
    const retention = (await screen.findByLabelText(
      "Sign-in records kept (days)"
    )) as HTMLInputElement
    await waitFor(() => expect(retention.value).toBe("14"))
    fireEvent.change(retention, { target: { value: "21" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    const body = JSON.parse(put?.body ?? "{}")
    expect(body.retentionDays).toBe(21)
    expect("maxFailures" in body).toBe(false)
  })

  it("refuses a value outside the range, with the sentence that names it", async () => {
    const calls = mockApi(SAVED)
    render(<SignInSettingsForm canWrite />)
    const maxFailures = (await screen.findByLabelText(
      "Failed sign-ins before the lock"
    )) as HTMLInputElement
    fireEvent.change(maxFailures, { target: { value: "2" } })
    await waitFor(() =>
      expect(screen.getByText("maxFailures must be a whole number from 3 to 20.")).toBeTruthy()
    )
    expect(screen.getByRole("button", { name: "Save" }).hasAttribute("disabled")).toBe(true)
    expect(calls.some((c) => c.method === "PUT")).toBe(false)
  })

  it("offers no Save to a caller without settings:write", async () => {
    mockApi(SAVED)
    render(<SignInSettingsForm canWrite={false} />)
    await screen.findByLabelText("Failed sign-ins before the lock")
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()
    expect(screen.getByText(/Only an administrator/i)).toBeTruthy()
  })
})
