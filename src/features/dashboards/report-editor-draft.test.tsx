// BI-62 PR B (T8, plan D8): the assistant inside the report editor, on the
// real page with a fake conversation. `useCopilot` and `useCopilotStream`
// are stubbed with a small store: `send` records what it was asked and waits
// until the test answers, `chat.delta` streams text, and `stop` ends the
// answer as stopped. `mock.module` outlives this file in one `bun test`
// process, so the modules' own exports are put back in `afterAll`.
import * as React from "react"
import * as realNavigation from "next/navigation"
import * as realCopilot from "@/features/copilot/use-copilot"
import type { CopilotStreamValue, Msg } from "@/features/copilot/use-copilot"
import type { PageContext } from "@/features/copilot/page-context"
import * as realTileBody from "./tile-body"
import { afterAll, afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"

const realNavigationModule = { ...realNavigation }
const realCopilotModule = { ...realCopilot }
const realTileBodyModule = { ...realTileBody }

mock.module("next/navigation", () => ({
  usePathname: () => "/dashboards/reports/r1/edit",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({ push: () => {}, replace: () => {}, refresh: () => {}, back: () => {}, forward: () => {}, prefetch: () => {} }),
}))
mock.module("./tile-body", () => ({
  ...realTileBodyModule,
  TileBody: ({ spec }: { spec: { id: string } }) => <div data-testid={`tile-${spec.id}`} />,
}))

type Outcome = { content: string } | { error: string } | { stopped: true }
type Sent = { text: string; mode?: string; context?: string; history?: number }
type Snap = { busy: boolean; messages: Msg[]; error: string | null }

/** The fake conversation the page's one assistant is, for these tests. */
const chat: {
  snap: Snap
  stream: CopilotStreamValue
  context: PageContext | null
  sends: Sent[]
  stops: number
  settle: ((o: Outcome) => void) | null
  listeners: Set<() => void>
  set: (patch: Partial<Snap>) => void
  delta: (piece: string) => void
  reset: () => void
} = {
  snap: { busy: false, messages: [], error: null },
  stream: { draft: "", liveReasoning: "", progress: null },
  context: null,
  sends: [],
  stops: 0,
  settle: null,
  listeners: new Set(),
  set(patch) {
    chat.snap = { ...chat.snap, ...patch }
    chat.listeners.forEach((l) => l())
  },
  delta(piece) {
    chat.stream = { ...chat.stream, draft: chat.stream.draft + piece }
    chat.listeners.forEach((l) => l())
  },
  reset() {
    chat.snap = { busy: false, messages: [], error: null }
    chat.stream = { draft: "", liveReasoning: "", progress: null }
    chat.context = null
    chat.sends = []
    chat.stops = 0
    chat.settle = null
  },
}
const subscribe = (l: () => void) => {
  chat.listeners.add(l)
  return () => {
    chat.listeners.delete(l)
  }
}
async function send(text: string, history?: Msg[], options?: { mode?: "ask" | "build" }) {
  const q = text.trim()
  if (chat.snap.busy) return
  chat.sends.push({ text: q, mode: options?.mode, context: chat.context?.system, history: history?.length })
  const next: Msg[] = [...(history ?? chat.snap.messages), { role: "user", content: q }]
  chat.set({ messages: next, busy: true, error: null })
  await new Promise<void>((resolve) => {
    chat.settle = (o) => {
      chat.settle = null
      chat.stream = { ...chat.stream, draft: "" }
      if ("content" in o) chat.set({ busy: false, messages: [...next, { role: "assistant", content: o.content }] })
      else if ("stopped" in o) chat.set({ busy: false, messages: [...next, { role: "assistant", content: "", stopped: true }] })
      else chat.set({ busy: false, error: o.error })
      resolve()
    }
  })
}
const stop = () => {
  chat.stops += 1
  chat.settle?.({ stopped: true })
}
const setPageContext = (ctx: PageContext | null) => {
  chat.context = ctx
}

mock.module("@/features/copilot/use-copilot", () => ({
  ...realCopilotModule,
  useCopilot: () => {
    const base = realCopilotModule.useCopilot()
    const snap = React.useSyncExternalStore(subscribe, () => chat.snap)
    return React.useMemo(() => ({ ...base, ...snap, send, stop, setPageContext }), [base, snap])
  },
  useCopilotStream: () => {
    React.useSyncExternalStore(subscribe, () => chat.stream)
    return chat.stream
  },
}))

afterAll(() => {
  mock.module("next/navigation", () => realNavigationModule)
  mock.module("@/features/copilot/use-copilot", () => realCopilotModule)
  mock.module("./tile-body", () => realTileBodyModule)
})

const { act, cleanup, fireEvent, render, screen, waitFor, within } = await import("@testing-library/react")
const { AuthProvider } = await import("@/features/auth/auth-provider")
const { ReportEditorPage } = await import("./report-editor-page")
const { docToBlocks } = await import("@/lib/report-doc")
type Editor = import("@tiptap/core").Editor
type DocNode = import("@/lib/report-doc").DocNode

const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
type Call = { method: string; url: string; body?: unknown }

const report = {
  id: "r1",
  title: "Q3 visitors",
  folderId: "",
  createdBy: "Rina",
  createdAt: "2026-11-13T00:00:00Z",
  updatedBy: "Rina",
  updatedAt: "2026-11-13T00:00:00Z",
  blocks: [
    { type: "heading", text: "Summary", level: 1 },
    { type: "paragraph", text: "Visits rose." },
    { type: "chart", chart: "c_one", board: "b_1" },
  ],
}
const view = {
  report,
  charts: {
    c_one: {
      spec: { id: "c_one", title: "Visits by month", kind: "bar", source: "ui" },
      cell: { columns: ["month", "visits"], rows: [{ month: "Jan", visits: 10 }, { month: "Feb", visits: 12 }] },
      board: "b_1",
      boardName: "Sales",
    },
  },
  reporting: null,
  access: { level: "edit" },
  maxCharts: 30,
  pdfSupported: false,
  canDownload: false,
}
const LOADED = [
  { type: "heading", text: "Summary", level: 1 },
  { type: "paragraph", text: "Visits rose." },
  { type: "chart", chart: "c_one" },
]

/** A console with a person holding `permissions`; the report saves and every PUT is recorded. */
function serve(permissions: string[]) {
  const calls: Call[] = []
  spyOn(globalThis, "fetch").mockImplementation(((input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const call: Call = { method: init?.method ?? "GET", url, body: typeof init?.body === "string" ? JSON.parse(init.body) : undefined }
    calls.push(call)
    if (url.includes("/api/auth/me")) return Promise.resolve(json({ id: "u1", name: "Rina", email: null, roles: ["R"], permissions, tenants: [] }))
    if (url.startsWith("/api/dashboard/reports/r1") && call.method === "GET") return Promise.resolve(json(view))
    if (url.startsWith("/api/dashboard/reports/r1") && call.method === "PUT") return Promise.resolve(json({ ok: true, report: { ...report, ...(call.body as object) } }))
    if (url.startsWith("/api/dashboard/boards")) return Promise.resolve(json({ boards: [{ id: "b_1", name: "Sales" }] }))
    if (url.startsWith("/api/dashboard/specs")) return Promise.resolve(json({ charts: [{ id: "c_one", title: "Visits by month", board: "b_1", kind: "bar" }] }))
    if (url.includes("/api/dashboard/folders")) return Promise.resolve(json({ folders: [] }))
    return Promise.resolve(json({}))
  }) as unknown as typeof fetch)
  return calls
}
const saves = (calls: Call[]) => calls.filter((c) => c.method === "PUT" || c.method === "POST")

const WRITER = ["dashboard:read", "dashboard:write", "assistant:ask"]

async function openEditor(permissions = WRITER) {
  const calls = serve(permissions)
  const { container } = render(
    <AuthProvider>
      <ReportEditorPage id="r1" />
    </AuthProvider>,
  )
  const editor = await waitFor(() => {
    const el = container.querySelector(".ProseMirror") as (HTMLElement & { editor?: Editor }) | null
    if (!el?.editor) throw new Error("no editor yet")
    return el.editor
  })
  await screen.findByTestId("tile-c_one")
  return { calls, container, editor }
}

const blocks = (editor: Editor) => docToBlocks(editor.getJSON() as DocNode).blocks
const docJson = (editor: Editor) => JSON.stringify(editor.getJSON())

/** An empty line at the end with the cursor on it; answers the document's JSON then. */
function emptyLineAtEnd(editor: Editor): string {
  const end = editor.state.doc.content.size
  act(() => {
    editor.chain().insertContentAt(end, { type: "paragraph" }).setTextSelection(end + 1).run()
  })
  return docJson(editor)
}

/** Type "/" and pick "Ask AI to write…" from the menu. */
async function askAi(editor: Editor) {
  act(() => {
    editor.commands.insertContent("/")
  })
  const option = await screen.findByRole("option", { name: /Ask AI to write/ })
  act(() => {
    fireEvent.mouseDown(option)
  })
  return screen.findByLabelText("Tell AI Copilot what to write")
}

/** Leaving the page saves at once whatever is not saved yet. */
async function leave() {
  await act(async () => {
    window.dispatchEvent(new Event("pagehide"))
    await new Promise((r) => setTimeout(r, 20))
  })
}

beforeEach(() => chat.reset())
afterEach(() => {
  cleanup()
  mock.restore()
})

describe("Ask AI to write… (BI-62 D8)", () => {
  it("streams a draft from the one conversation in Ask mode, saves nothing, and Accept inserts the parsed blocks", async () => {
    const { calls, editor, container } = await openEditor()
    emptyLineAtEnd(editor)
    const input = await askAi(editor)
    // The prompt line has the three ideas of mockup D.
    expect(screen.getByRole("button", { name: "Summarise this report" })).toBeTruthy()
    fireEvent.change(input, { target: { value: "a short summary" } })
    act(() => {
      fireEvent.submit(input.closest("form")!)
    })
    await waitFor(() => expect(chat.sends).toHaveLength(1))
    expect(chat.sends[0].text).toBe("Write for this report: a short summary")
    expect(chat.sends[0].mode).toBe("ask")
    // Sent with the report as Markdown in the page context.
    expect(chat.sends[0].context).toContain('The user is writing the report "Q3 visitors" in the report editor (report id r1, 1 of 30 charts).')
    expect(chat.sends[0].context).toContain("# Summary\n\nVisits rose.\n\n[chart: Visits by month · Sales]")
    // The exchange is the page's one conversation (the AI Copilot tab reads it).
    expect(chat.snap.messages.map((m) => m.content)).toEqual(["Write for this report: a short summary"])

    act(() => chat.delta("Visits **rose**"))
    const strong = await screen.findByText("rose")
    expect(strong.tagName).toBe("STRONG")
    expect(container.querySelector("[data-phase=writing]")).toBeTruthy()
    expect(screen.getByRole("button", { name: /Stop/ })).toBeTruthy()

    await act(async () => chat.settle!({ content: "## In short\n\nVisits **rose** in June.\n\n- North\n- South" }))
    const accept = await screen.findByRole("button", { name: /Accept/ })
    // Nothing reaches the save service before Accept.
    await leave()
    expect(saves(calls)).toHaveLength(0)
    expect(blocks(editor)).toEqual(LOADED as never)

    act(() => {
      fireEvent.click(accept)
    })
    const accepted = [
      ...LOADED,
      { type: "heading", text: "In short", level: 2 },
      { type: "paragraph", text: "Visits **rose** in June." },
      { type: "list", items: ["North", "South"] },
    ]
    expect(blocks(editor)).toEqual(accepted as never)
    expect(container.querySelector("[data-ai-draft]")).toBeNull()
    await leave()
    await waitFor(() => expect(saves(calls)).toHaveLength(1))
    expect((saves(calls)[0].body as { blocks: unknown[] }).blocks).toEqual(accepted)
  })

  it("Discard mid-stream stops the answer first and leaves the document byte-identical", async () => {
    const { calls, editor, container } = await openEditor()
    const before = emptyLineAtEnd(editor)
    await askAi(editor)
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Write an introduction" }))
    })
    await waitFor(() => expect(chat.sends).toHaveLength(1))
    act(() => chat.delta("This report"))
    await screen.findByText(/This report/)
    await act(async () => {
      fireEvent.keyDown(within(container).getByRole("group", { name: "AI Copilot draft" }), { key: "Escape" })
    })
    expect(chat.stops).toBe(1)
    expect(container.querySelector("[data-ai-draft]")).toBeNull()
    expect(docJson(editor)).toBe(before)
    await leave()
    expect(saves(calls)).toHaveLength(0)
  })

  it("an empty prompt is discarded with its X and the document is as before", async () => {
    const { editor } = await openEditor()
    const before = emptyLineAtEnd(editor)
    await askAi(editor)
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Discard" }))
    })
    expect(docJson(editor)).toBe(before)
    expect(chat.sends).toHaveLength(0)
  })
})

describe("Describe (BI-62 D8)", () => {
  it("sends the chart with the rows the page drew it with; an error shows one line, Retry asks again, Accept puts it under the chart", async () => {
    const { editor, container } = await openEditor()
    const card = container.querySelector('[data-chart-id="c_one"]') as HTMLElement
    act(() => {
      fireEvent.click(within(card).getByRole("button", { name: "Describe" }))
    })
    await waitFor(() => expect(chat.sends).toHaveLength(1))
    expect(chat.sends[0].text).toBe("Describe the chart “Visits by month” for this report")
    expect(chat.sends[0].mode).toBe("ask")
    expect(chat.sends[0].context).toContain(
      'The chart to describe: "Visits by month" (chart id c_one) from the dashboard "Sales". Its rows as the reader sees them: rows: Jan=10, Feb=12.',
    )
    expect(screen.getByText("AI Copilot · about Visits by month")).toBeTruthy()

    await act(async () => chat.settle!({ error: "The model is not reachable." }))
    expect((await screen.findByRole("alert")).textContent).toBe("The model is not reachable.")
    expect(screen.queryByRole("button", { name: /Accept/ })).toBeNull()
    expect(blocks(editor)).toEqual(LOADED as never)

    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /Retry/ }))
    })
    await waitFor(() => expect(chat.sends).toHaveLength(2))
    // The failed attempt is replaced, not repeated after itself.
    expect(chat.sends[1].history).toBe(0)
    await act(async () => chat.settle!({ content: "Visits rose from 10 in January to 12 in February." }))
    act(() => {
      fireEvent.click(screen.getByRole("button", { name: /Accept/ }))
    })
    expect(blocks(editor)).toEqual([...LOADED, { type: "paragraph", text: "Visits rose from 10 in January to 12 in February." }] as never)
  })

  it("Stop leaves nothing to accept: the draft says Stopped, with Retry", async () => {
    const { container } = await openEditor()
    const card = container.querySelector('[data-chart-id="c_one"]') as HTMLElement
    act(() => {
      fireEvent.click(within(card).getByRole("button", { name: "Describe" }))
    })
    await waitFor(() => expect(chat.sends).toHaveLength(1))
    act(() => chat.delta("Visits rose"))
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: /Stop/ }))
    })
    expect(chat.stops).toBe(1)
    expect((await screen.findByRole("alert")).textContent).toBe("Stopped")
    expect(screen.queryByRole("button", { name: /Accept/ })).toBeNull()
    expect(screen.getByRole("button", { name: /Retry/ })).toBeTruthy()
  })
})

describe("one draft at a time (BI-62 D8)", () => {
  it("while AI Copilot answers, Ask AI and Describe stay visible but disabled, with the reason", async () => {
    const { editor, container } = await openEditor()
    act(() => chat.set({ busy: true }))
    emptyLineAtEnd(editor)
    act(() => {
      editor.commands.insertContent("/")
    })
    const option = await screen.findByRole("option", { name: /Ask AI to write/ })
    expect(option.getAttribute("aria-disabled")).toBe("true")
    expect(option.textContent).toContain("Wait for AI Copilot to finish its answer")
    act(() => {
      fireEvent.mouseDown(option)
    })
    expect(screen.queryByLabelText("Tell AI Copilot what to write")).toBeNull()
    const describe = within(container.querySelector('[data-chart-id="c_one"]') as HTMLElement).getByRole("button", { name: "Describe" })
    expect(describe.getAttribute("aria-disabled")).toBe("true")
    act(() => {
      fireEvent.click(describe)
    })
    expect(chat.sends).toHaveLength(0)
    expect(container.querySelector("[data-ai-draft]")).toBeNull()
  })

  it("while a draft is open, a second one cannot start", async () => {
    const { container } = await openEditor()
    const card = container.querySelector('[data-chart-id="c_one"]') as HTMLElement
    act(() => {
      fireEvent.click(within(card).getByRole("button", { name: "Describe" }))
    })
    await waitFor(() => expect(chat.sends).toHaveLength(1))
    await act(async () => chat.settle!({ content: "Visits rose." }))
    await screen.findByRole("button", { name: /Accept/ })
    const describe = within(card).getByRole("button", { name: "Describe" })
    expect(describe.getAttribute("aria-disabled")).toBe("true")
    act(() => {
      fireEvent.click(describe)
    })
    expect(chat.sends).toHaveLength(1)
    expect(container.querySelectorAll("[data-ai-draft]")).toHaveLength(1)
  })

  it("offers neither to a person without the assistant", async () => {
    const { editor, container } = await openEditor(["dashboard:read", "dashboard:write"])
    emptyLineAtEnd(editor)
    act(() => {
      editor.commands.insertContent("/")
    })
    expect(await screen.findByRole("option", { name: /Heading 1/ })).toBeTruthy()
    expect(screen.queryByRole("option", { name: /Ask AI to write/ })).toBeNull()
    expect(within(container.querySelector('[data-chart-id="c_one"]') as HTMLElement).queryByRole("button", { name: "Describe" })).toBeNull()
  })
})
