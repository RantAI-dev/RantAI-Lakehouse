import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import type { ReportSubscription } from "@/services/contracts/reports"
import { AS_CREATOR_SENTENCE, EMAIL_NOT_SET_UP, SubscriptionsDialog } from "./subscriptions-dialog"

const originalFetch = global.fetch

type Call = { url: string; method: string; body: unknown }

function json(body: unknown, status = 200) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } })
}

const sub: ReportSubscription = {
  id: "s1", board: "b1", ownerId: "u1",
  schedule: { kind: "weekly", weekday: 1, hour: 8, minute: 0 },
  timeZone: "Asia/Jakarta",
  attachments: { pdf: { paper: "a4", orientation: "portrait" }, charts: [{ chart: "c1", format: "xlsx", values: "formatted" }] },
  filters: [{ column: "region", values: ["Bali"] }], params: null, grain: null, skipWhenEmpty: true,
  paused: false, pausedReason: null, consecutiveFailures: 0, nextRunAt: "", createdAt: "",
  recipients: [{ userId: "u2", name: "Colleague", email: "c@company.example", active: true }, { email: "press@partner.example" }],
  lastDelivery: { id: "d1", startedAt: "2026-10-19 01:00:00 +00:00:00", finishedAt: "", status: "failed", origin: "schedule", recipientCount: 2, reason: "The PDF took too long to make." },
  canEdit: true,
}

function mockApi(opts: { items?: ReportSubscription[]; emailSetUp?: boolean; pdfSupported?: boolean; createStatus?: number; createBody?: unknown } = {}) {
  const calls: Call[] = []
  global.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
    const url = String(input)
    const method = init?.method ?? "GET"
    calls.push({ url, method, body: init?.body ? JSON.parse(String(init.body)) : undefined })
    if (url.startsWith("/api/reports/subscriptions?board=")) {
      return json({ items: opts.items ?? [], emailSetUp: opts.emailSetUp ?? true, pdfSupported: opts.pdfSupported ?? true })
    }
    if (url.startsWith("/api/reports/recipients")) return json({ items: [{ id: "u9", name: "Dewi", email: "dewi@company.example" }] })
    if (url === "/api/reports/subscriptions" && method === "POST") return json(opts.createBody ?? sub, opts.createStatus ?? 200)
    if (url.endsWith("/send")) return json({ id: "d", status: "failed", origin: "manual", recipientCount: 0, reason: EMAIL_NOT_SET_UP, startedAt: "", finishedAt: "" })
    return json({})
  }) as unknown as typeof fetch
  return calls
}

const props = {
  open: true, onOpenChange: () => {}, board: "b1", dashName: "Sales",
  charts: [{ id: "c1", title: "Visits" }, { id: "c2", title: "Revenue" }],
  readState: { filters: [{ column: "region", values: ["Bali"] }], params: null, grain: null },
  reportZone: "Asia/Jakarta",
}

beforeEach(() => { localStorage.clear() })
afterEach(() => { global.fetch = originalFetch; cleanup() })

describe("SubscriptionsDialog", () => {
  it("lists a subscription with its schedule in words, recipients, attachments and last send", async () => {
    mockApi({ items: [sub] })
    render(<SubscriptionsDialog {...props} />)
    expect(await screen.findByText("Every Monday at 08:00 (Asia/Jakarta)")).toBeDefined()
    expect(screen.getByText(/PDF and 1 chart file · to Colleague, press@partner.example/)).toBeDefined()
    expect(screen.getByText("Failed. The PDF took too long to make.")).toBeDefined()
  })

  it("says email is not set up while it is not, and still lets a subscription be made", async () => {
    mockApi({ emailSetUp: false })
    render(<SubscriptionsDialog {...props} />)
    expect((await screen.findByRole("status")).textContent).toContain(EMAIL_NOT_SET_UP)
    expect((screen.getByRole("button", { name: "New subscription" }) as HTMLButtonElement).disabled).toBe(false)
  })

  it("shows the as-you sentence above the recipients and the schedule in words as it is chosen", async () => {
    mockApi()
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    expect(screen.getByText(AS_CREATOR_SENTENCE)).toBeDefined()
    expect(screen.getByText("Every Monday at 08:00 (Asia/Jakarta)")).toBeDefined()
    fireEvent.change(screen.getByLabelText("Repeat"), { target: { value: "monthly" } })
    fireEvent.change(screen.getByLabelText("Day of the month"), { target: { value: "last" } })
    expect(screen.getByText("On the last day of every month at 08:00 (Asia/Jakarta)")).toBeDefined()
  })

  it("sends the schedule, attachments, the filters on screen and the recipients", async () => {
    const calls = mockApi()
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    fireEvent.change(screen.getByLabelText("File for Visits"), { target: { value: "csv" } })
    fireEvent.change(screen.getByLabelText("Find a person"), { target: { value: "dew" } })
    fireEvent.click(await screen.findByRole("button", { name: /Dewi/ }))
    fireEvent.change(screen.getByLabelText("Outside address"), { target: { value: "Press@Partner.Example" } })
    fireEvent.click(screen.getByRole("button", { name: "Add address" }))
    fireEvent.click(screen.getByLabelText("Skip when empty: send nothing when every chart has no rows"))
    fireEvent.click(screen.getByRole("button", { name: "Subscribe" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const post = calls.find((c) => c.method === "POST")?.body as Record<string, unknown>
    expect(post).toEqual({
      board: "b1",
      schedule: { kind: "weekly", weekday: 1, hour: 8, minute: 0 },
      timeZone: "Asia/Jakarta",
      attachments: { pdf: { paper: "a4", orientation: "portrait" }, charts: [{ chart: "c1", format: "csv", values: "formatted" }] },
      filters: [{ column: "region", values: ["Bali"] }],
      params: null,
      grain: null,
      skipWhenEmpty: true,
      recipients: [{ userId: "u9" }, { email: "press@partner.example" }],
    })
  })

  it("keeps the form open and shows the server's sentence when an outside domain is not allowed", async () => {
    mockApi({ createStatus: 400, createBody: { error: "An outside address is on a domain an administrator has not allowed for reports." } })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    fireEvent.change(screen.getByLabelText("Outside address"), { target: { value: "x@nowhere.example" } })
    fireEvent.click(screen.getByRole("button", { name: "Add address" }))
    fireEvent.click(screen.getByRole("button", { name: "Subscribe" }))
    expect((await screen.findByRole("alert")).textContent).toBe("An outside address is on a domain an administrator has not allowed for reports.")
    expect(screen.getByRole("button", { name: "Subscribe" })).toBeDefined()
  })

  it("disables the PDF choice and says why when the deployment has no PDF service", async () => {
    mockApi({ pdfSupported: false })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    const pdf = screen.getByLabelText("The dashboard as a PDF") as HTMLInputElement
    expect(pdf.disabled).toBe(true)
    expect(pdf.checked).toBe(false)
    expect(screen.getByText(/PDF service is not set up/)).toBeDefined()
  })

  it("sends now and shows the plain reason, and pauses by sending only the flag", async () => {
    const calls = mockApi({ items: [sub] })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "Send now" }))
    expect(await screen.findByText(`Not sent. ${EMAIL_NOT_SET_UP}`)).toBeDefined()
    expect(calls.some((c) => c.method === "POST" && c.url.endsWith("/s1/send"))).toBe(true)
    fireEvent.click(await screen.findByRole("button", { name: "Pause" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")?.body as Record<string, unknown>
    // RPT-1 review fix (SHOULD-FIX) R4: only the flag is sent.
    expect(put).toEqual({ paused: true })
  })

  it("shows Sending… with a spinner on the row until the send answers, then the result (RPT-1 QA fix Q1)", async () => {
    const calls = mockApi({ items: [sub] })
    const base = global.fetch
    let release: () => void = () => {}
    const gate = new Promise<void>((r) => { release = r })
    global.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
      if (String(input).endsWith("/send")) await gate
      return base(input, init)
    }) as unknown as typeof fetch
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "Send now" }))
    const row = await screen.findByRole("status")
    expect(row.textContent).toContain("Sending…")
    expect(row.querySelector("svg.animate-spin")).not.toBeNull()
    expect(screen.queryByRole("button", { name: "Send now" })).toBeNull()
    release()
    expect(await screen.findByText(`Not sent. ${EMAIL_NOT_SET_UP}`)).toBeDefined()
    expect(screen.queryByText("Sending…")).toBeNull()
    expect(calls.length).toBeGreaterThan(0)
  })
})
