/**
 * `WEB-18`: the History page is gone, and its old address redirects to
 * `/copilot?open=latest`, which opens the person's most recent conversation
 * (the chats are listed in that page's sidebar), or Home when there is none.
 *
 * `next/navigation` is stubbed at the module boundary with the address the
 * redirect lands on, and the API with a fetch stub for the chat list.
 */
const replace = mock<(href: string) => void>(() => {})
// One router for every render, as Next's is: the page's effects depend on it.
const router = { push: () => {}, replace, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }
const searchParams = new URLSearchParams("open=latest")
mock.module("next/navigation", () => ({
  usePathname: () => "/copilot",
  useSearchParams: () => searchParams,
  useRouter: () => router,
}))

import { cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { CopilotPage } from "./copilot-page"
import { CopilotProvider } from "./use-copilot"

const originalFetch = globalThis.fetch

function stubSessions(sessions: { id: string; title: string; mode: string }[], status = 200) {
  globalThis.fetch = (async (input: RequestInfo | URL) => {
    const url = String(input)
    if (url.startsWith("/api/ai/sessions")) {
      return new Response(JSON.stringify({ sessions, hasMore: false, nextOffset: null, counts: {} }), {
        status,
        headers: { "Content-Type": "application/json" },
      })
    }
    return new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } })
  }) as typeof fetch
}

beforeEach(() => replace.mockClear())
afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

describe("/copilot?open=latest, the old History address (WEB-18)", () => {
  it("opens the most recent conversation by its address", async () => {
    stubSessions([
      { id: "newest", title: "Quarterly revenue", mode: "ask" },
      { id: "older", title: "Orders pipeline", mode: "build" },
    ])
    render(
      <CopilotProvider>
        <CopilotPage />
      </CopilotProvider>,
    )
    await waitFor(() => expect(replace).toHaveBeenCalledWith("/copilot?id=newest"))
    // It never bounced to Home while it was asking.
    expect(replace).not.toHaveBeenCalledWith("/")
  })

  it("goes to Home when there is no conversation", async () => {
    stubSessions([])
    render(
      <CopilotProvider>
        <CopilotPage />
      </CopilotProvider>,
    )
    await waitFor(() => expect(replace).toHaveBeenCalledWith("/"))
    expect(replace).toHaveBeenCalledTimes(1)
  })

  it("goes to Home when the list cannot be loaded", async () => {
    stubSessions([], 500)
    render(
      <CopilotProvider>
        <CopilotPage />
      </CopilotProvider>,
    )
    await waitFor(() => expect(replace).toHaveBeenCalledWith("/"))
  })

  it("has no History button or panel of its own", async () => {
    stubSessions([])
    render(
      <CopilotProvider>
        <CopilotPage />
      </CopilotProvider>,
    )
    expect(screen.queryByRole("button", { name: /history/i })).toBeNull()
    expect(screen.queryByRole("complementary", { name: /history/i })).toBeNull()
    expect(screen.queryByRole("link", { name: /history/i })).toBeNull()
  })
})
