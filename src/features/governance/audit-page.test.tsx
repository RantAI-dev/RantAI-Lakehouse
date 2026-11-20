// The audit drawer's "Detail" row (QA LH-DATA-15-03 F1, plan D9): a catalog
// grant's entry says who got which permission; an entry without a detail
// shows no such row.
// `search` is the address's query, so a test can open `/audit?outcome=refused`
// or the `?event=<id>` link another page builds.
let search = new URLSearchParams()
mock.module("next/navigation", () => ({
  usePathname: () => "/audit",
  useSearchParams: () => search,
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { TableProviders } from "@/components/app-shell/table-providers"
import type { AuditEvent } from "@/services/contracts/governance"
import { AuditPage, EventDetail } from "./audit-page"

beforeEach(() => {
  search = new URLSearchParams()
  // The page writes its filters into the address, which needs a real origin
  // (happy-dom starts on about:blank); start every test from a bare one.
  ;(window as unknown as { happyDOM: { setURL: (url: string) => void } }).happyDOM.setURL(
    "http://localhost/audit",
  )
})

afterEach(() => {
  // The tests' `useSearchParams` never follows the address the page writes, so
  // nuqs keeps that write queued as unconfirmed. A popstate makes it drop the
  // queue, as a real navigation does, before the next test mounts the page.
  window.dispatchEvent(new PopStateEvent("popstate"))
  cleanup()
  mock.restore()
})

const event = (over: Partial<AuditEvent> = {}): AuditEvent => ({
  id: "audit-1",
  at: "2026-11-06T08:00:00.000Z",
  actor: "Platform Admin",
  actorId: "usr_admin",
  actorKind: "user",
  action: "catalog.grant",
  resource: "catalog:silver.orders",
  outcome: "success",
  outcomeDetail: "executed",
  policyDecision: "allow",
  obligations: [],
  reason: null,
  ...over,
})

describe("EventDetail", () => {
  /** The value cell of a metadata row, read through its label. */
  function rowValueOf(label: string): string | null {
    return screen.getByText(label).closest("div")?.querySelector("dd")?.textContent ?? null
  }

  it("shows the detail sentence of a grant under a Detail label", () => {
    render(
      <EventDetail
        event={event({ detail: "Granted query:read to QA Approver <qa-approver@example.test> until 2026-12-06" })}
      />,
    )
    expect(screen.getByText("Detail")).toBeDefined()
    expect(
      screen.getByText("Granted query:read to QA Approver <qa-approver@example.test> until 2026-12-06"),
    ).toBeDefined()
  })

  it("shows no Detail row for an entry that has no detail", () => {
    render(<EventDetail event={event({ action: "catalog.annotate" })} />)
    expect(screen.queryByText("Detail")).toBeNull()
  })

  // plan D2, D4, D4b: the detail names the principal behind the actor, the
  // outcome the row stored (beside the four-word one the badge shows), and
  // the stored detail column, when it holds one.
  it("shows the actor id, the stored outcome and the stored reason of an event that carries them", () => {
    render(
      <EventDetail
        event={event({
          actorId: "usr_admin",
          outcomeDetail: "failed",
          reason: "Central returned 403 while the tool ran.",
        })}
      />,
    )
    expect(screen.getByText("Actor id")).toBeDefined()
    expect(rowValueOf("Actor id")).toBe("usr_admin")
    expect(screen.getByText("Stored outcome")).toBeDefined()
    expect(rowValueOf("Stored outcome")).toBe("failed")
    expect(screen.getByText("Reason")).toBeDefined()
    expect(rowValueOf("Reason")).toBe("Central returned 403 while the tool ran.")
  })

  it("shows a dash for the actor id of an event nobody signed in caused", () => {
    render(<EventDetail event={event({ actorId: null })} />)
    expect(rowValueOf("Actor id")).toBe("—")
  })

  it("shows no Reason row for an event that stored no reason", () => {
    render(<EventDetail event={event({ reason: null })} />)
    expect(screen.queryByText("Reason")).toBeNull()
  })
})

// GOV-1 Decision 9: the API answers 403 on the audit list for a person without
// `audit:read`; the page shows the refusal state and no event rows.
describe("AuditPage", () => {
  it("shows the permission refusal, and no events, when the list answers 403", async () => {
    const answer = {
      error: "permission_denied: reading the audit trail needs the audit:read permission",
    }
    spyOn(globalThis, "fetch").mockImplementation((async () =>
      new Response(JSON.stringify(answer), {
        status: 403,
        headers: { "Content-Type": "application/json" },
      })) as unknown as typeof fetch)

    // `TableProviders` supplies the `NuqsAdapter` the data table needs.
    render(
      <TableProviders>
        <AuditPage />
      </TableProviders>,
    )

    await waitFor(() => expect(screen.getByText("You don't have access")).toBeDefined())
    expect(document.body.textContent ?? "").toContain("audit:read")
    expect(document.body.textContent ?? "").not.toMatch(/permission_denied/)
    expect(screen.queryByRole("table")).toBeNull()
    expect(screen.queryByText("catalog.grant")).toBeNull()
  })
})

// GOV-1 part B, Decisions 5 and 9: the page asks the server for the events,
// narrows them there, pages with a cursor, opens one event by id and exports
// the same search as a CSV file.
describe("AuditPage: search, paging, deep link and export", () => {
  const SEARCH = "/api/governance/audit/events"
  const ACTORS = "/api/governance/audit/actors"
  const EXPORT = "/api/governance/audit/export.csv"

  /** The row the API sends: the same object as one row of the old list. */
  const row = (id: string, action: string) => event({ id, action, resource: `dashboard:${id}` })

  /** The principals the actors route answers, when a test needs named people. */
  const actorsPage = {
    actors: [
      { actorId: "usr_admin", actor: "Platform Admin", actorKind: "user" as const },
      { actorId: "agt_qa", actor: "QA Runner", actorKind: "agent" as const },
    ],
  }

  type Sent = { path: string; params: URLSearchParams }
  let sent: Sent[] = []
  let saved: Blob[] = []

  /**
   * Stubs `fetch`: `answer` gets each request's path and query and returns
   * the response; the actors route is answered with `actors` (an empty list
   * unless a test passes one, or a Response to make the route fail).
   */
  function stubApi(answer: (path: string, params: URLSearchParams) => Response, actors?: Response) {
    sent = []
    spyOn(globalThis, "fetch").mockImplementation((async (input: RequestInfo | URL) => {
      const url = new URL(String(input), "http://localhost")
      sent.push({ path: url.pathname, params: url.searchParams })
      if (url.pathname === ACTORS) return actors ?? json({ actors: [] })
      return answer(url.pathname, url.searchParams)
    }) as unknown as typeof fetch)
  }

  const json = (body: unknown, status = 200) =>
    new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })

  const searches = () => sent.filter((c) => c.path === SEARCH)

  function renderPage() {
    // `TableProviders` supplies the `NuqsAdapter` the page's URL state needs.
    return render(
      <TableProviders>
        <AuditPage />
      </TableProviders>,
    )
  }

  beforeEach(() => {
    saved = []
    // The browser hand-off of a file: record the blob instead of opening a link.
    URL.createObjectURL = ((blob: Blob) => {
      saved.push(blob)
      return "blob:test"
    }) as typeof URL.createObjectURL
    URL.revokeObjectURL = () => {}
  })

  it("asks the server for the newest events and shows them, with no filter", async () => {
    stubApi(() => json({ events: [row("a1", "dashboard.share")], nextCursor: null }))
    renderPage()
    await waitFor(() => expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0))
    expect(searches()).toHaveLength(1)
    expect([...searches()[0].params.keys()]).toEqual([])
  })

  it("sends every filter the address holds, and the date range as UTC instants with the last day included", async () => {
    search = new URLSearchParams(
      "actor=u-1&action=dashboard.&resourceKind=dashboard&resourceId=d1&outcome=refused&from=2026-11-01&to=2026-11-10",
    )
    stubApi(() => json({ events: [], nextCursor: null }))
    renderPage()
    await waitFor(() => expect(searches().length).toBeGreaterThan(0))
    expect(Object.fromEntries(searches()[0].params)).toEqual({
      actor: "u-1",
      action: "dashboard.",
      resourceKind: "dashboard",
      resourceId: "d1",
      outcome: "refused",
      from: "2026-11-01T00:00:00Z",
      // `to` is exclusive on the server: the day after, so the typed day is in.
      to: "2026-11-11T00:00:00Z",
    })
  })

  // plan D2: with the actors route unreadable the person field falls back to
  // the raw id, and the search still sends what was typed.
  it("Search sends the typed person id when the actors list cannot be read, and does not filter the loaded rows in the browser", async () => {
    stubApi(
      () => json({ events: [row("a1", "dashboard.share")], nextCursor: null }),
      json({ error: "The audit trail could not be read." }, 500),
    )
    renderPage()
    await waitFor(() => expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0))
    expect(screen.getByText(/Could not read the trail's people/)).toBeDefined()

    fireEvent.change(screen.getByLabelText("Person"), { target: { value: "usr-7" } })
    fireEvent.change(screen.getByLabelText("Action"), { target: { value: "catalog." } })
    chooseOption(screen.getByRole("combobox", { name: "Outcome" }), "Failed")
    fireEvent.click(screen.getByRole("button", { name: "Search" }))

    await waitFor(() => expect(searches().length).toBe(2))
    expect(Object.fromEntries(searches()[1].params)).toEqual({
      actor: "usr-7",
      action: "catalog.",
      outcome: "failed",
    })
    // The filters are kept in the address, so a link reproduces the search.
    await waitFor(() =>
      expect(Object.fromEntries(new URLSearchParams(window.location.search))).toEqual({
        actor: "usr-7",
        action: "catalog.",
        outcome: "failed",
      }),
    )
  })

  // plan D2: the person filter is a picker over the trail's principals.
  it("lists the trail's actors in the person picker, and picking one sends actor=<id> on the search", async () => {
    stubApi(
      () => json({ events: [row("a1", "dashboard.share")], nextCursor: null }),
      json(actorsPage),
    )
    renderPage()
    await waitFor(() => expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0))

    fireEvent.click(screen.getByRole("button", { name: "Person" }))
    const options = await screen.findAllByRole("option")
    expect(options.map((option) => option.textContent)).toEqual([
      "Platform Admin · User",
      "QA Runner · Agent",
    ])
    fireEvent.click(options[1] as HTMLElement)

    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Person" }).textContent).toContain("QA Runner"),
    )
    fireEvent.click(screen.getByRole("button", { name: "Search" }))
    await waitFor(() => expect(searches().length).toBe(2))
    expect(searches()[1].params.get("actor")).toBe("agt_qa")
    await waitFor(() =>
      expect(Object.fromEntries(new URLSearchParams(window.location.search)).actor).toBe("agt_qa"),
    )
  })

  // plan D2: clicking the actor in a row narrows the trail to that principal,
  // without waiting for a Search click.
  it("sets the person filter when the actor in a row is clicked, and the next request carries that actor", async () => {
    stubApi(
      () => json({ events: [row("a1", "dashboard.share")], nextCursor: null }),
      json(actorsPage),
    )
    renderPage()
    await waitFor(() => expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0), {
      timeout: 5000,
    })

    fireEvent.click(screen.getByRole("button", { name: /Platform Admin/ }))

    await waitFor(() => expect(searches().length).toBe(2), { timeout: 5000 })
    expect(searches()[1].params.get("actor")).toBe("usr_admin")
    // The picker shows the picked person by name, so the filter is legible.
    await waitFor(
      () =>
        expect(screen.getByRole("button", { name: "Person" }).textContent).toContain("Platform Admin"),
      { timeout: 5000 },
    )
  })

  // plan D2: the same from the detail drawer, whose Actor row is a button too.
  it("sets the person filter when the actor in the open detail is clicked", async () => {
    stubApi(
      () => json({ events: [row("a1", "dashboard.share")], nextCursor: null }),
      json(actorsPage),
    )
    renderPage()
    await waitFor(() => expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0))

    fireEvent.click(screen.getByRole("button", { name: "dashboard.share" }))
    expect(await screen.findByText("Audit event a1")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Platform Admin (User)" }))

    await waitFor(() => expect(searches().length).toBe(2))
    expect(searches()[1].params.get("actor")).toBe("usr_admin")
  })

  // plan D2: a row nobody signed in caused has no principal to filter by, so
  // its actor is plain text and not a button.
  it("leaves the actor of an event with no actor id unclickable", async () => {
    stubApi(() =>
      json(
        {
          events: [{ ...row("a1", "auth.login"), actor: "Sign-in form", actorId: null }],
          nextCursor: null,
        },
      ),
    )
    renderPage()
    await waitFor(() => expect(screen.getAllByText("auth.login").length).toBeGreaterThan(0))

    expect(screen.getAllByText("Sign-in form").length).toBeGreaterThan(0)
    expect(screen.queryByRole("button", { name: /Sign-in form/ })).toBeNull()
  })

  it("Load more sends the cursor of the last page and appends the next one", async () => {
    stubApi((_path, params) =>
      params.get("cursor") === "next-1"
        ? json({ events: [row("a2", "catalog.grant")], nextCursor: null })
        : json({ events: [row("a1", "dashboard.share")], nextCursor: "next-1" }),
    )
    renderPage()
    await waitFor(() => expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0))
    expect(screen.queryByText("catalog.grant")).toBeNull()

    fireEvent.click(screen.getByRole("button", { name: "Load more" }))

    await waitFor(() => expect(screen.getAllByText("catalog.grant").length).toBeGreaterThan(0))
    expect(searches()[1].params.get("cursor")).toBe("next-1")
    expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0)
    // The last page has no cursor: there is nothing more to load.
    expect(screen.queryByRole("button", { name: "Load more" })).toBeNull()
  })

  it("Export CSV asks for the file with the filters in the address and saves it", async () => {
    search = new URLSearchParams("actor=u-1&outcome=failed&from=2026-11-01")
    stubApi((path) =>
      path === EXPORT
        ? new Response("at,actor\n", {
            status: 200,
            headers: { "content-disposition": 'attachment; filename="audit-events-2026-11-15.csv"' },
          })
        : json({ events: [], nextCursor: null }),
    )
    renderPage()
    await waitFor(() => expect(searches().length).toBe(1))

    fireEvent.click(screen.getByRole("button", { name: "Export CSV" }))

    await waitFor(() => expect(saved).toHaveLength(1))
    const exportCall = sent.find((c) => c.path === EXPORT)
    expect(Object.fromEntries(exportCall?.params ?? [])).toEqual({
      actor: "u-1",
      outcome: "failed",
      from: "2026-11-01T00:00:00Z",
    })
    expect(screen.queryByText(/Narrow the time range/)).toBeNull()
  })

  it("says the export was cut, and how many events matched", async () => {
    stubApi((path) =>
      path === EXPORT
        ? new Response("at\n", {
            status: 200,
            headers: { "x-audit-export-truncated": "true", "x-audit-export-total": "250000" },
          })
        : json({ events: [], nextCursor: null }),
    )
    renderPage()
    await waitFor(() => expect(searches().length).toBe(1))

    fireEvent.click(screen.getByRole("button", { name: "Export CSV" }))

    expect(
      await screen.findByText(
        "The export holds the newest 100,000 of 250,000 matching events. Narrow the time range to get the rest.",
      ),
    ).toBeDefined()
  })

  it("opens an event from a link when it is not on the loaded page, by asking for that event", async () => {
    search = new URLSearchParams("event=audit-9")
    stubApi((path) =>
      path === `${SEARCH}/audit-9`
        ? json({ event: row("audit-9", "pipeline.trigger") })
        : json({ events: [row("a1", "dashboard.share")], nextCursor: null }),
    )
    renderPage()

    expect(await screen.findByText("Audit event audit-9")).toBeDefined()
    expect(sent.some((c) => c.path === `${SEARCH}/audit-9`)).toBe(true)
  })

  it("opens an event from a link without another request when the page already holds it", async () => {
    search = new URLSearchParams("event=a1")
    stubApi(() => json({ events: [row("a1", "dashboard.share")], nextCursor: null }))
    renderPage()

    expect(await screen.findByText("Audit event a1")).toBeDefined()
    // The list and the actors list load; the one event is not fetched again.
    expect(sent.every((c) => c.path === SEARCH || c.path === ACTORS)).toBe(true)
  })

  it("shows the list and a plain sentence, not a broken page, when the linked event is gone", async () => {
    search = new URLSearchParams("event=gone")
    stubApi((path) =>
      path === `${SEARCH}/gone`
        ? json({ error: "No audit event has that id." }, 404)
        : json({ events: [row("a1", "dashboard.share")], nextCursor: null }),
    )
    renderPage()

    expect(await screen.findByText("No audit event has the id gone.")).toBeDefined()
    expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0)
    expect(screen.queryByText("Something went wrong")).toBeNull()
  })

  // GOV-1 review T-2: only a 404 means "no such event". A 500 says nothing
  // about the id, so the page shows the server's message and keeps the list.
  it("shows the error, not the no-such-event sentence, when the linked event cannot be read", async () => {
    search = new URLSearchParams("event=audit-9")
    stubApi((path) =>
      path === `${SEARCH}/audit-9`
        ? json({ error: "The audit trail could not be read." }, 500)
        : json({ events: [row("a1", "dashboard.share")], nextCursor: null }),
    )
    renderPage()

    expect(await screen.findByText("The audit trail could not be read.")).toBeDefined()
    expect(document.body.textContent ?? "").not.toContain("No audit event has the id")
    expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0)
  })

  // GOV-1 review T-3: the server sends no total, so the footer must not
  // print one; "Showing 1 of 1" next to a Load more button is a made-up count.
  it("names no total in the footer while more events can be loaded, and says so at the end", async () => {
    stubApi((_path, params) =>
      params.get("cursor") === "next-1"
        ? json({ events: [row("a2", "catalog.grant")], nextCursor: null })
        : json({ events: [row("a1", "dashboard.share")], nextCursor: "next-1" }),
    )
    renderPage()
    await waitFor(() => expect(screen.getAllByText("dashboard.share").length).toBeGreaterThan(0))
    expect(document.body.textContent ?? "").not.toMatch(/\d+ of \d+/)
    expect(screen.getByText("Showing 1 so far")).toBeDefined()

    fireEvent.click(screen.getByRole("button", { name: "Load more" }))

    await waitFor(() => expect(screen.getAllByText("catalog.grant").length).toBeGreaterThan(0))
    expect(document.body.textContent ?? "").not.toMatch(/\d+ of \d+/)
    expect(screen.getByText("Showing 2 · End of list")).toBeDefined()
  })
})
