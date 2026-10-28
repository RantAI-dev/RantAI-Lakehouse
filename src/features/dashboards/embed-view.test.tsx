import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock } from "bun:test"
import * as realThemes from "next-themes"
import * as realTileBody from "./tile-body"
import * as realGrid from "./dashboard-grid"

// BI-26 T4 (plan D6, D7). `mock.module` outlives this file in `bun test src`, so the real exports
// are kept and put back in `afterAll`, as in dashboard-look.test.tsx.
const themes = { ...realThemes }
const tileBody = { ...realTileBody }
const grid = { ...realGrid }
mock.module("next-themes", () => ({ ...themes, useTheme: () => ({ resolvedTheme: "light", setTheme: () => {} }) }))
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: (props: { dark: boolean }) => <div data-testid="tile" data-dark={String(props.dark)} />,
}))
mock.module("./dashboard-grid", () => ({
  ...grid,
  DashboardGrid: ({ items }: { items: { id: string; body: React.ReactNode }[] }) => <div>{items.map((i) => <div key={i.id}>{i.body}</div>)}</div>,
}))
afterAll(() => {
  mock.module("next-themes", () => themes)
  mock.module("./tile-body", () => tileBody)
  mock.module("./dashboard-grid", () => grid)
})

const { EmbedView } = await import("./embed-view")
const { DEFAULT_EMBED_LOOK } = await import("@/lib/embed-look")

const originalFetch = global.fetch
const originalParent = Object.getOwnPropertyDescriptor(window, "parent")
const originalRO = (globalThis as { ResizeObserver?: unknown }).ResizeObserver
afterEach(() => {
  cleanup()
  global.fetch = originalFetch
  if (originalParent) Object.defineProperty(window, "parent", originalParent)
  ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver = originalRO
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

const CHART = { id: "c1", title: "Visits", kind: "bar", mart: "m", x: "k", y: "v", source: "ui" }

function payload(extra: Record<string, unknown> = {}, board: Record<string, unknown> = {}) {
  return {
    board: { id: "b1", name: "Visits", ...board },
    layout: {},
    charts: [CHART],
    results: {},
    reporting: { timeZone: "UTC", weekStart: "monday" },
    ...extra,
  }
}

type Call = { url: string; body: Record<string, unknown> }

/** Serves the embed's three routes and records what was asked. */
function serve(opts: { data?: unknown; origins?: string[]; values?: string[] } = {}): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const body = init?.body ? JSON.parse(String(init.body)) : {}
    calls.push({ url, body })
    if (url === "/api/embed/frame") return json({ origins: opts.origins ?? [] })
    if (url === "/api/embed/values") return json({ column: body.column, values: opts.values ?? [], truncated: false })
    return json(opts.data ?? payload())
  }) as unknown as typeof fetch
  return calls
}

describe("the look in the embed address (BI-26 D6)", () => {
  it("shows our name on a single chart, and logo=none hides it", async () => {
    serve()
    render(<EmbedView jwt="j" chartId="c1" />)
    await screen.findByText("Rantai Lake")
    cleanup()
    serve()
    render(<EmbedView jwt="j" chartId="c1" look={{ ...DEFAULT_EMBED_LOOK, showName: false }} />)
    await screen.findByText("Visits")
    expect(screen.queryByText("Rantai Lake")).toBeNull()
  })

  it("theme=dark draws a light dashboard dark, theme=light a dark one light, 'dashboard' keeps its own", async () => {
    serve({ data: payload({}, { theme: "light" }) })
    const { container, unmount } = render(<EmbedView jwt="j" look={{ ...DEFAULT_EMBED_LOOK, theme: "dark" }} />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("true"))
    expect((container.firstElementChild as HTMLElement).className).toContain("dark")
    unmount()
    serve({ data: payload({}, { theme: "dark" }) })
    render(<EmbedView jwt="j" look={{ ...DEFAULT_EMBED_LOOK, theme: "light" }} />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("false"))
    cleanup()
    serve({ data: payload({}, { theme: "dark" }) })
    render(<EmbedView jwt="j" />)
    await waitFor(() => expect(screen.getByTestId("tile").getAttribute("data-dark")).toBe("true"))
  })

  it("background=transparent makes the page and the frame transparent", async () => {
    serve()
    const { container } = render(<EmbedView jwt="j" chartId="c1" look={{ ...DEFAULT_EMBED_LOOK, transparent: true }} />)
    await screen.findByText("Visits")
    const root = container.firstElementChild as HTMLElement
    expect(root.className).toContain("!bg-transparent")
    expect((root.firstElementChild as HTMLElement).className).toContain("bg-transparent")
    expect((root.firstElementChild as HTMLElement).className).not.toContain("bg-card")
  })

  it("a chart token's payload is drawn as one chart filling the frame", async () => {
    serve({ data: payload({ onlyChart: "c1" }) })
    const { container } = render(<EmbedView jwt="j" />)
    await screen.findByText("Visits")
    expect((container.firstElementChild as HTMLElement).className).toContain("h-screen")
  })
})

describe("editable filters in a signed embed (BI-26 D4, D5)", () => {
  const filters = [{ column: "region", kind: "text", filter: { values: ["Bali"] } }]

  it("lists the allowed values, starts at the saved value, and asks again with the viewer's choice", async () => {
    const calls = serve({ data: payload({ embedFilters: filters }), values: ["Bali", "Java"] })
    render(<EmbedView jwt="the.jwt" />)
    const select = (await screen.findByLabelText("region")) as HTMLSelectElement
    await waitFor(() => expect(select.options.length).toBe(3))
    expect(select.value).toBe("Bali")
    expect(calls.some((c) => c.url === "/api/embed/values" && c.body.column === "region" && c.body.jwt === "the.jwt")).toBe(true)

    fireEvent.change(select, { target: { value: "Java" } })
    await waitFor(() =>
      expect(calls.some((c) => c.url === "/api/embed/data" && JSON.stringify(c.body.filters) === JSON.stringify([{ column: "region", op: "in", values: ["Java"] }]))).toBe(true),
    )
    // "All" clears it: an inactive filter replaces the saved one.
    fireEvent.change(select, { target: { value: "" } })
    await waitFor(() =>
      expect(calls.some((c) => c.url === "/api/embed/data" && JSON.stringify(c.body.filters) === JSON.stringify([{ column: "region", op: "in", values: [] }]))).toBe(true),
    )
  })

  it("shows no controls when the payload describes no editable filter", async () => {
    serve()
    render(<EmbedView jwt="j" />)
    await screen.findByTestId("tile")
    expect(screen.queryByRole("group", { name: "Filters" })).toBeNull()
  })

  it("a public-link embed has no filter controls", async () => {
    const calls = serve({ data: payload({ embedFilters: filters }) })
    render(<EmbedView token="p_tok" />)
    await screen.findByTestId("tile")
    expect(screen.queryByLabelText("region")).toBeNull()
    expect(calls.some((c) => c.url === "/api/embed/values")).toBe(false)
  })
})

describe("height messages (BI-26 D7, decision 9)", () => {
  function frameUnderTest() {
    const posted: { message: unknown; origin: string }[] = []
    Object.defineProperty(window, "parent", {
      configurable: true,
      value: { postMessage: (message: unknown, origin: string) => posted.push({ message, origin }) },
    })
    let notify: () => void = () => {}
    ;(globalThis as { ResizeObserver?: unknown }).ResizeObserver = class {
      constructor(cb: () => void) { notify = cb }
      observe() {}
      disconnect() {}
    }
    return { posted, resize: () => notify() }
  }

  it("posts the content height once to each allowed site, as its own target origin", async () => {
    const { posted, resize } = frameUnderTest()
    serve({ origins: ["https://a.example", "https://b.example"] })
    render(<EmbedView jwt="j" />)
    await screen.findByTestId("tile")
    await waitFor(() => expect(posted.length).toBe(2), { timeout: 2000 })
    expect(posted.map((p) => p.origin).sort()).toEqual(["https://a.example", "https://b.example"])
    expect(posted.every((p) => (p.message as { type: string }).type === "rantai-embed:size")).toBe(true)
    expect(typeof (posted[0].message as { height: number }).height).toBe("number")
    // The same height again is not sent again.
    resize()
    await new Promise((r) => setTimeout(r, 250))
    expect(posted.length).toBe(2)
  })

  it("posts nothing while the payload is still loading, then the real height (BI-26 review fix R5)", async () => {
    const { posted } = frameUnderTest()
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => { release = r })
    global.fetch = mock(async (input: RequestInfo | URL) => {
      if (String(input) === "/api/embed/frame") return json({ origins: ["https://a.example"] })
      await gate
      return json(payload())
    }) as unknown as typeof fetch
    render(<EmbedView jwt="j" />)
    await new Promise((r) => setTimeout(r, 400))
    expect(posted.length).toBe(0)
    release()
    await screen.findByTestId("tile")
    await waitFor(() => expect(posted.length).toBe(1), { timeout: 2000 })
  })

  it("posts nothing for a chart token, whose payload says it is one chart (BI-26 review fix R5)", async () => {
    const { posted } = frameUnderTest()
    serve({ origins: ["https://a.example"], data: payload({ onlyChart: "c1" }) })
    render(<EmbedView jwt="j" />)
    await screen.findByText("Visits")
    await new Promise((r) => setTimeout(r, 350))
    expect(posted.length).toBe(0)
  })

  it("posts nothing when no site is allowed", async () => {
    const { posted } = frameUnderTest()
    serve({ origins: [] })
    render(<EmbedView jwt="j" />)
    await screen.findByTestId("tile")
    await new Promise((r) => setTimeout(r, 350))
    expect(posted.length).toBe(0)
  })

  it("a single chart fills the frame and reports no height of its own", async () => {
    const { posted } = frameUnderTest()
    serve({ origins: ["https://a.example"] })
    render(<EmbedView jwt="j" chartId="c1" />)
    await screen.findByText("Visits")
    await new Promise((r) => setTimeout(r, 350))
    expect(posted.length).toBe(0)
  })
})
