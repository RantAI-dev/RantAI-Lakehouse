/**
 * `AI-24` D2: the title bar of `/copilot` shows the open conversation's own
 * title, not a lookup in the newest 20 conversations the history menus list —
 * a conversation reopened from History is usually older than those.
 *
 * `next/navigation` is stubbed at the module boundary: the page reads the
 * address (`useSearchParams`) to open the conversation a link names, and
 * rewrites it (`useRouter`), and neither hook has a Router context here.
 */
mock.module("next/navigation", () => ({
  usePathname: () => "/copilot",
  useSearchParams: () => new URLSearchParams("id=old"),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { act, cleanup, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import * as React from "react"
import { CopilotPage } from "./copilot-page"
import { CopilotProvider, useCopilot } from "./use-copilot"

const originalFetch = globalThis.fetch

beforeEach(() => {
  globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = typeof input === "string" ? input : String(input)
    if (init?.method === "PATCH") {
      const sent = JSON.parse(String(init.body)) as { id?: string; title?: string }
      return {
        ok: true,
        status: 200,
        json: async () => ({ ok: true, id: sent.id, title: sent.title }),
      } as unknown as Response
    }
    if (url.startsWith("/api/ai/sessions?") && url.includes("id=old")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({
          session: {
            id: "old",
            title: "Quarterly revenue",
            mode: "ask",
            messages: [{ id: "u1", role: "user", content: "How was revenue?" }],
          },
        }),
      } as unknown as Response
    }
    if (url.startsWith("/api/ai/sessions")) {
      return {
        ok: true,
        status: 200,
        json: async () => ({ sessions: [], hasMore: false, nextOffset: null, counts: {} }),
      } as unknown as Response
    }
    return { ok: true, status: 200, json: async () => ({}) } as unknown as Response
  }) as typeof fetch
})

afterEach(() => {
  cleanup()
  globalThis.fetch = originalFetch
})

type Copilot = ReturnType<typeof useCopilot>
let copilot: Copilot | null = null

function Harness() {
  const api = useCopilot()
  React.useEffect(() => {
    copilot = api
  }, [api])
  return null
}

function renderPage() {
  render(
    <CopilotProvider>
      <Harness />
      <CopilotPage />
    </CopilotProvider>,
  )
}

describe("the /copilot title bar (AI-24)", () => {
  it("shows the opened conversation's title, though it is not among the newest 20", async () => {
    renderPage()
    await waitFor(() => expect(copilot?.sessionId).toBe("old"))
    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Quarterly revenue"))
  })

  it("shows the new title as soon as the open conversation is renamed", async () => {
    renderPage()
    await waitFor(() => expect(copilot?.sessionId).toBe("old"))

    await act(async () => {
      await copilot!.renameSession("old", "Board plan")
    })

    await waitFor(() => expect(screen.getByRole("heading", { level: 1 }).textContent).toBe("Board plan"))
  })
})
