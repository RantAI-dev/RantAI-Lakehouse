import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import * as React from "react"
import { SERVER_SEARCH_ABOVE, TableFinder, WINDOW_ABOVE, finderKind } from "./table-finder"
import { useColumnPreviews } from "./use-discovery"

/**
 * SRC-13 decisions D1 to D9 in the console: one Find flow for every type, the orchestrator's progress, the
 * server-side search past a page, windowed rows, and the columns panel with its "inferred from a sample" mark.
 */

const originalFetch = global.fetch
afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

const json = (body: unknown, status = 200) =>
  new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

type Call = { url: string; method: string; body: unknown }
type Handler = (call: Call) => Response | undefined

/** A fetch that records every call and answers by `handler`; an unanswered call is a test bug. */
function stub(handler: Handler): Call[] {
  const calls: Call[] = []
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const call: Call = {
      url: String(input),
      method: init?.method ?? "GET",
      body: init?.body ? JSON.parse(init.body as string) : undefined,
    }
    calls.push(call)
    const answer = handler(call)
    if (!answer) throw new Error(`unexpected fetch: ${call.method} ${call.url}`)
    return answer
  }) as unknown as typeof fetch
  return calls
}

const LIST = "/api/connectors/c1/discovery/list"
const PREVIEW = "/api/connectors/c1/discovery/preview"

function Harness({
  adapter,
  dial,
  single = false,
  selected = [],
  onToggle = () => {},
  onSetMany = () => {},
}: {
  adapter: string
  dial: Record<string, unknown>
  single?: boolean
  selected?: string[]
  onToggle?: (name: string, label?: string) => void
  onSetMany?: (names: string[], on: boolean, labels?: Record<string, string>) => void
}) {
  const previews = useColumnPreviews("c1", 5)
  return (
    <TableFinder
      connectorId="c1"
      adapter={adapter}
      dial={dial}
      defaultSchema="public"
      previews={previews}
      selectedNames={new Set(selected)}
      onToggle={onToggle}
      onSetMany={onSetMany}
      single={single}
      pollMs={5}
    />
  )
}

const tables = (n: number) =>
  Array.from({ length: n }, (_, i) => ({ name: `public.t${String(i).padStart(5, "0")}`, kind: "table" }))

describe("finderKind", () => {
  it("gives every type but Google Sheets a Find flow, and tells Oracle apart", () => {
    expect(finderKind("sql", { driver: "postgres" })).toBe("sql")
    expect(finderKind("cdc", { driver: "postgres" })).toBe("sql")
    expect(finderKind("sql", { driver: "oracle" })).toBe("oracle")
    for (const adapter of ["files", "sftp", "rest", "mongodb", "kafka"]) {
      expect(finderKind(adapter, {})).toBe(adapter as never)
    }
    expect(finderKind("sheets", {})).toBeNull()
    // Oracle change capture lists nothing (Amendment 10, S10-c): its tables are typed by hand.
    expect(finderKind("cdc", { driver: "oracle" })).toBeNull()
  })
})

describe("TableFinder, SQL sources", () => {
  it("lists the schemas into the schema field's choices, and still takes a typed one", async () => {
    const calls = stub((c) =>
      c.url === LIST
        ? json({ kind: "list", state: "done", objects: [{ name: "public", kind: "schema" }, { name: "sales", kind: "schema" }] })
        : undefined
    )
    const { container } = render(<Harness adapter="sql" dial={{ driver: "postgres" }} />)
    fireEvent.click(screen.getByRole("button", { name: "List schemas" }))
    await waitFor(() => expect(container.querySelectorAll("#discover-schemas option").length).toBe(2))
    expect([...container.querySelectorAll("#discover-schemas option")].map((o) => o.getAttribute("value"))).toEqual([
      "public",
      "sales",
    ])
    // No schema is asked for: that is how the API knows to answer the schemas.
    expect(calls[0]?.body).toEqual({})
    fireEvent.change(screen.getByLabelText("Schema"), { target: { value: "typed_schema" } })
    expect((screen.getByLabelText("Schema") as HTMLInputElement).value).toBe("typed_schema")
  })

  it("says why a listing failed in the API's fixed words, and nothing from the source", async () => {
    stub(() => json({ kind: "list", state: "failed", errorClass: "sign_in_refused", reason: "The source refused the sign-in." }))
    render(<Harness adapter="sql" dial={{ driver: "postgres" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    expect(await screen.findByText("Cannot list tables · The source refused the sign-in.")).toBeDefined()
  })

  it("filters a whole short list here, without asking the server again", async () => {
    const calls = stub((c) => (c.url === LIST ? json({ kind: "list", state: "done", objects: tables(12) }) : undefined))
    render(<Harness adapter="sql" dial={{ driver: "postgres" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    await screen.findByText("public.t00000")
    fireEvent.change(screen.getByLabelText("Filter tables"), { target: { value: "T0000" } })
    expect(screen.getByText("public.t00001")).toBeDefined()
    expect(screen.queryByText("public.t00010")).toBeNull()
    expect(calls.length).toBe(1)
  })
})

describe("TableFinder, long lists (D9)", () => {
  it("draws only the rows in view of a list far past a page", async () => {
    stub((c) => (c.url === LIST ? json({ kind: "list", state: "done", objects: tables(5000) }) : undefined))
    const { container } = render(<Harness adapter="sql" dial={{ driver: "postgres" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    await screen.findByTestId("windowed-rows")
    const rows = container.querySelectorAll("[data-testid=windowed-rows] li")
    expect(rows.length).toBeGreaterThan(0)
    expect(rows.length).toBeLessThan(100)
    expect(WINDOW_ABOVE).toBeLessThan(5000)
  })

  it("searches on the server once the list is past a page, after typing pauses, and shows what it returns", async () => {
    const calls = stub((c) => {
      if (c.url !== LIST) return undefined
      const search = (c.body as { search?: string }).search
      return search
        ? json({ kind: "list", state: "done", objects: [{ name: "public.t04242", kind: "table" }] })
        : json({ kind: "list", state: "done", objects: tables(SERVER_SEARCH_ABOVE + 1) })
    })
    render(<Harness adapter="sql" dial={{ driver: "postgres" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    await screen.findByLabelText("Filter tables")
    fireEvent.change(screen.getByLabelText("Filter tables"), { target: { value: "4242" } })
    // Typing is debounced: nothing has been sent yet.
    expect(calls.filter((c) => (c.body as { search?: string }).search).length).toBe(0)
    await waitFor(() => expect(screen.getByText("public.t04242")).toBeDefined(), { timeout: 3000 })
    const searched = calls.filter((c) => (c.body as { search?: string }).search)
    expect(searched.map((c) => c.body)).toEqual([{ schema: "public", search: "4242" }])
    // Clearing the search brings the whole list back, with no new request.
    fireEvent.change(screen.getByLabelText("Filter tables"), { target: { value: "" } })
    await screen.findByText("public.t00000")
    expect(calls.length).toBe(2)
  })

  it("says when the list was cut, in the API's words, and searches the whole source on the server", async () => {
    const calls = stub((c) => {
      if (c.url !== LIST) return undefined
      return (c.body as { search?: string }).search
        ? json({ kind: "list", state: "done", objects: [{ name: "public.found_it", kind: "table" }], truncated: false })
        : json({
            kind: "list",
            state: "done",
            objects: tables(20),
            truncated: true,
            reason: "Only the first 10,000 are shown. Search to find the others.",
          })
    })
    render(<Harness adapter="sql" dial={{ driver: "postgres" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    expect(await screen.findByText("Only the first 10,000 are shown. Search to find the others.")).toBeDefined()
    fireEvent.change(screen.getByLabelText("Filter tables"), { target: { value: "found" } })
    await waitFor(() => expect(screen.getByText("public.found_it")).toBeDefined(), { timeout: 3000 })
    expect(calls.some((c) => (c.body as { search?: string }).search === "found")).toBe(true)
  })
})

describe("TableFinder, objects that come with a label (slice 5 review BLOCKER 14)", () => {
  const labelled = [
    { name: "s1/l1", kind: "object", load: "incremental", label: "Team / Tasks" },
    { name: "s1/l2", kind: "object", load: "incremental", label: "Team / Issues" },
    { name: "plain", kind: "object", load: "incremental" },
  ]

  const filler = Array.from({ length: 7 }, (_, i) => ({ name: `f${i}`, kind: "object", load: "incremental", label: `Filler / F${i}` }))

  it("shows the label in words, the name only as a tooltip, and hands the label on with the tick", async () => {
    stub((c) => (c.method === "POST" ? json({ kind: "list", state: "done", objects: labelled }) : undefined))
    const toggled: [string, string | undefined][] = []
    render(<Harness adapter="saas" dial={{ vendor: "sharepoint" }} onToggle={(n, l) => toggled.push([n, l])} />)
    fireEvent.click(screen.getByRole("button", { name: "Find objects" }))
    const row = await screen.findByText("Team / Tasks")
    expect(row.getAttribute("title")).toBe("s1/l1")
    expect(screen.getByText("plain").getAttribute("title")).toBeNull()
    fireEvent.click(screen.getByLabelText(/Team \/ Tasks/))
    fireEvent.click(screen.getByLabelText(/plain/))
    expect(toggled).toEqual([["s1/l1", "Team / Tasks"], ["plain", undefined]])
  })

  it("filters by the label as well as the name, and 'Select shown' hands over the labels", async () => {
    stub((c) => (c.method === "POST" ? json({ kind: "list", state: "done", objects: [...labelled, ...filler] }) : undefined))
    const many: [string[], boolean, Record<string, string> | undefined][] = []
    render(<Harness adapter="saas" dial={{ vendor: "sharepoint" }} onSetMany={(n, on, l) => many.push([n, on, l])} />)
    fireEvent.click(screen.getByRole("button", { name: "Find objects" }))
    await screen.findByText("Team / Tasks")
    fireEvent.change(screen.getByLabelText("Filter objects"), { target: { value: "issues" } })
    expect(screen.queryByText("Team / Tasks")).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Select shown" }))
    expect(many).toEqual([[["s1/l2"], true, { "s1/l2": "Team / Issues" }]])
  })
})

describe("TableFinder, the sources the orchestrator lists (D1)", () => {
  it("shows the wait while the job runs, then the collections", async () => {
    let polled = 0
    const calls = stub((c) => {
      if (c.method === "POST" && c.url === LIST) return json({ kind: "list", state: "running" })
      if (c.method === "GET" && c.url === LIST) {
        polled += 1
        return polled < 2
          ? json({ kind: "list", state: "running" })
          : json({ kind: "list", state: "done", objects: [{ name: "orders", kind: "collection" }] })
      }
      return undefined
    })
    render(<Harness adapter="mongodb" dial={{ database: "shop" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find collections" }))
    expect(await screen.findByText(/Waiting for the orchestrator/)).toBeDefined()
    expect(await screen.findByText("orders")).toBeDefined()
    expect(screen.queryByText(/Waiting for the orchestrator/)).toBeNull()
    expect(calls.filter((c) => c.method === "GET").length).toBeGreaterThanOrEqual(2)
  })

  it("shows a failed job by its class's sentence", async () => {
    stub((c) =>
      c.method === "POST"
        ? json({ kind: "list", state: "running" })
        : json({ kind: "list", state: "failed", errorClass: "timeout", reason: "The source did not answer in time." })
    )
    render(<Harness adapter="kafka" dial={{ topic: "orders" }} single />)
    fireEvent.click(screen.getByRole("button", { name: "Find topics" }))
    expect(await screen.findByText("Cannot list topics · The source did not answer in time.")).toBeDefined()
  })

  it("stops waiting when Cancel is pressed", async () => {
    stub(() => json({ kind: "list", state: "running" }))
    render(<Harness adapter="sql" dial={{ driver: "oracle", user: "hr" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    fireEvent.click(await screen.findByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull())
  })
})

describe("TableFinder, files and the other types", () => {
  it("lists the files under a glob with their size and date", async () => {
    const calls = stub((c) =>
      c.url === LIST
        ? json({
            kind: "list",
            state: "done",
            objects: [{ name: "sales/2026-01.csv", kind: "file", size: 2048, modifiedAt: "2026-01-31T10:00:00Z" }],
          })
        : undefined
    )
    render(<Harness adapter="files" dial={{ bucket: "b", endpoint: "https://s3.example.com" }} />)
    fireEvent.change(screen.getByLabelText("File pattern"), { target: { value: "sales/2026-*.csv" } })
    fireEvent.click(screen.getByRole("button", { name: "Find files" }))
    expect(await screen.findByText("sales/2026-01.csv")).toBeDefined()
    expect(calls[0]?.body).toEqual({ glob: "sales/2026-*.csv" })
    expect(screen.getByText(/2\.0 KB · /)).toBeDefined()
  })

  it("lists a REST connector's declared endpoints and ticks one by its path", async () => {
    stub((c) =>
      c.url === LIST
        ? json({ kind: "list", state: "done", objects: [{ name: "/orders", kind: "endpoint" }, { name: "/users", kind: "endpoint" }] })
        : undefined
    )
    const ticked: string[] = []
    render(<Harness adapter="rest" dial={{}} onToggle={(n) => ticked.push(n)} />)
    fireEvent.click(screen.getByRole("button", { name: "Find endpoints" }))
    fireEvent.click(await screen.findByLabelText("/users"))
    expect(ticked).toEqual(["/users"])
  })

  it("offers 'Select shown' to the multi-select flow and not to Kafka's one topic", async () => {
    stub((c) => (c.url === LIST ? json({ kind: "list", state: "done", objects: [{ name: "a", kind: "topic" }] }) : undefined))
    const many: [string[], boolean][] = []
    const view = render(<Harness adapter="mongodb" dial={{}} onSetMany={(n, on) => many.push([n, on])} />)
    fireEvent.click(screen.getByRole("button", { name: "Find collections" }))
    fireEvent.click(await screen.findByRole("button", { name: "Select shown" }))
    expect(many).toEqual([[["a"], true]])
    view.unmount()
    render(<Harness adapter="kafka" dial={{ topic: "a" }} single />)
    fireEvent.click(screen.getByRole("button", { name: "Find topics" }))
    await screen.findByLabelText("a")
    expect(screen.queryByRole("button", { name: "Select shown" })).toBeNull()
    expect((screen.getByLabelText("a") as HTMLInputElement).type).toBe("radio")
  })
})

describe("TableFinder, the columns of a listed object (D6)", () => {
  it("marks types guessed from a sample, and says what that means", async () => {
    const calls = stub((c) => {
      if (c.url === LIST) return json({ kind: "list", state: "done", objects: [{ name: "orders", kind: "collection" }] })
      if (c.url === PREVIEW)
        return json({
          kind: "preview",
          state: "done",
          object: "orders",
          columns: [{ name: "_id", typeName: "text", inferred: true }, { name: "total", typeName: "double", inferred: true }],
        })
      return undefined
    })
    render(<Harness adapter="mongodb" dial={{}} />)
    fireEvent.click(screen.getByRole("button", { name: "Find collections" }))
    fireEvent.click(await screen.findByRole("button", { name: "Columns" }))
    const panel = await screen.findByRole("group", { name: "Columns of orders" })
    expect(await within(panel).findByText("total")).toBeDefined()
    expect(within(panel).getByText(/Types are inferred from a sample/)).toBeDefined()
    expect(calls.find((c) => c.url === PREVIEW)?.body).toEqual({ object: "orders" })
  })

  it("shows a source's declared types and its key without the inferred note", async () => {
    stub((c) => {
      if (c.url === LIST) return json({ kind: "list", state: "done", objects: [{ name: "public.orders", kind: "table" }] })
      if (c.url === PREVIEW)
        return json({
          kind: "preview",
          state: "done",
          object: "public.orders",
          columns: [{ name: "id", typeName: "integer", inferred: false }],
          primaryKey: ["id"],
        })
      return undefined
    })
    render(<Harness adapter="sql" dial={{ driver: "postgres" }} />)
    fireEvent.click(screen.getByRole("button", { name: "Find tables" }))
    fireEvent.click(await screen.findByRole("button", { name: "Columns" }))
    const panel = await screen.findByRole("group", { name: "Columns of public.orders" })
    expect(await within(panel).findByText("integer")).toBeDefined()
    expect(within(panel).getByText("key")).toBeDefined()
    expect(within(panel).queryByText(/inferred from a sample/)).toBeNull()
  })

  it("says an empty topic has nothing to sample rather than inventing columns", async () => {
    stub((c) => {
      if (c.url === LIST) return json({ kind: "list", state: "done", objects: [{ name: "t", kind: "topic" }] })
      if (c.url === PREVIEW) return json({ kind: "preview", state: "done", object: "t", columns: [] })
      return undefined
    })
    render(<Harness adapter="kafka" dial={{}} single />)
    fireEvent.click(screen.getByRole("button", { name: "Find topics" }))
    fireEvent.click(await screen.findByRole("button", { name: "Columns" }))
    expect(await screen.findByText("Nothing to sample yet, so there are no columns to show.")).toBeDefined()
  })

  it("says a preview failed in the API's words and offers another try", async () => {
    let answered = 0
    stub((c) => {
      if (c.url === LIST) return json({ kind: "list", state: "done", objects: [{ name: "orders", kind: "collection" }] })
      if (c.url === PREVIEW) {
        answered += 1
        return answered === 1
          ? json({ kind: "preview", state: "failed", object: "orders", errorClass: "no_permission", reason: "The credential may not read this." })
          : json({ kind: "preview", state: "done", object: "orders", columns: [{ name: "a", typeName: "text", inferred: true }] })
      }
      return undefined
    })
    render(<Harness adapter="mongodb" dial={{}} />)
    fireEvent.click(screen.getByRole("button", { name: "Find collections" }))
    fireEvent.click(await screen.findByRole("button", { name: "Columns" }))
    expect(await screen.findByText("Cannot read the columns · The credential may not read this.")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Try again" }))
    expect(await screen.findByText("a")).toBeDefined()
  })

  it("reads previews one at a time, so one never replaces another's answer on the orchestrator", async () => {
    let inFlight = 0
    let overlap = false
    global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
      const url = String(input)
      if (url === LIST) return json({ kind: "list", state: "done", objects: [{ name: "a", kind: "collection" }, { name: "b", kind: "collection" }] })
      const body = JSON.parse(init!.body as string) as { object: string }
      inFlight += 1
      if (inFlight > 1) overlap = true
      await new Promise((r) => setTimeout(r, 20))
      inFlight -= 1
      return json({ kind: "preview", state: "done", object: body.object, columns: [] })
    }) as unknown as typeof fetch
    render(<Harness adapter="mongodb" dial={{}} />)
    fireEvent.click(screen.getByRole("button", { name: "Find collections" }))
    const buttons = await screen.findAllByRole("button", { name: "Columns" })
    fireEvent.click(buttons[0]!)
    fireEvent.click(buttons[1]!)
    await waitFor(() => expect(screen.getByRole("group", { name: "Columns of b" })).toBeDefined())
    await screen.findByText("Nothing to sample yet, so there are no columns to show.")
    expect(overlap).toBe(false)
  })
})
