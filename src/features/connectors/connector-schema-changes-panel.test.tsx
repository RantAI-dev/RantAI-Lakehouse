// The "Schema changes" tab of a connector (`SRC-8`, task 10): what waits, the
// buttons the API's `canApprove` allows, the notice for a table that was not
// added, the policy and its 403.
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { SchemaChange, SchemaChangeList } from "@/services/contracts/connectors"
import {
  APPLY_ALL_NOTE,
  CANNOT_BE_LOADED_NOTE,
  ConnectorSchemaChangesPanel,
  changeLabel,
  waitingTables,
} from "./connector-schema-changes-panel"

const originalFetch = global.fetch

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

function change(over: Partial<SchemaChange>): SchemaChange {
  return {
    id: "chg-1",
    objectName: "public.orders",
    kind: "column_removed",
    columnName: "note",
    beforeValue: "text",
    afterValue: null,
    breaking: true,
    status: "pending",
    runId: "run-1",
    detectedAt: "2026-10-09T01:00:00.000Z",
    decidedBy: null,
    decidedAt: null,
    canApprove: true,
    ...over,
  }
}

const EMPTY: SchemaChangeList = { pending: [], recent: [], inactiveColumns: [] }

type Call = { key: string; body: unknown }

/** `GET .../schema-changes` answers `lists` in turn (the last one repeats); other routes are `overrides`. */
function stubFetch(lists: SchemaChangeList[], overrides: Record<string, () => Response> = {}) {
  const calls: Call[] = []
  let reads = 0
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const target = String(input).replace(/^https?:\/\/[^/]+/, "")
    const key = `${init?.method ?? "GET"} ${target}`
    calls.push({ key, body: init?.body ? JSON.parse(String(init.body)) : undefined })
    if (overrides[key]) return overrides[key]()
    if (key === "GET /api/connectors/conn-a/schema-changes") {
      return json(lists[Math.min(reads++, lists.length - 1)])
    }
    throw new Error(`unexpected fetch: ${key}`)
  }) as unknown as typeof fetch
  return calls
}

const settle = () =>
  act(async () => {
    await new Promise((resolve) => setTimeout(resolve, 0))
  })

function renderPanel(props: Partial<React.ComponentProps<typeof ConnectorSchemaChangesPanel>> = {}) {
  const onChanged = mock(() => {})
  const onWaiting = mock<(tables: Set<string>, groups: number) => void>(() => {})
  render(
    <ConnectorSchemaChangesPanel
      connectorId="conn-a"
      connectorType="PostgreSQL"
      policy="apply_non_breaking"
      onChanged={onChanged}
      onWaiting={onWaiting}
      {...props}
    />
  )
  return { onChanged, onWaiting }
}

describe("ConnectorSchemaChangesPanel", () => {
  it("says nothing waits and nothing was recorded for a connector with no changes", async () => {
    stubFetch([EMPTY])
    const { onWaiting } = renderPanel()
    expect(await screen.findByText("Nothing is waiting for a decision.")).toBeTruthy()
    expect(screen.getByText("No schema change has been recorded yet.")).toBeTruthy()
    await waitFor(() => expect(onWaiting).toHaveBeenCalled())
    expect(onWaiting.mock.calls[0][1]).toBe(0)
  })

  it("offers one Approve and resume per table that waits, and reports which tables wait", async () => {
    const list: SchemaChangeList = {
      pending: [change({}), change({ id: "chg-2", kind: "column_added", columnName: "qty", beforeValue: null, afterValue: "integer", breaking: false })],
      recent: [],
      inactiveColumns: [],
    }
    stubFetch([list])
    const { onWaiting } = renderPanel()
    expect(await screen.findByText("Column removed")).toBeTruthy()
    expect(screen.getByText("Column added")).toBeTruthy()
    expect(screen.getAllByRole("button", { name: "Approve and resume" })).toHaveLength(1)
    await waitFor(() => expect(onWaiting).toHaveBeenCalled())
    expect([...onWaiting.mock.calls[0][0]]).toEqual(["public.orders"])
    expect(onWaiting.mock.calls[0][1]).toBe(1)
  })

  it("hides the button and explains why when the API says a type change cannot be approved", async () => {
    const list: SchemaChangeList = {
      pending: [change({ kind: "type_changed", columnName: "qty", beforeValue: "integer", afterValue: "text", canApprove: false })],
      recent: [],
      inactiveColumns: [],
    }
    stubFetch([list])
    renderPanel()
    expect(await screen.findByText("Type changed from integer to text")).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Approve and resume" })).toBeNull()
    expect(screen.getByText(CANNOT_BE_LOADED_NOTE)).toBeTruthy()
    const group = screen.getByText("public.orders").closest("li") as HTMLElement
    expect(within(group).getByText("Waiting for a decision")).toBeTruthy()
  })

  it("shows a table that was not added as a notice with Dismiss, and does not count it as waiting", async () => {
    const reason = "Not added: its Bronze table name is already used by another table. Add it by hand with a different target."
    const list: SchemaChangeList = {
      pending: [change({ kind: "table_added", objectName: "public.files", columnName: "", beforeValue: null, afterValue: reason, breaking: false })],
      recent: [],
      inactiveColumns: [],
    }
    const calls = stubFetch([list, EMPTY], {
      "POST /api/connectors/conn-a/schema-changes/approve": () =>
        json({ object: "public.files", approved: [], pauseLifted: false }),
    })
    const { onWaiting, onChanged } = renderPanel()
    expect(await screen.findByText(reason)).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Approve and resume" })).toBeNull()
    await waitFor(() => expect(onWaiting).toHaveBeenCalled())
    expect(onWaiting.mock.calls[0][0].size).toBe(0)

    fireEvent.click(screen.getByRole("button", { name: "Dismiss" }))

    await waitFor(() => expect(onChanged).toHaveBeenCalled())
    expect(calls.find((c) => c.key.startsWith("POST"))?.body).toEqual({ object: "public.files" })
    expect(await screen.findByText("Nothing is waiting for a decision.")).toBeTruthy()
  })

  it("posts the approval, then reads the list again and tells the page", async () => {
    const pending: SchemaChangeList = { pending: [change({})], recent: [], inactiveColumns: [] }
    const after: SchemaChangeList = {
      pending: [],
      recent: [change({ status: "approved", canApprove: false })],
      inactiveColumns: [{ objectName: "public.orders", columnName: "note", inactiveSince: "2026-10-09T02:00:00.000Z" }],
    }
    const calls = stubFetch([pending, after], {
      "POST /api/connectors/conn-a/schema-changes/approve": () =>
        json({ object: "public.orders", approved: [change({ status: "approved" })], pauseLifted: true }),
    })
    const { onChanged } = renderPanel()

    fireEvent.click(await screen.findByRole("button", { name: "Approve and resume" }))

    await waitFor(() => expect(onChanged).toHaveBeenCalled())
    expect(calls.find((c) => c.key.startsWith("POST"))?.body).toEqual({ object: "public.orders" })
    expect(await screen.findByText("Nothing is waiting for a decision.")).toBeTruthy()
    expect(screen.getByText(/inactive since/)).toBeTruthy()
  })

  it("shows the API's own message when approving is refused with a 409", async () => {
    stubFetch([{ pending: [change({})], recent: [], inactiveColumns: [] }], {
      "POST /api/connectors/conn-a/schema-changes/approve": () => json({ error: CANNOT_BE_LOADED_NOTE }, 409),
    })
    const { onChanged } = renderPanel()

    fireEvent.click(await screen.findByRole("button", { name: "Approve and resume" }))

    expect((await screen.findByRole("alert")).textContent).toBe(CANNOT_BE_LOADED_NOTE)
    expect(onChanged).not.toHaveBeenCalled()
  })

  it("lists recent changes in plain words with their before and after", async () => {
    const list: SchemaChangeList = {
      pending: [],
      recent: [
        change({ id: "a", status: "applied", kind: "type_changed", columnName: "qty", beforeValue: "integer", afterValue: "bigint", breaking: false }),
        change({ id: "b", status: "applied", kind: "primary_key_changed", columnName: "", beforeValue: "id", afterValue: "id, tenant" }),
        change({ id: "c", status: "applied", kind: "table_added", objectName: "public.invoices", columnName: "", beforeValue: null, afterValue: "shop_invoices", breaking: false }),
      ],
      inactiveColumns: [],
    }
    stubFetch([list])
    renderPanel()
    expect(await screen.findByText("Type changed from integer to bigint")).toBeTruthy()
    expect(screen.getByText("Primary key changed")).toBeTruthy()
    expect(screen.getByText("id → id, tenant")).toBeTruthy()
    expect(screen.getByText("Table added")).toBeTruthy()
    expect(screen.getByText("Loads into bronze.shop_invoices")).toBeTruthy()
  })

  it("saves the policy through the connector update and tells the page", async () => {
    const calls = stubFetch([EMPTY], {
      "PATCH /api/connectors/conn-a": () => json({ id: "conn-a", schemaChangePolicy: "ask_first" }),
    })
    const { onChanged } = renderPanel()
    await settle()
    const save = screen.getByRole("button", { name: "Save policy" }) as HTMLButtonElement
    expect(save.disabled).toBe(true)

    fireEvent.click(screen.getByRole("radio", { name: "Ask first" }))
    fireEvent.click(save)

    await waitFor(() => expect(onChanged).toHaveBeenCalled())
    expect(calls.find((c) => c.key.startsWith("PATCH"))?.body).toEqual({ schemaChangePolicy: "ask_first" })
  })

  it("shows the API's message when saving the policy is refused with a 403", async () => {
    stubFetch([EMPTY], {
      "PATCH /api/connectors/conn-a": () => json({ error: "connector:manage is required" }, 403),
    })
    renderPanel()
    await settle()
    fireEvent.click(screen.getByRole("radio", { name: "Pause" }))
    fireEvent.click(screen.getByRole("button", { name: "Save policy" }))
    expect((await screen.findByRole("alert")).textContent).toBe("connector:manage is required")
  })

  it("puts the Apply all note only on a source type that cannot list its tables", async () => {
    stubFetch([EMPTY])
    const { unmount } = render(
      <ConnectorSchemaChangesPanel connectorId="conn-a" connectorType="Oracle" policy="apply_all" onChanged={() => {}} />
    )
    await settle()
    expect(screen.getByText(APPLY_ALL_NOTE)).toBeTruthy()
    unmount()
    cleanup()
    render(
      <ConnectorSchemaChangesPanel connectorId="conn-a" connectorType="SQL Server" policy="apply_all" onChanged={() => {}} />
    )
    await settle()
    expect(screen.queryByText(APPLY_ALL_NOTE)).toBeNull()
    expect(within(screen.getByRole("group")).getAllByRole("radio")).toHaveLength(4)
  })
})

describe("helpers", () => {
  it("names a type change with both types and every other kind in plain words", () => {
    expect(changeLabel(change({ kind: "type_changed", beforeValue: "int", afterValue: "text" }))).toBe(
      "Type changed from int to text"
    )
    expect(changeLabel(change({ kind: "column_added" }))).toBe("Column added")
  })

  it("counts a table once however many of its changes wait, and not a notice", () => {
    const list: SchemaChangeList = {
      pending: [change({}), change({ id: "x" }), change({ id: "n", kind: "table_added", objectName: "public.n" })],
      recent: [],
      inactiveColumns: [],
    }
    expect([...waitingTables(list)]).toEqual(["public.orders"])
  })
})
