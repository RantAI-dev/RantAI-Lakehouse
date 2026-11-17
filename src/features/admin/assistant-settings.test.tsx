import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import { AssistantSettingsForm } from "./assistant-settings"

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

/** What the server answers for the saved provider. */
const SAVED = {
  url: { value: "https://api.example.com/v1" as string | null, source: "default" },
  model: { value: "example-model" as string | null, source: "default" },
  key: { set: false, source: "default" },
}

/**
 * Answers the provider read with `current`, the save with the same, and the
 * test with `testResult`.
 */
function mockApi(current: unknown, testResult: unknown = { answered: true, message: "The assistant answered." }) {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url
    const method = (init?.method ?? "GET").toUpperCase()
    calls.push({ method, path: url, body: typeof init?.body === "string" ? init.body : null })
    if (url.includes("/api/settings/assistant/test")) return jsonResponse(testResult)
    return jsonResponse(current)
  }) as unknown as typeof fetch
  return calls
}

describe("AssistantSettingsForm (ADM-9)", () => {
  it("shows a setting the server owns as read-only, and says why", async () => {
    mockApi({
      ...SAVED,
      url: { value: "https://env.example.com/v1", source: "server" },
      model: { value: "env-model", source: "server" },
      key: { set: true, source: "server" },
    })
    render(<AssistantSettingsForm canWrite />)
    const url = (await screen.findByLabelText("Provider address")) as HTMLInputElement
    await waitFor(() => expect(url.value).toBe("https://env.example.com/v1"))
    expect(url.disabled).toBe(true)
    expect(screen.getAllByText("Set by the server").length).toBeGreaterThan(0)
  })

  it("never pre-fills the key, and only asks for it when it is replaced", async () => {
    mockApi({ ...SAVED, key: { set: true, source: "console" } })
    render(<AssistantSettingsForm canWrite />)
    await screen.findByText("Set")
    expect(screen.queryByLabelText("Key")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Replace" }))
    const key = (await screen.findByLabelText("Key")) as HTMLInputElement
    expect(key.value).toBe("")
  })

  it("sends the fields that changed and leaves the server's own alone", async () => {
    const calls = mockApi({
      ...SAVED,
      url: { value: "https://env.example.com/v1", source: "server" },
      model: { value: "example-model", source: "console" },
    })
    render(<AssistantSettingsForm canWrite />)
    const model = (await screen.findByLabelText("Model")) as HTMLInputElement
    await waitFor(() => expect(model.value).toBe("example-model"))
    fireEvent.change(model, { target: { value: "another-model" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")
    const body = JSON.parse(put?.body ?? "{}")
    expect(body.model).toBe("another-model")
    expect("url" in body).toBe(false)
  })

  it("shows the server's sentence after a test", async () => {
    mockApi(SAVED, { answered: false, message: "Could not reach the provider." })
    render(<AssistantSettingsForm canWrite />)
    await screen.findByText("Not set")
    fireEvent.click(screen.getByRole("button", { name: "Test the provider" }))
    await waitFor(() => expect(screen.getByText("Could not reach the provider.")).toBeTruthy())
  })

  it("offers no Save to a caller without settings:write", async () => {
    mockApi(SAVED)
    render(<AssistantSettingsForm canWrite={false} />)
    await screen.findByText("Not set")
    expect(screen.queryByRole("button", { name: "Save" })).toBeNull()
    expect(screen.getByText(/Only an administrator/i)).toBeTruthy()
  })
})
