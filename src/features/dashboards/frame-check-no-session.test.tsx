import { cleanup, render, waitFor } from "@testing-library/react"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

import { __resetApiFetchRedirectStateForTests } from "@/services/http"
import { stubResizeObserver } from "@/lib/test-layout"
import { PublicDashboard } from "./public-dashboard"

/**
 * `PR #651 review 1` (BLOCKER): a public link and a signed embed have no
 * session, and the frame-check route needs one. The web card must not ask
 * there: the 401 would go through `apiFetch`'s redirect to `/login`
 * (`src/services/http.ts`) and carry the anonymous viewer off the page they
 * are reading. Each test below serves the page's own public routes, answers
 * 401 for everything else — the frame check among them — and asserts three
 * things: the card's frame is drawn (the card really rendered), no request
 * went to `/api/dashboard/frame-check`, and no navigation was performed.
 *
 * The card reads the page's own address to know there is no session, so the
 * test only sets the address (`happyDOM.setURL`, as `public-dashboard.test.tsx`
 * does for relative URLs). No module is mocked: `mock.module` of
 * `next/navigation` here broke `chart-builder-page.test.tsx`'s own mock of
 * it in the same process, and nothing in this file needs a Next router.
 */

// happy-dom would otherwise try to load the page a frame points at; the test
// reads the frame only.
const happyDOM = (window as unknown as { happyDOM?: { settings: { disableIframePageLoading: boolean } } }).happyDOM
if (happyDOM) happyDOM.settings.disableIframePageLoading = true

// The grid measures itself with a ResizeObserver, which happy-dom does not
// provide; the repo's own stand-in is used (`stubResizeObserver`, rule 4),
// put back after the file so a later file sees what was there.
const undoResizeObserver = stubResizeObserver()

const originalFetch = global.fetch

beforeEach(() => {
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
    "http://localhost/",
  )
})

afterEach(() => {
  cleanup()
  global.fetch = originalFetch
  __resetApiFetchRedirectStateForTests()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

/** A web-page card on an allowed site: the one tile this file is about. */
const WEB_CARD = { id: "c1", title: "Status", kind: "web", def: { url: "https://status.example/page" } }

/** The reporting a public or embed payload carries, narrowed to the board's sites. */
const REPORTING = {
  timeZone: "UTC", weekStart: "monday" as const, numberLocale: "en-US" as const,
  companyPalette: [], fonts: [], contentSites: ["https://status.example"],
}

function payload() {
  return {
    board: { id: "b1", name: "Sales" },
    layout: {},
    charts: [WEB_CARD],
    results: {},
    reporting: REPORTING,
  }
}

/**
 * Serves the page's own public routes and answers 401 for everything else,
 * as a session-only route does for an anonymous visitor. Every URL asked is
 * recorded, so a test can say the frame-check route was never one of them.
 */
function serveBoardRoute(boardUrl: string): string[] {
  const calls: string[] = []
  global.fetch = mock(async (input: RequestInfo | URL) => {
    const url = String(input)
    calls.push(url)
    if (url.includes("/api/branding")) {
      return json({ workspaceName: null, logo: null, logoDark: null, icon: null })
    }
    if (url.includes(boardUrl)) return json(payload())
    if (url.includes("/api/embed/frame")) return json({ origins: [] })
    if (url.includes("/api/embed/values")) return json({ column: null, values: [], truncated: false })
    return new Response("", { status: 401 })
  }) as unknown as typeof fetch
  return calls
}

/** Watches the navigation that `apiFetch`'s 401 path would perform. */
function watchAssign() {
  return spyOn(window.location, "assign").mockImplementation(() => {})
}

describe("a web-page card on a page without a session (PR #651 review 1)", () => {
  it("a public link draws the frame and never asks the session-only frame check", async () => {
    ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
      "http://localhost/public/dashboard/tok",
    )
    const calls = serveBoardRoute("/api/public/dashboard/")
    const assign = watchAssign()
    __resetApiFetchRedirectStateForTests()
    const { container } = render(<PublicDashboard token="tok" />)

    // The card's frame is drawn: the "no claim" answer keeps the iframe, as
    // before the frame check existed.
    await waitFor(() => expect(container.querySelector("iframe")).not.toBeNull())
    expect(calls.some((url) => url.includes("/api/dashboard/frame-check"))).toBe(false)
    expect(assign).not.toHaveBeenCalled()
    assign.mockRestore()
  })

  it("a signed embed draws the frame and never asks the session-only frame check", async () => {
    ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
      "http://localhost/embed/dashboard/tok",
    )
    const calls = serveBoardRoute("/api/embed/data")
    const assign = watchAssign()
    __resetApiFetchRedirectStateForTests()
    const { EmbedView } = await import("./embed-view")
    const { container } = render(<EmbedView jwt="j" />)

    await waitFor(() => expect(container.querySelector("iframe")).not.toBeNull())
    expect(calls.some((url) => url.includes("/api/dashboard/frame-check"))).toBe(false)
    expect(assign).not.toHaveBeenCalled()
    assign.mockRestore()
  })

  // The ask still happens on a page that has a session: the check's whole
  // point. The 401 stand-in here plays a session that expired while the page
  // was open, and the redirect that follows is `apiFetch`'s standard expiry
  // behaviour — which is precisely why the no-session pages above must not
  // ask at all.
  it("asks on a console page, where a session is expected", async () => {
    const calls = serveBoardRoute("/api/public/dashboard/")
    const assign = watchAssign()
    __resetApiFetchRedirectStateForTests()
    const { container } = render(<PublicDashboard token="tok" />)

    await waitFor(() => expect(container.querySelector("iframe")).not.toBeNull())
    expect(calls.some((url) => url.includes("/api/dashboard/frame-check"))).toBe(true)
    expect(assign).toHaveBeenCalledTimes(1)
    assign.mockRestore()
  })
})

// `bun test src` runs every file in one process: the stand-ins this file set
// (the ResizeObserver) go back here, or a later file inherits them.
afterAll(() => {
  undoResizeObserver()
})
