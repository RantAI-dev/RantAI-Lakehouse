// The pipeline editor (BLD-1 part 1 D17), rendered: create and edit.
//
// Ported from the five-step wizard's tests: the re-upload switch and the
// source-table suggestions (DATA-19 D3 and D4), the zone and column pickers
// (WEB-8 D8), the connector-page entry (BLD-7 decisions 4 and 5), changing
// the connector (BLD-9) and the plain copy and FBIC (BLD-10). The pure rules
// behind them are pinned in `pipeline-reupload-trigger.test.ts`,
// `pipeline-connector-trigger.test.ts` and `lib/pipeline-editor.test.ts`;
// what is proved HERE is the wiring.
//
// Everything below the fetch is real: the real service clients, the real
// `apiFetch`, the real auth provider, the real component. Only `fetch` is
// stubbed.
const pushed: string[] = []
/** The query string `useSearchParams` answers with; each test sets it. */
let search = ""
mock.module("next/navigation", () => ({
  usePathname: () => "/pipelines/create",
  useSearchParams: () => new URLSearchParams(search),
  useRouter: () => ({
    push: (to: string) => {
      pushed.push(to)
    },
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import { currentCompletions, startCompletion } from "@codemirror/autocomplete"
import { EditorView } from "@codemirror/view"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { AuthProvider } from "@/features/auth/auth-provider"
import type { PipelineDetail } from "@/services/contracts/pipelines"
import { PipelineEditor } from "./pipeline-editor"

const originalFetch = global.fetch
/** Every body the form sent, by method and path. */
const sent: { method: string; path: string; body: Record<string, unknown> }[] = []

/** The connector a QA report made its pipeline from, and its ingest targets. */
const SEED_CONNECTOR = "conn-pg-lakehouse"
let ingestTargets: { name: string; target: string }[] = []
let connectorList: { id: string; name: string }[] = []
/** The person signed in (`GET /api/auth/me`). */
const ME = "u-me"
/** What a save answers; a test sets a refusal to see it on the item it names. */
let saveAnswer: { status: number; body: unknown } = { status: 200, body: { id: "pl-1", name: "orders_clean" } }
/** What a preview answers. */
let previewAnswer: { status: number; body: unknown } = {
  status: 200,
  body: { columns: [{ name: "id", type: "Int64" }], rows: [[1], [2]], truncated: false },
}

function jsonResponse(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), {
    status,
    headers: { "Content-Type": "application/json" },
  })
}

/** One Bronze table, the way `/api/lakehouse/tables` answers for it. */
function table(name: string) {
  return {
    namespace: "bronze",
    name,
    formatVersion: { value: 2, measured: true },
    currentSnapshotId: null,
    lastUpdatedAt: null,
    fileCount: { value: 1, measured: true },
    recordCount: { value: 4, measured: true },
    totalBytes: { value: 1024, measured: true },
  }
}

beforeEach(() => {
  sent.length = 0
  pushed.length = 0
  search = ""
  saveAnswer = { status: 200, body: { id: "pl-1", name: "orders_clean" } }
  previewAnswer = {
    status: 200,
    body: { columns: [{ name: "id", type: "Int64" }], rows: [[1], [2]], truncated: false },
  }
  ingestTargets = [
    { name: "public.orders", target: "orders" },
    { name: "public.customers", target: "customers" },
  ]
  connectorList = [{ id: SEED_CONNECTOR, name: "Lakehouse OLTP (Postgres)" }]
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    let body: Record<string, unknown> = {}
    if (typeof init?.body === "string") body = JSON.parse(init.body)
    sent.push({ method, path: url.split("?")[0] ?? url, body })
    if (url.startsWith("/api/auth/me")) {
      return jsonResponse({ id: ME, name: "Me", email: null, roles: [], permissions: ["pipeline:write"], tenants: [] })
    }
    if (url.includes("/ingest-spec")) {
      // BLD-7 decision 5: the connector the page already reads through
      // `connectorService` answers with its ingest targets.
      return jsonResponse({
        adapter: "sql",
        ingestMode: "batch",
        dial: {},
        sourceObjects: ingestTargets,
        scheduleCron: null,
        secretRefs: { primary: "env:POSTGRES_PASSWORD", secondary: null },
        nextRunAt: null,
      })
    }
    if (url.startsWith("/api/connectors")) {
      return jsonResponse(connectorList)
    }
    if (url.startsWith("/api/lakehouse/namespaces")) {
      return jsonResponse({
        namespaces: [
          { name: "bronze", tableCount: { value: 2, measured: true } },
          { name: "silver", tableCount: { value: 1, measured: true } },
        ],
      })
    }
    // WEB-8 D8: the source table's own schema, for the column pickers.
    // Only the tables the listing above names have one; anything else is a
    // table this deployment does not have (404), which is the case the
    // free-text fallback covers.
    if (url.startsWith("/api/lakehouse/tables/")) {
      const name = url.split("/api/lakehouse/tables/")[1] ?? ""
      if (!["bronze/orders_raw", "bronze/stock_raw"].includes(name)) {
        return jsonResponse({ error: "table not found" }, 404)
      }
      return jsonResponse({
        schema: [
          { id: 1, name: "id", type: "long", required: true },
          { id: 2, name: "updated_at", type: "timestamp", required: false },
          { id: 3, name: "status", type: "string", required: false },
        ],
        partitionSpec: [],
        properties: {},
        snapshots: [],
        stats: {},
      })
    }
    if (url.startsWith("/api/lakehouse/tables")) {
      return jsonResponse({ tables: [table("stock_raw"), table("orders_raw")] })
    }
    if (url.startsWith("/api/pipelines/preview")) {
      return jsonResponse(previewAnswer.body, previewAnswer.status)
    }
    if (url.startsWith("/api/pipelines") && (method === "POST" || method === "PUT")) {
      return jsonResponse(saveAnswer.body, saveAnswer.status)
    }
    return jsonResponse({})
  }) as unknown as typeof fetch
})

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

/** The editor inside the real auth provider, which reads `GET /api/auth/me`. */
function renderEditor(existingPipeline?: PipelineDetail) {
  return render(
    <AuthProvider>
      <PipelineEditor existing={existingPipeline} />
    </AuthProvider>
  )
}

/** A stored authored pipeline made before BLD-1 part 1, as the API returns it now (D3). */
function existing(over: Partial<PipelineDetail> = {}): PipelineDetail {
  return {
    id: "pl-orders-1",
    name: "orders_clean",
    kind: "batch",
    status: "ready",
    owner: "system",
    source: "bronze.orders_raw",
    target: "silver.orders_clean",
    schedule: "manual",
    lastRunAt: null,
    slaOk: null,
    freshnessLagSeconds: { value: null, measured: false },
    engine: "authored",
    description: null,
    graph: null,
    config: [],
    runAs: ME,
    definition: {
      inputs: [{ name: "orders_raw", zone: "bronze", table: "orders_raw" }],
      steps: [{ name: "transformed", kind: "transform", reads: "orders_raw", transforms: ["dedupe(id)"] }],
      sourceZone: "bronze",
      sourceTable: "orders_raw",
      incrementalColumn: "updated_at",
      transforms: ["dedupe(id)"],
      fbicEnabled: false,
      targetZone: "silver",
      targetTable: "orders_clean",
      connectorId: null,
    },
    dependsOn: ["file_ingest_job", "pl-raw-loads-2"],
    ...over,
  } as PipelineDetail
}

/** `existing()` with its first input changed. */
function withInput(input: { name: string; zone: string; table: string }, over: Partial<PipelineDetail> = {}) {
  const base = existing(over)
  const definition = base.definition!
  return {
    ...base,
    definition: {
      ...definition,
      inputs: [input],
      steps: [{ name: "transformed", kind: "transform" as const, reads: input.name, transforms: ["dedupe(id)"] }],
    },
  }
}

const reuploadSwitch = () => screen.queryByRole("switch", { name: /re-uploaded/i })
const saveButton = (name: RegExp) => screen.getByRole("button", { name }) as HTMLButtonElement

/** base-ui's Select commits on the pointer sequence, not on a bare click. */
function pickOption(option: HTMLElement) {
  fireEvent.pointerDown(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseDown(option, { button: 0 })
  fireEvent.pointerUp(option, { button: 0, pointerType: "mouse" })
  fireEvent.mouseUp(option, { button: 0 })
  fireEvent.click(option)
}

/** Opens one of the two tabs. */
async function openTab(name: "Build" | "Schedule and settings") {
  const tab = screen.getByRole("tab", { name })
  fireEvent.click(tab)
  await waitFor(() => expect(tab.getAttribute("aria-selected")).toBe("true"))
}

/** Selects an item of the rail by the text it shows. */
function selectRail(label: string | RegExp) {
  const rail = screen.getByRole("navigation", { name: "Pipeline parts" })
  fireEvent.click(within(rail).getByRole("button", { name: label }))
}

/** Selects the output, the one item of the rail's Output group. */
function selectOutput() {
  const group = screen.getByRole("group", { name: "Output" })
  fireEvent.click(within(group).getByRole("button"))
}

/** Saves, once the page allows it, and returns the body sent. */
async function saveAndRead(method: "POST" | "PUT") {
  const button = saveButton(method === "POST" ? /create pipeline/i : /save changes/i)
  await waitFor(() => expect(button.disabled).toBe(false))
  fireEvent.click(button)
  await waitFor(() => expect(sent.some((c) => c.method === method)).toBe(true))
  return sent.find((c) => c.method === method)?.body ?? {}
}

/**
 * Fills a new pipeline with one Bronze input and one form step that dedupes
 * on `id`, picked from the table's own columns (WEB-8 D8), and names the
 * output.
 */
async function fillNewPipeline() {
  fireEvent.change(screen.getByPlaceholderText("orders_hourly_rollup"), { target: { value: "orders_clean" } })
  fireEvent.change(screen.getByPlaceholderText("orders_events"), { target: { value: "orders_raw" } })
  await addDedupeStep()
  selectOutput()
  fireEvent.change(screen.getByPlaceholderText("orders_hourly"), { target: { value: "orders_clean" } })
}

/** Adds a form step and its one dedupe operation on `id`. */
async function addDedupeStep() {
  fireEvent.click(screen.getByRole("button", { name: "Form step" }))
  fireEvent.click(await screen.findByRole("combobox", { name: "Key column" }))
  pickOption(await screen.findByRole("option", { name: "id" }))
  fireEvent.click(screen.getByRole("button", { name: /add transform/i }))
}

describe("the re-upload switch in the pipeline editor", () => {
  it("is offered, and ON, for a Bronze first input — on a new pipeline and on a stored one", async () => {
    const { unmount } = renderEditor()
    await openTab("Schedule and settings")
    await waitFor(() => expect(reuploadSwitch()).not.toBeNull())
    // A pipeline that was never saved defaults ON: the feature's point.
    expect(reuploadSwitch()?.getAttribute("aria-checked")).toBe("true")
    unmount()

    renderEditor(existing())
    await openTab("Schedule and settings")
    const stored = await screen.findByRole("switch", { name: /re-uploaded/i })
    expect(stored.getAttribute("aria-checked")).toBe("true")
  })

  it("is not offered at all when the first input is not in Bronze", async () => {
    renderEditor(withInput({ name: "orders_clean", zone: "silver", table: "orders_clean" }))
    await screen.findByPlaceholderText("orders_events")
    await openTab("Schedule and settings")
    expect(reuploadSwitch()).toBeNull()
  })

  it("carries every other upstream through a save, and only this one is the switch", async () => {
    renderEditor(existing())
    await openTab("Schedule and settings")
    const stored = await screen.findByRole("switch", { name: /re-uploaded/i })
    expect(stored.getAttribute("aria-checked")).toBe("true")

    const body = await saveAndRead("PUT")
    // `pl-raw-loads-2` is not this switch's business and survives it.
    expect(body.dependsOn).toEqual(["file_ingest_job", "pl-raw-loads-2"])
  })

  it("clears the upload upstream when the switch is turned off, keeping the rest", async () => {
    renderEditor(existing())
    await openTab("Schedule and settings")
    const stored = await screen.findByRole("switch", { name: /re-uploaded/i })
    fireEvent.click(stored)
    expect(stored.getAttribute("aria-checked")).toBe("false")

    const body = await saveAndRead("PUT")
    expect(body.dependsOn).toEqual(["pl-raw-loads-2"])
  })

  it("adds the upload upstream on create, and sends no other upstream", async () => {
    renderEditor()
    await fillNewPipeline()

    const body = await saveAndRead("POST")
    expect(body.dependsOn).toEqual(["file_ingest_job"])
  })
})

describe("table suggestions in the pipeline editor", () => {
  it("offers the zone's real table names and still accepts a typed one", async () => {
    renderEditor()
    const input = await screen.findByPlaceholderText("orders_events")
    // The suggestions come from `/api/lakehouse/tables` for the input's
    // zone, once the zone is one the lakehouse client lists.
    await waitFor(() => expect(input.getAttribute("list")).toBe("input-table-options-input-1"))
    const options = Array.from(document.querySelectorAll("#input-table-options-input-1 option")).map((o) =>
      o.getAttribute("value")
    )
    expect(options).toEqual(["orders_raw", "stock_raw"])

    // Typing is still allowed: a datalist filters, it never narrows.
    fireEvent.change(input, { target: { value: "typed_by_hand" } })
    expect((input as HTMLInputElement).value).toBe("typed_by_hand")
  })
})

// WEB-8 D8: zones are the lakehouse's namespaces and columns are the input
// table's own schema, both read from the page's existing service calls.
describe("the zone and column pickers in the pipeline editor (WEB-8 D8)", () => {
  it("offers the namespaces the page loaded as an input's zone", async () => {
    renderEditor()
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Zone" }).textContent).toContain("bronze"))
    fireEvent.click(screen.getByRole("combobox", { name: "Zone" }))
    // `within` the popup: only the open list's own options are read.
    expect(within(await screen.findByRole("listbox")).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "bronze",
      "silver",
    ])
  })

  it("offers the namespaces as the output zone too", async () => {
    renderEditor(existing())
    selectOutput()
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Output zone" }).textContent).toContain("silver"))
  })

  it("takes the columns, with their types, from the input table's schema once the table exists", async () => {
    renderEditor()
    fireEvent.change(screen.getByPlaceholderText("orders_events"), { target: { value: "orders_raw" } })
    // The input pane lists the columns and their types; the line that stood
    // in for them is gone.
    const list = await screen.findByRole("list", { name: "Columns of this table" })
    expect(within(list).getAllByRole("listitem").map((item) => item.textContent)).toEqual([
      "id long",
      "updated_at timestamp",
      "status string",
    ])
    expect(screen.queryByText("Columns appear here once the table exists")).toBeNull()
    // A form step reading the input picks from those columns, not free text.
    fireEvent.click(screen.getByRole("button", { name: "Form step" }))
    const column = await screen.findByRole("combobox", { name: "Key column" })
    fireEvent.click(column)
    expect(within(await screen.findByRole("listbox")).getAllByRole("option").map((o) => o.textContent)).toEqual([
      "id",
      "updated_at",
      "status",
    ])
  })

  it("keeps the fields free text, and says so, while the table does not exist", async () => {
    renderEditor()
    fireEvent.change(screen.getByPlaceholderText("orders_events"), { target: { value: "no_such_table" } })
    // The detail read answers 404: the columns are unknown, so the page says
    // why instead of showing an empty list, and a step's field is typed.
    await waitFor(() => expect(screen.getByText("Columns appear here once the table exists")).toBeTruthy())
    fireEvent.click(screen.getByRole("button", { name: "Form step" }))
    await waitFor(() => expect(screen.getByRole("textbox", { name: "Key column" })).toBeTruthy())
    expect(screen.getByText("Columns appear here once the table exists")).toBeTruthy()
  })
})

describe("the connector page's Create pipeline entry (BLD-7)", () => {
  const connectorSwitch = () => screen.queryByRole("switch", { name: /connector loads the table/i })
  const fixedZone = () => screen.getByRole("textbox", { name: /^zone, fixed/i }) as HTMLInputElement
  const tablePicker = () => screen.getByRole("combobox", { name: /^table/i })

  beforeEach(() => {
    search = `connectorId=${SEED_CONNECTOR}`
  })

  it("fixes the zone to bronze, offers only that connector's tables, and switches the trigger to its load", async () => {
    renderEditor()

    // Zone: shown, not editable (decision 5).
    await waitFor(() => expect(fixedZone().value).toBe("bronze"))
    expect(fixedZone().disabled).toBe(true)
    expect(fixedZone().readOnly).toBe(true)

    // Table: exactly the connector's ingest targets. The prompt is the
    // placeholder (WEB-9 D2), so the list holds the two tables.
    await waitFor(() => expect(tablePicker().textContent).toContain("Choose one of the connector's tables"))
    await act(async () => {})
    fireEvent.click(tablePicker())
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual(["customers", "orders"])

    // Trigger: the connector's load, ON by default; the upload switch
    // belongs to a table this pipeline uploads, not to a connector's.
    await openTab("Schedule and settings")
    expect(connectorSwitch()?.getAttribute("aria-checked")).toBe("true")
    expect(reuploadSwitch()).toBeNull()
  })

  it("sends the chosen table as the input and the connector's load as the pipeline's upstream", async () => {
    renderEditor()
    fireEvent.change(screen.getByPlaceholderText("orders_hourly_rollup"), { target: { value: "northwind_orders_clean" } })
    await waitFor(() => expect(tablePicker()).toBeTruthy())
    await act(async () => {})
    chooseOption(tablePicker(), "orders")

    // The connector's table does not exist in the lakehouse listing here,
    // so the key column is typed.
    fireEvent.click(screen.getByRole("button", { name: "Form step" }))
    fireEvent.change(await screen.findByPlaceholderText("order_id"), { target: { value: "id" } })
    fireEvent.click(screen.getByRole("button", { name: /add transform/i }))
    selectOutput()
    fireEvent.change(screen.getByPlaceholderText("orders_hourly"), { target: { value: "northwind_orders" } })

    const body = await saveAndRead("POST")
    expect(body.inputs).toEqual([{ name: "orders", zone: "bronze", table: "orders" }])
    expect(body.connectorId).toBe(SEED_CONNECTOR)
    // Decision 4: the pipeline runs after that table's load, and the upload
    // job is not part of a connector-fed pipeline.
    expect(body.dependsOn).toEqual(["ingest_job"])
  })

  it("says to run the ingest first, and links to the Ingest tab, when the connector has loaded nothing", async () => {
    ingestTargets = []
    renderEditor()
    expect(await screen.findByText(/has not loaded any table yet/)).not.toBeNull()
    const link = screen.getByRole("link", { name: /open its ingest tab/i })
    expect(link.getAttribute("href")).toBe(`/connectors/${SEED_CONNECTOR}?tab=ingest`)
    // No picker to choose from, so the pipeline cannot be saved.
    expect(screen.queryByRole("combobox", { name: /^table/i })).toBeNull()
    expect(saveButton(/create pipeline/i).disabled).toBe(true)
  })

  it("keeps the plain form when there is no connector in the URL", async () => {
    search = ""
    renderEditor()
    // The zone is the ordinary picker, and the connector-load switch is not
    // offered on a pipeline that has no connector.
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Zone" })).toBeTruthy())
    expect(screen.queryByRole("textbox", { name: /^zone, fixed/i })).toBeNull()
    await openTab("Schedule and settings")
    await waitFor(() => expect(reuploadSwitch()).not.toBeNull())
    expect(connectorSwitch()).toBeNull()
  })
})

describe("changing a pipeline's connector on the edit form (BLD-9)", () => {
  const connectorPicker = () => screen.getByRole("combobox", { name: "Connector" })
  const fixedZone = () => screen.queryByRole("textbox", { name: /^zone, fixed/i }) as HTMLInputElement | null

  it("offers every connector and No connector, and sends null when the connector is cleared", async () => {
    // The connector's own table, as `BLD-7` requires.
    const stored = withInput(
      { name: "orders", zone: "bronze", table: "orders" },
      { dependsOn: ["ingest_job", "pl-raw-loads-2"] }
    )
    stored.definition = { ...stored.definition, connectorId: SEED_CONNECTOR }
    renderEditor(stored)

    // With a connector the zone is fixed to bronze (decision 5).
    await waitFor(() => expect(fixedZone()?.value).toBe("bronze"))
    expect(fixedZone()?.disabled).toBe(true)

    await openTab("Schedule and settings")
    await waitFor(() => expect(connectorPicker().textContent).toContain("Lakehouse OLTP (Postgres)"))
    // WEB-9 D2: "No connector" is a real choice, so it stays an option.
    chooseOption(connectorPicker(), "No connector (read a ClickHouse table directly)")
    fireEvent.click(connectorPicker())
    expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([
      "No connector (read a ClickHouse table directly)",
      "Lakehouse OLTP (Postgres)",
    ])
    // "No connector" is a real choice: the zone becomes editable again — a
    // picker over the lakehouse's namespaces (WEB-8 D8).
    await openTab("Build")
    await waitFor(() => expect(screen.getByRole("combobox", { name: "Zone" })).toBeTruthy())

    const body = await saveAndRead("PUT")
    expect(body.connectorId).toBeNull()
    // Clearing the connector does not rewrite the rest of the chain: the form
    // owns one upstream at a time, and `ingest_job` is still accepted on a
    // Bronze first input.
    expect(body.dependsOn).toEqual(["ingest_job", "pl-raw-loads-2"])
  })

  it("sets the connector picked in the edit form", async () => {
    // A Bronze input whose table the connector loads, so the API will accept
    // the pair.
    const stored = withInput({ name: "orders", zone: "bronze", table: "orders" }, { dependsOn: ["file_ingest_job"] })
    renderEditor(stored)

    await openTab("Schedule and settings")
    await waitFor(() =>
      expect(connectorPicker().textContent).toContain("No connector (read a ClickHouse table directly)")
    )
    chooseOption(connectorPicker(), "Lakehouse OLTP (Postgres)")
    // The input switches to the connector's shape as soon as it is picked:
    // zone fixed, table from the connector's own targets.
    await openTab("Build")
    await waitFor(() => expect(fixedZone()?.value).toBe("bronze"))
    expect(fixedZone()?.disabled).toBe(true)
    await waitFor(() => expect(screen.getByRole("combobox", { name: /^table/i }).textContent).toContain("orders"))

    const body = await saveAndRead("PUT")
    expect(body.connectorId).toBe(SEED_CONNECTOR)
    expect(body.inputs).toEqual([{ name: "orders", zone: "bronze", table: "orders" }])
  })
})

// BLD-10: a pipeline with no step is a plain copy chosen on purpose rather
// than a side effect, and FBIC is sent as nothing runs it.
describe("FBIC and the plain copy in the pipeline editor (BLD-10)", () => {
  const plainCopySwitch = () => screen.getByRole("switch", { name: /copy the input as it is/i })

  /** Fills the name, the input and the output; no step. */
  async function fillWithoutStep() {
    fireEvent.change(screen.getByPlaceholderText("orders_hourly_rollup"), { target: { value: "orders_clean" } })
    fireEvent.change(screen.getByPlaceholderText("orders_events"), { target: { value: "orders_raw" } })
    selectOutput()
    fireEvent.change(screen.getByPlaceholderText("orders_hourly"), { target: { value: "orders_clean" } })
    await waitFor(() => expect(plainCopySwitch()).toBeTruthy())
  }

  it("keeps Create disabled on a pipeline with no step until the plain copy is switched on", async () => {
    renderEditor()
    await fillWithoutStep()

    const create = saveButton(/create pipeline/i)
    expect(create.disabled).toBe(true)
    expect(screen.getByText("Add a step, or choose to copy the input as it is.")).toBeTruthy()

    fireEvent.click(plainCopySwitch())
    await waitFor(() => expect(create.disabled).toBe(false))
  })

  it("switches the plain copy off and disables it when a step is added", async () => {
    renderEditor()
    await fillWithoutStep()

    fireEvent.click(plainCopySwitch())
    await waitFor(() => expect(plainCopySwitch().getAttribute("aria-checked")).toBe("true"))

    fireEvent.click(screen.getByRole("button", { name: "Form step" }))
    selectOutput()
    await waitFor(() => expect(plainCopySwitch().getAttribute("aria-checked")).toBe("false"))
    expect(plainCopySwitch().getAttribute("aria-disabled")).toBe("true")
  })

  it("says that a pipeline with no step is a plain copy of its input", async () => {
    renderEditor()
    await fillWithoutStep()
    expect(screen.getByText("No steps: the output is a copy of the input.")).toBeTruthy()
  })

  it("sends fbicEnabled false on a create, because nothing runs it", async () => {
    renderEditor()
    await fillWithoutStep()
    fireEvent.click(plainCopySwitch())

    const body = await saveAndRead("POST")
    expect(body.fbicEnabled).toBe(false)
  })

  it("sends a stored fbicEnabled true back on an edit, unchanged", async () => {
    const stored = existing()
    stored.definition = { ...stored.definition!, fbicEnabled: true }
    renderEditor(stored)

    const body = await saveAndRead("PUT")
    expect(body.fbicEnabled).toBe(true)
  })
})

// BLD-1 part 1 S3-T5: what the editor adds over the wizard.
describe("the pipeline editor of inputs and steps (BLD-1 part 1)", () => {
  /** A stored pipeline of two inputs, a form step and a SQL step (plan D1). */
  function twoSteps(over: Partial<PipelineDetail> = {}): PipelineDetail {
    const base = existing(over)
    return {
      ...base,
      definition: {
        ...base.definition!,
        inputs: [
          { name: "orders_raw", zone: "bronze", table: "orders_raw" },
          { name: "stock", zone: "bronze", table: "stock_raw" },
        ],
        steps: [
          { name: "paid", kind: "transform", reads: "orders_raw", transforms: ["filter(status = 'paid')"] },
          { name: "by_stock", kind: "sql", sql: "SELECT * FROM paid JOIN stock ON paid.id = stock.id" },
        ],
      },
    }
  }

  /** Waits until the session is known, so a check of who reads as whom is not made against an unknown user. */
  async function signedIn() {
    await waitFor(() => expect(sent.some((c) => c.path === "/api/auth/me")).toBe(true))
    await act(async () => {})
  }

  it("opens a pipeline made before this change as one input and one form step (BLD-1-AC9)", async () => {
    renderEditor(existing())
    const inputs = screen.getByRole("group", { name: /^Inputs/ })
    expect(within(inputs).getAllByRole("button", { name: /orders_raw/ })).toHaveLength(1)
    const steps = screen.getByRole("group", { name: /^Steps/ })
    expect(within(steps).getByRole("button", { name: /1\. transformed/ })).toBeTruthy()

    selectRail(/1\. transformed/)
    expect(await screen.findByText("Step 1 · Form")).toBeTruthy()
    expect(screen.getByRole("combobox", { name: "Reads" }).textContent).toContain("orders_raw")
    expect(within(screen.getByRole("list", { name: "Operations of this step" })).getByText("dedupe(id)")).toBeTruthy()
  })

  it("does not offer a sixth input (decision 4)", async () => {
    renderEditor()
    for (let n = 0; n < 4; n += 1) fireEvent.click(screen.getByRole("button", { name: "Add input" }))
    expect(screen.getByRole("group", { name: "Inputs · 5 of 5" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: "Add input" })).toBeNull()
  })

  it("completes a SQL step with the inputs and the earlier steps", async () => {
    renderEditor(existing())
    fireEvent.click(screen.getByRole("button", { name: "SQL step" }))
    expect(await screen.findByText("Step 2 · SQL")).toBeTruthy()
    const content = await waitFor(() => {
      const found = document.querySelector(".cm-content")
      if (!found) throw new Error("no editor yet")
      return found as HTMLElement
    })
    const view = EditorView.findFromDOM(content)
    if (!view) throw new Error("no editor view")
    const text = "SELECT * FROM "
    act(() => {
      view.dispatch({ changes: { from: 0, to: view.state.doc.length, insert: text }, selection: { anchor: text.length } })
    })
    startCompletion(view)
    // The editor waits a moment before it asks its sources.
    await new Promise((resolve) => setTimeout(resolve, 250))
    const labels = currentCompletions(view.state).map((completion) => completion.label)
    expect(labels).toContain("orders_raw")
    expect(labels).toContain("transformed")
    // A step does not read itself.
    expect(labels).not.toContain("step_2")
  })

  it("selects the step a 422 names, and shows the message in its pane (D7)", async () => {
    const message = "steps[1]: you may not read a table this step reads"
    saveAnswer = { status: 422, body: { error: message } }
    renderEditor(twoSteps())
    // The first input is selected when the page opens.
    expect(await screen.findByText("Input 1")).toBeTruthy()

    fireEvent.click(saveButton(/save changes/i))

    expect(await screen.findByText("Step 2 · SQL")).toBeTruthy()
    const alert = screen.getByRole("alert")
    expect(alert.textContent).toBe(message)
    // The rail item is the selected one, and is described by the message.
    const item = screen.getByRole("button", { name: /2\. by_stock/ })
    expect(item.getAttribute("aria-current")).toBe("true")
    expect(item.getAttribute("aria-describedby")).toBe(alert.id)
  })

  it("previews the selected input, and marks the preview stale after an edit", async () => {
    renderEditor(existing())
    fireEvent.click(screen.getByRole("button", { name: "Preview" }))
    expect(await screen.findByText("First 2 rows.")).toBeTruthy()
    const call = sent.find((c) => c.path === "/api/pipelines/preview")
    expect(call?.body).toEqual({
      inputs: [{ name: "orders_raw", zone: "bronze", table: "orders_raw" }],
      steps: [],
      upTo: "orders_raw",
    })
    expect(screen.queryByText("Stale: the definition changed since this preview ran.")).toBeNull()

    fireEvent.change(screen.getByLabelText("Name in this pipeline"), { target: { value: "orders" } })
    expect(await screen.findByText("Stale: the definition changed since this preview ran.")).toBeTruthy()
  })

  it("shows the preview's refusal where the preview is", async () => {
    previewAnswer = { status: 400, body: { error: "inputs[0]: a zone and a table must be plain names" } }
    renderEditor(existing())
    fireEvent.click(screen.getByRole("button", { name: "Preview" }))
    expect((await screen.findByRole("alert")).textContent).toBe("inputs[0]: a zone and a table must be plain names")
  })

  // BLD-1-AC11, WEB-11: the shared refusal sentence, and no item selected
  // for it, because it names none.
  it("shows the shared sentence when the person may not preview", async () => {
    previewAnswer = { status: 403, body: { error: "permission_denied: pipeline:write" } }
    renderEditor(existing())
    fireEvent.click(screen.getByRole("button", { name: "Preview" }))
    expect((await screen.findByRole("alert")).textContent).toBe(
      "You don't have access. This needs the pipeline:write permission."
    )
  })

  it("creates with inputs, steps and kind batch, and none of the fields of before (D3, decision 18)", async () => {
    renderEditor()
    await fillNewPipeline()

    const body = await saveAndRead("POST")
    expect(body.kind).toBe("batch")
    expect(body.inputs).toEqual([{ name: "orders_raw", zone: "bronze", table: "orders_raw" }])
    expect(body.steps).toEqual([{ name: "step_1", kind: "transform", reads: "orders_raw", transforms: ["dedupe(id)"] }])
    for (const old of ["sourceZone", "sourceTable", "transforms", "incrementalColumn"]) {
      expect(Object.keys(body)).not.toContain(old)
    }
  })

  it("sends the stored kind back on an edit, and no incremental column (decision 18)", async () => {
    renderEditor(existing({ kind: "incremental" }))

    const body = await saveAndRead("PUT")
    expect(body.kind).toBe("incremental")
    expect(body.inputs).toEqual([{ name: "orders_raw", zone: "bronze", table: "orders_raw" }])
    expect(body.steps).toEqual([{ name: "transformed", kind: "transform", reads: "orders_raw", transforms: ["dedupe(id)"] }])
    for (const old of ["sourceZone", "sourceTable", "transforms", "incrementalColumn"]) {
      expect(Object.keys(body)).not.toContain(old)
    }
  })

  it("says before saving that the pipeline will read as you, when its steps change and it reads as someone else (D17a)", async () => {
    renderEditor(existing({ runAs: "u-author", runAsName: "Author" }))
    await signedIn()
    // A change to the schedule only does not move the reader (D10).
    await openTab("Schedule and settings")
    fireEvent.click(screen.getByRole("radio", { name: "Hourly" }))
    expect(screen.queryByText("This pipeline will read as you from now on.")).toBeNull()

    await openTab("Build")
    fireEvent.click(screen.getByRole("button", { name: "SQL step" }))
    expect(await screen.findByText("This pipeline will read as you from now on.")).toBeTruthy()
  })

  it("does not say so when the pipeline already reads as you", async () => {
    renderEditor(existing({ runAs: ME }))
    await signedIn()
    fireEvent.click(screen.getByRole("button", { name: "SQL step" }))
    expect(await screen.findByText("Step 2 · SQL")).toBeTruthy()
    expect(screen.queryByText("This pipeline will read as you from now on.")).toBeNull()
  })

  it("says the preview is what you may see when the pipeline reads as someone else (decision 13)", async () => {
    renderEditor(existing({ runAs: "u-author", runAsName: "Author" }))
    await signedIn()
    expect(
      await screen.findByText(/The pipeline reads as Author, so what it writes can differ\./)
    ).toBeTruthy()
  })

  it("moves and removes a step, and renames a step's reader with its input", async () => {
    renderEditor(twoSteps())
    selectRail(/2\. by_stock/)
    fireEvent.click(await screen.findByRole("button", { name: "Move by_stock up" }))
    const steps = screen.getByRole("group", { name: /^Steps/ })
    expect(within(steps).getAllByRole("button").map((b) => b.textContent?.match(/^\d\. [a-z_]+/)?.[0]).filter(Boolean)).toEqual([
      "1. by_stock",
      "2. paid",
    ])
    selectRail(/1\. by_stock/)
    fireEvent.click(screen.getByRole("button", { name: "Remove step" }))
    expect(within(steps).queryByRole("button", { name: /by_stock/ })).toBeNull()

    selectRail(/^orders_raw/)
    fireEvent.change(screen.getByLabelText("Name in this pipeline"), { target: { value: "orders" } })
    expect(within(steps).getByRole("button", { name: /1\. paid/ }).textContent).toContain("reads orders")
  })
})
