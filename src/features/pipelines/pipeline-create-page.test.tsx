// The pipeline editor's re-upload switch and source-table suggestions
// (DATA-19 D3 and D4), rendered, and the connector-page entry (BLD-7
// decisions 4 and 5). The pure rules behind the switches are pinned in
// `pipeline-reupload-trigger.test.ts` and
// `pipeline-connector-trigger.test.ts`; what is proved HERE is the
// wiring: that the switch exists only for a Bronze source, that it
// carries the STORED chain through a save, that switching it off clears
// the upload upstream, that the source-table suggestions come from the
// lakehouse client's own namespace listing rather than a hardcoded list,
// and that `?connectorId=` fixes the zone, picks from that connector's
// ingest targets and chains its load.
//
// Everything below the fetch is real: the real service clients, the real
// `apiFetch`, the real component. Only `fetch` is stubbed.
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

import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock } from "bun:test"
import type { PipelineDetail } from "@/services/contracts/pipelines"
import { PipelineEditor } from "./pipeline-create-page"

const originalFetch = global.fetch
/** Every body the form sent, by method and path. */
const sent: { method: string; path: string; body: Record<string, unknown> }[] = []

/** The connector a QA report made its pipeline from, and its ingest targets. */
const SEED_CONNECTOR = "conn-pg-lakehouse"
let ingestTargets: { name: string; target: string }[] = []

function jsonResponse(body: unknown) {
  return new Response(JSON.stringify(body), {
    status: 200,
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
  ingestTargets = [
    { name: "public.orders", target: "orders" },
    { name: "public.customers", target: "customers" },
  ]
  global.fetch = mock(async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    let body: Record<string, unknown> = {}
    if (typeof init?.body === "string") body = JSON.parse(init.body)
    sent.push({ method, path: url.split("?")[0] ?? url, body })
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
      return new Response(JSON.stringify([]), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
    }
    if (url.startsWith("/api/lakehouse/namespaces")) {
      return new Response(JSON.stringify({ namespaces: [{ name: "bronze", tableCount: { value: 2, measured: true } }] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
    }
    if (url.startsWith("/api/lakehouse/tables")) {
      return new Response(JSON.stringify({ tables: [table("stock_raw"), table("orders_raw")] }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
    }
    if (url.startsWith("/api/pipelines") && (method === "POST" || method === "PUT")) {
      return new Response(JSON.stringify({ id: "pl-1", name: "orders_clean" }), {
        status: 200,
        headers: { "Content-Type": "application/json" },
      })
    }
    return new Response("{}", { status: 200, headers: { "Content-Type": "application/json" } })
  }) as unknown as typeof fetch
})

afterEach(() => {
  global.fetch = originalFetch
  cleanup()
})

/** A stored authored pipeline, `dependsOn` whatever the test passes. */
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
    definition: {
      sourceZone: "bronze",
      sourceTable: "orders_raw",
      // Step 0 of the form asks for a non-blank incremental column before
      // it lets anybody reach the steps after it, so a stored pipeline
      // with none cannot be walked past in a test.
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

const reuploadSwitch = () => screen.queryByRole("switch", { name: /re-uploaded/i })

/**
 * The form's submit button only exists on its LAST step
 * (`FormStepLayout`), so a save means walking there: every step's fields
 * are already valid for the fixture, so "Next" is enough.
 */
async function advance() {
  const next = screen.getByRole("button", { name: "Next" })
  await waitFor(() => expect((next as HTMLButtonElement).disabled).toBe(false))
  fireEvent.click(next)
}

async function walkToReview() {
  // source -> transform -> target -> schedule -> review
  for (let step = 0; step < 4; step += 1) await advance()
}

describe("the re-upload switch in the pipeline editor", () => {
  it("is offered, and ON, for a Bronze source — on a new pipeline and on a stored one", async () => {
    const { unmount } = render(<PipelineEditor />)
    await waitFor(() => expect(reuploadSwitch()).not.toBeNull())
    // A pipeline that was never saved defaults ON: the feature's point.
    expect(reuploadSwitch()?.getAttribute("aria-checked")).toBe("true")
    unmount()

    render(<PipelineEditor existing={existing()} />)
    const stored = await screen.findByRole("switch", { name: /re-uploaded/i })
    expect(stored.getAttribute("aria-checked")).toBe("true")
  })

  it("is not offered at all when the source zone is not Bronze", async () => {
    render(<PipelineEditor existing={existing({ definition: { ...existing().definition!, sourceZone: "silver", sourceTable: "orders_clean" } })} />)
    await screen.findByPlaceholderText("orders_events")
    expect(reuploadSwitch()).toBeNull()
  })

  it("carries every other upstream through a save, and only this one is the switch", async () => {
    render(<PipelineEditor existing={existing()} />)
    const stored = await screen.findByRole("switch", { name: /re-uploaded/i })
    expect(stored.getAttribute("aria-checked")).toBe("true")

    await walkToReview()
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }))
    await waitFor(() => expect(sent.some((c) => c.method === "PUT")).toBe(true))
    const save = sent.find((c) => c.method === "PUT")
    // `pl-raw-loads-2` is not this switch's business and survives it.
    expect(save?.body.dependsOn).toEqual(["file_ingest_job", "pl-raw-loads-2"])
  })

  it("clears the upload upstream when the switch is turned off, keeping the rest", async () => {
    render(<PipelineEditor existing={existing()} />)
    const stored = await screen.findByRole("switch", { name: /re-uploaded/i })
    fireEvent.click(stored)
    expect(stored.getAttribute("aria-checked")).toBe("false")

    await walkToReview()
    fireEvent.click(screen.getByRole("button", { name: /save changes/i }))
    await waitFor(() => expect(sent.some((c) => c.method === "PUT")).toBe(true))
    const save = sent.find((c) => c.method === "PUT")
    expect(save?.body.dependsOn).toEqual(["pl-raw-loads-2"])
  })

  it("adds the upload upstream on create, and sends no other upstream", async () => {
    render(<PipelineEditor />)
    await waitFor(() => expect(reuploadSwitch()).not.toBeNull())
    // Step 0: a name and a source table (zone and incremental column
    // carry their defaults). Step 1 wants a transform: the one transform
    // this form can add with nothing but a placeholder is a dedupe key.
    fireEvent.change(screen.getByPlaceholderText("orders_hourly_rollup"), { target: { value: "orders_clean" } })
    fireEvent.change(screen.getByPlaceholderText("orders_events"), { target: { value: "orders_raw" } })
    await advance()
    fireEvent.change(screen.getByPlaceholderText("order_id"), { target: { value: "id" } })
    fireEvent.click(screen.getByRole("button", { name: /add transform/i }))
    // Step 2: a target table; step 3's cron is the default.
    await advance()
    fireEvent.change(screen.getByPlaceholderText("orders_hourly"), { target: { value: "orders_clean" } })
    await advance()
    await advance()

    fireEvent.click(screen.getByRole("button", { name: /create pipeline/i }))
    await waitFor(() => expect(sent.some((c) => c.method === "POST")).toBe(true))
    expect(sent.find((c) => c.method === "POST")?.body.dependsOn).toEqual(["file_ingest_job"])
  })
})

describe("source-table suggestions in the pipeline editor", () => {
  it("offers the zone's real table names and still accepts a typed one", async () => {
    render(<PipelineEditor />)
    const input = await screen.findByPlaceholderText("orders_events")
    // The suggestions come from `/api/lakehouse/tables` for the source
    // zone, once the zone is one the lakehouse client lists.
    await waitFor(() =>
      expect(input.getAttribute("list")).toBe("source-table-options")
    )
    const options = Array.from(
      document.querySelectorAll("#source-table-options option")
    ).map((o) => o.getAttribute("value"))
    expect(options).toEqual(["orders_raw", "stock_raw"])

    // Typing is still allowed: a datalist filters, it never narrows.
    fireEvent.change(input, { target: { value: "typed_by_hand" } })
    expect((input as HTMLInputElement).value).toBe("typed_by_hand")
  })
})

describe("the connector page's Create pipeline entry (BLD-7)", () => {
  const connectorSwitch = () =>
    screen.queryByRole("switch", { name: /connector loads the table/i })
  const sourceZoneInput = () =>
    screen.getByRole("textbox", { name: /^source zone/i }) as HTMLInputElement
  const sourceTablePicker = () =>
    screen.getByRole("combobox", { name: /^source table/i }) as HTMLSelectElement

  beforeEach(() => {
    search = `connectorId=${SEED_CONNECTOR}`
  })

  it("fixes the zone to bronze, offers only that connector's tables, and switches the trigger to its load", async () => {
    render(<PipelineEditor />)

    // Zone: shown, not editable (decision 5).
    await waitFor(() => expect(sourceZoneInput().value).toBe("bronze"))
    expect(sourceZoneInput().disabled).toBe(true)
    expect(sourceZoneInput().readOnly).toBe(true)

    // Table: exactly the connector's ingest targets, plus "choose one".
    await waitFor(() =>
      expect(Array.from(sourceTablePicker().options).map((o) => o.value)).toEqual([
        "",
        "customers",
        "orders",
      ])
    )

    // Trigger: the connector's load, ON by default; the upload switch
    // belongs to a table this pipeline uploads, not to a connector's.
    expect(connectorSwitch()?.getAttribute("aria-checked")).toBe("true")
    expect(reuploadSwitch()).toBeNull()
  })

  it("sends the chosen table and the connector's load as the pipeline's upstream", async () => {
    render(<PipelineEditor />)
    fireEvent.change(screen.getByPlaceholderText("orders_hourly_rollup"), {
      target: { value: "northwind_orders_clean" },
    })
    await waitFor(() => expect(sourceTablePicker().options.length).toBe(3))
    fireEvent.change(sourceTablePicker(), { target: { value: "orders" } })

    await advance()
    fireEvent.change(screen.getByPlaceholderText("order_id"), { target: { value: "id" } })
    fireEvent.click(screen.getByRole("button", { name: /add transform/i }))
    await advance()
    fireEvent.change(screen.getByPlaceholderText("orders_hourly"), {
      target: { value: "northwind_orders" },
    })
    await advance()
    await advance()

    fireEvent.click(screen.getByRole("button", { name: /create pipeline/i }))
    await waitFor(() => expect(sent.some((c) => c.method === "POST")).toBe(true))
    const body = sent.find((c) => c.method === "POST")?.body
    expect(body?.sourceZone).toBe("bronze")
    expect(body?.sourceTable).toBe("orders")
    expect(body?.connectorId).toBe(SEED_CONNECTOR)
    // Decision 4: the pipeline runs after that table's load, and the
    // upload job is not part of a connector-sourced pipeline.
    expect(body?.dependsOn).toEqual(["ingest_job"])
  })

  it("says to run the ingest first, and links to the Ingest tab, when the connector has loaded nothing", async () => {
    ingestTargets = []
    render(<PipelineEditor />)
    expect(await screen.findByText(/has not loaded any table yet/)).not.toBeNull()
    const link = screen.getByRole("link", { name: /open its ingest tab/i })
    expect(link.getAttribute("href")).toBe(`/connectors/${SEED_CONNECTOR}?tab=ingest`)
    // No picker to choose from, so the step cannot be left.
    expect(screen.queryByRole("combobox", { name: /^source table/i })).toBeNull()
  })

  it("keeps the plain form when there is no connector in the URL", async () => {
    search = ""
    render(<PipelineEditor />)
    await waitFor(() => expect(reuploadSwitch()).not.toBeNull())
    // The zone is the ordinary editable field, and the connector-load
    // switch is not offered on a pipeline that has no connector.
    const zone = screen.getByDisplayValue("bronze") as HTMLInputElement
    expect(zone.disabled).toBe(false)
    expect(connectorSwitch()).toBeNull()
  })
})
