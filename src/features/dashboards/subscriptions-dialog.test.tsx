import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it } from "bun:test"
import { chooseOption } from "@/components/patterns/select-field.testing"
import { formatDateTime } from "@/lib/format"
import type { ReportSubscription } from "@/services/contracts/reports"
import { ALWAYS_SENT_SENTENCE, AS_CREATOR_SENTENCE, EMAIL_NOT_SET_UP, SubscriptionsDialog } from "./subscriptions-dialog"

/**
 * Picks a choice of one of the console's searchable fields (a Base UI
 * combobox: the time zone, the chart data, the recipients). Typing filters
 * it; happy-dom's `change` does not open its list, so the arrow key does,
 * as it does for a keyboard user.
 */
async function pick(field: HTMLElement, text: string, option: string | RegExp) {
  fireEvent.change(field, { target: { value: text } })
  fireEvent.keyDown(field, { key: "ArrowDown" })
  fireEvent.click(await screen.findByRole("option", { name: option }))
}

async function newSubscription(extra: Partial<typeof props> = {}) {
  render(<SubscriptionsDialog {...props} {...extra} />)
  fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
}

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

function mockApi(opts: { items?: ReportSubscription[]; emailSetUp?: boolean; pdfSupported?: boolean; createStatus?: number; createBody?: unknown; sendStatus?: number; sendBody?: unknown } = {}) {
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
    if (url.endsWith("/send") && opts.sendStatus) return json(opts.sendBody ?? {}, opts.sendStatus)
    if (url.endsWith("/send")) return json({ id: "d", status: "failed", origin: "manual", recipientCount: 0, reason: EMAIL_NOT_SET_UP, startedAt: "", finishedAt: "" })
    return json({})
  }) as unknown as typeof fetch
  return calls
}

const props = {
  open: true, onOpenChange: () => {}, board: "b1", dashName: "Sales",
  charts: [{ id: "c1", title: "Visits", grouped: true }, { id: "c2", title: "Revenue", grouped: false }],
  readState: { filters: [{ column: "region", values: ["Bali"] }], params: null, grain: null },
  reportZone: "Asia/Jakarta",
}

beforeEach(() => { localStorage.clear() })
afterEach(() => { global.fetch = originalFetch; cleanup() })

describe("SubscriptionsDialog", () => {

  it("says next to Skip when empty that a dashboard with only single-value charts is always sent (RPT-1 QA fix Q3)", async () => {
    mockApi()
    const single = [{ id: "c2", title: "Revenue", grouped: false }]
    const { unmount } = render(<SubscriptionsDialog {...props} charts={single} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    // Owner QA 2026-11-16: one short line, the full reason behind its ⓘ.
    expect(await screen.findByText(/Always sent: every chart here has a single value/)).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: ALWAYS_SENT_SENTENCE }))
    expect(await screen.findByText(ALWAYS_SENT_SENTENCE)).toBeDefined()
    unmount()
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    await screen.findByRole("switch", { name: "Skip when empty" })
    expect(screen.getByText("Send nothing when every chart has no rows.")).toBeDefined()
    expect(screen.queryByRole("button", { name: ALWAYS_SENT_SENTENCE })).toBeNull()
  })
  it("lists a subscription with its schedule in words, recipients, attachments and last send", async () => {
    mockApi({ items: [sub] })
    render(<SubscriptionsDialog {...props} />)
    expect(await screen.findByText("Every Monday at 08:00 (Asia/Jakarta)")).toBeDefined()
    expect(screen.getByText(/PDF and 1 chart file · to Colleague, press@partner.example/)).toBeDefined()
    expect(screen.getByText("Failed. The PDF took too long to make.")).toBeDefined()
  })

  it("shows Not sent with the time of the last attempt on a subscription while email is not set up, instead of its last status", async () => {
    const lastAttemptAt = "2026-11-12T04:15:00Z"
    const withoutEmail: ReportSubscription = {
      ...sub,
      consecutiveFailures: 0,
      notSent: { cause: "email_not_set_up", lastAttemptAt },
      lastDelivery: { id: "d2", startedAt: lastAttemptAt, finishedAt: lastAttemptAt, status: "failed", origin: "schedule", recipientCount: 2, reason: EMAIL_NOT_SET_UP },
    }
    mockApi({ items: [withoutEmail], emailSetUp: false })
    render(<SubscriptionsDialog {...props} />)
    expect(await screen.findByText(`Not sent: e-mail is not set up on this installation. Last attempt: ${formatDateTime(lastAttemptAt)}.`)).toBeDefined()
    expect(screen.queryByText(`Failed. ${EMAIL_NOT_SET_UP}`)).toBeNull()
  })

  it("keeps the Paused line under Not sent", async () => {
    const paused: ReportSubscription = {
      ...sub,
      paused: true,
      pausedReason: "Paused after 5 failed sends in a row.",
      notSent: { cause: "email_not_set_up", lastAttemptAt: null },
    }
    mockApi({ items: [paused], emailSetUp: false })
    render(<SubscriptionsDialog {...props} />)
    expect(await screen.findByText("Not sent: e-mail is not set up on this installation.")).toBeDefined()
    expect(screen.getByText("Paused. Paused after 5 failed sends in a row.")).toBeDefined()
  })

  it("shows the last status as before when the list says nothing of the kind", async () => {
    mockApi({ items: [{ ...sub, notSent: null }], emailSetUp: false })
    render(<SubscriptionsDialog {...props} />)
    expect(await screen.findByText("Failed. The PDF took too long to make.")).toBeDefined()
    expect(screen.queryByText(/Not sent: e-mail is not set up/)).toBeNull()
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
    // Feature decision 1, said above the recipient field in one line; the whole sentence names its ⓘ.
    const asYou = screen.getByText("Everyone sees the data as you see it")
    expect(screen.getByRole("button", { name: AS_CREATOR_SENTENCE })).toBeDefined()
    const field = screen.getByRole("combobox", { name: "Recipients" })
    expect(asYou.compareDocumentPosition(field) & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy()
    expect(screen.getByText("Every Monday at 08:00 (Asia/Jakarta)")).toBeDefined()
    chooseOption(screen.getByRole("combobox", { name: "Repeat" }), "Every month")
    chooseOption(screen.getByRole("combobox", { name: "Day of the month" }), "The last day")
    expect(screen.getByText("On the last day of every month at 08:00 (Asia/Jakarta)")).toBeDefined()
  })

  it("sends the schedule, attachments, the filters on screen and the recipients", async () => {
    const calls = mockApi()
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    await pick(screen.getByRole("combobox", { name: "Chart data" }), "vis", "Visits")
    fireEvent.click(screen.getByRole("radio", { name: "CSV" }))
    const recipients = screen.getByRole("combobox", { name: "Recipients" })
    fireEvent.change(recipients, { target: { value: "dew" } })
    await waitFor(() => expect(calls.some((c) => c.url.startsWith("/api/reports/recipients"))).toBe(true))
    fireEvent.keyDown(recipients, { key: "ArrowDown" })
    fireEvent.click(await screen.findByRole("option", { name: /Dewi/ }))
    await pick(recipients, "Press@Partner.Example", /Add press@partner\.example/)
    expect(screen.getByRole("button", { name: "Remove Dewi" })).toBeDefined()
    expect(screen.getByRole("button", { name: "Remove press@partner.example" })).toBeDefined()
    fireEvent.click(screen.getByRole("switch", { name: "Skip when empty" }))
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
    await pick(screen.getByRole("combobox", { name: "Recipients" }), "x@nowhere.example", /Add x@nowhere\.example/)
    fireEvent.click(screen.getByRole("button", { name: "Subscribe" }))
    expect((await screen.findByRole("alert")).textContent).toBe("An outside address is on a domain an administrator has not allowed for reports.")
    expect(screen.getByRole("button", { name: "Subscribe" })).toBeDefined()
  })

  it("disables the PDF choice and says why when the deployment has no PDF service", async () => {
    mockApi({ pdfSupported: false })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    const pdf = screen.getByRole("switch", { name: "The dashboard as a PDF" })
    expect(pdf.getAttribute("aria-disabled") === "true" || pdf.hasAttribute("data-disabled")).toBe(true)
    expect(pdf.getAttribute("aria-checked")).toBe("false")
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

  it("asks before deleting, names the subscription, and deletes only on the confirm (BI-35)", async () => {
    const calls = mockApi({ items: [sub] })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "Delete" }))
    expect(await screen.findByText("Delete this subscription?")).toBeDefined()
    expect(screen.getByText(/Every Monday at 08:00 \(Asia\/Jakarta\) on Sales will stop/)).toBeDefined()
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: "Cancel" }))
    await waitFor(() => expect(screen.queryByText("Delete this subscription?")).toBeNull())
    expect(calls.some((c) => c.method === "DELETE")).toBe(false)
    fireEvent.click(screen.getByRole("button", { name: "Delete" }))
    fireEvent.click(await screen.findByRole("button", { name: "Delete subscription" }))
    await waitFor(() => expect(calls.some((c) => c.method === "DELETE" && c.url.endsWith("/s1"))).toBe(true))
  })

  it("shows the sentence of a send that was pressed again too soon (BI-35)", async () => {
    mockApi({ items: [sub], sendStatus: 429, sendBody: { error: "This report was just sent. Wait a little before sending it again." } })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "Send now" }))
    expect((await screen.findByRole("alert")).textContent).toBe("This report was just sent. Wait a little before sending it again.")
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

describe("the subscription time zone picker (WEB-8 D8, owner QA 2026-11-16)", () => {
  it("is a searchable list of the names listTimeZones names, including UTC and the report's own zone", async () => {
    mockApi()
    await newSubscription()
    const field = screen.getByRole("combobox", { name: "Time zone" }) as HTMLInputElement
    expect(field.value).toBe("Asia/Jakarta")
    expect(document.querySelector("datalist")).toBeNull()
    fireEvent.keyDown(field, { key: "ArrowDown" })
    const names = (await screen.findAllByRole("option")).map((o) => o.textContent ?? "")
    for (const zone of ["UTC", "Asia/Jakarta", "Europe/Berlin"]) expect(names.some((n) => n.startsWith(zone))).toBe(true)
    // Each name shows its offset now.
    expect(names.find((n) => n.startsWith("Asia/Jakarta"))).toContain("UTC+7")
  })

  it("narrows the list as the person types, with the words of a name, and says so when nothing matches", async () => {
    mockApi()
    await newSubscription()
    const field = screen.getByRole("combobox", { name: "Time zone" })
    fireEvent.change(field, { target: { value: "new york" } })
    fireEvent.keyDown(field, { key: "ArrowDown" })
    await waitFor(() => expect(screen.getAllByRole("option").map((o) => o.textContent)).toEqual([expect.stringContaining("America/New_York")]))
    fireEvent.change(field, { target: { value: "zzzz" } })
    expect(await screen.findByText("No time zone matches.")).toBeDefined()
  })

  it("offers the browser's own zone first, marked as the person's", async () => {
    mockApi()
    await newSubscription({ reportZone: "UTC" })
    const own = Intl.DateTimeFormat().resolvedOptions().timeZone
    fireEvent.keyDown(screen.getByRole("combobox", { name: "Time zone" }), { key: "ArrowDown" })
    const first = (await screen.findAllByRole("option"))[0]
    expect(first.textContent).toContain(own)
    expect(first.textContent).toContain("Your zone")
  })

  it("sends the zone picked from the list and says it in the schedule line", async () => {
    const calls = mockApi()
    await newSubscription()
    await pick(screen.getByRole("combobox", { name: "Time zone" }), "berl", /^Europe\/Berlin/)
    expect(await screen.findByText("Every Monday at 08:00 (Europe/Berlin)")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Subscribe" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const post = calls.find((c) => c.method === "POST")?.body as Record<string, unknown>
    expect(post.timeZone).toBe("Europe/Berlin")
  })

  it("keeps the zone an existing subscription was saved with", async () => {
    mockApi({ items: [{ ...sub, timeZone: "America/Sao_Paulo" }] })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }))
    expect((screen.getByRole("combobox", { name: "Time zone" }) as HTMLInputElement).value).toBe("America/Sao_Paulo")
  })
})

describe("the subscription recipient field (owner QA 2026-11-16)", () => {
  it("takes a person and an outside address in one field, as chips, and sends both", async () => {
    const calls = mockApi()
    await newSubscription()
    const field = screen.getByRole("combobox", { name: "Recipients" })
    fireEvent.change(field, { target: { value: "dew" } })
    fireEvent.keyDown(field, { key: "ArrowDown" })
    const dewi = await screen.findByRole("option", { name: /Dewi/ })
    expect(dewi.textContent).toContain("dewi@company.example")
    fireEvent.click(dewi)
    // A full address is offered as an outside one; a comma takes it too.
    fireEvent.change(field, { target: { value: "ops@partner.example" } })
    fireEvent.keyDown(field, { key: "," })
    expect(screen.getByRole("button", { name: "Remove ops@partner.example" })).toBeDefined()
    expect((field as HTMLInputElement).value).toBe("")
    fireEvent.click(screen.getByRole("button", { name: "Subscribe" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const post = calls.find((c) => c.method === "POST")?.body as Record<string, unknown>
    expect(post.recipients).toEqual([{ userId: "u9" }, { email: "ops@partner.example" }])
  })

  it("does not offer text that is not a full address, and says how to add one", async () => {
    mockApi()
    await newSubscription()
    const base = global.fetch
    global.fetch = (async (input: RequestInfo | URL, init?: RequestInit) =>
      String(input).startsWith("/api/reports/recipients") ? json({ items: [] }) : base(input, init)) as unknown as typeof fetch
    const field = screen.getByRole("combobox", { name: "Recipients" })
    fireEvent.change(field, { target: { value: "press@partner" } })
    fireEvent.keyDown(field, { key: "ArrowDown" })
    expect(await screen.findByText("No one found. Type a full email address to add it.")).toBeDefined()
    expect(screen.queryAllByRole("option")).toEqual([])
    fireEvent.keyDown(field, { key: "," })
    expect(screen.queryByRole("button", { name: /Remove press/ })).toBeNull()
  })

  it("offers a typed address as the person who has it, not as an outside address", async () => {
    mockApi()
    await newSubscription()
    const field = screen.getByRole("combobox", { name: "Recipients" })
    fireEvent.change(field, { target: { value: "dewi@company.example" } })
    fireEvent.keyDown(field, { key: "ArrowDown" })
    expect(await screen.findByRole("option", { name: /Dewi/ })).toBeDefined()
    expect(screen.queryByRole("option", { name: /Add dewi@company\.example/ })).toBeNull()
  })

  it("removes a chip with its own button, and opens an existing subscription with its recipients", async () => {
    const calls = mockApi({ items: [sub] })
    render(<SubscriptionsDialog {...props} />)
    fireEvent.click(await screen.findByRole("button", { name: "Edit" }))
    expect(screen.getByRole("button", { name: "Remove Colleague" })).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Remove press@partner.example" }))
    expect(screen.queryByRole("button", { name: "Remove press@partner.example" })).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Save changes" }))
    await waitFor(() => expect(calls.some((c) => c.method === "PUT")).toBe(true))
    const put = calls.find((c) => c.method === "PUT")?.body as Record<string, unknown>
    expect(put.recipients).toEqual([{ userId: "u2" }])
  })
})

describe("the subscription form's shape (owner QA 2026-11-16)", () => {
  it("has one way out of the form: the back arrow, which asks before dropping changes", async () => {
    mockApi()
    await newSubscription()
    expect(screen.queryByRole("button", { name: "Cancel" })).toBeNull()
    // Unchanged: back goes straight to the list.
    fireEvent.click(screen.getByRole("button", { name: "Back to the subscriptions" }))
    fireEvent.click(await screen.findByRole("button", { name: "New subscription" }))
    fireEvent.click(screen.getByRole("switch", { name: "Skip when empty" }))
    fireEvent.click(screen.getByRole("button", { name: "Back to the subscriptions" }))
    expect(await screen.findByText("Discard your changes?")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Discard changes" }))
    expect(await screen.findByRole("button", { name: "New subscription" })).toBeDefined()
  })

  it("attaches chart data from one field, one quiet row per chart with its format, and leaves a chart out again", async () => {
    const calls = mockApi()
    await newSubscription()
    expect(screen.queryByRole("combobox", { name: /File for/ })).toBeNull()
    expect(screen.queryByRole("list", { name: "Attached chart data" })).toBeNull()
    await pick(screen.getByRole("combobox", { name: "Chart data" }), "rev", "Revenue")
    const formats = screen.getByRole("radiogroup", { name: "Format for Revenue" })
    expect(within(formats).getByRole("radio", { name: "Excel" }).getAttribute("aria-checked")).toBe("true")
    expect(screen.getByRole("combobox", { name: "Values in the files" })).toBeDefined()
    await pick(screen.getByRole("combobox", { name: "Chart data" }), "vis", "Visits")
    fireEvent.click(screen.getByRole("button", { name: "Leave out Revenue" }))
    expect(screen.queryByRole("radiogroup", { name: "Format for Revenue" })).toBeNull()
    fireEvent.click(screen.getByRole("button", { name: "Subscribe" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    const post = calls.find((c) => c.method === "POST")?.body as { attachments: { charts: unknown[] } }
    expect(post.attachments.charts).toEqual([{ chart: "c1", format: "xlsx", values: "formatted" }])
  })

  it("uses quiet buttons for the filters, and Clear sends none", async () => {
    const calls = mockApi()
    await newSubscription()
    expect(screen.getByText("1 filter will be applied to this report.")).toBeDefined()
    fireEvent.click(screen.getByRole("button", { name: "Clear" }))
    expect(screen.getByText("No filters: the dashboard's own.")).toBeDefined()
    expect((screen.getByRole("button", { name: "Clear" }) as HTMLButtonElement).disabled).toBe(true)
    fireEvent.click(screen.getByRole("button", { name: "Subscribe" }))
    await waitFor(() => expect(calls.some((c) => c.method === "POST")).toBe(true))
    expect((calls.find((c) => c.method === "POST")?.body as Record<string, unknown>).filters).toBeNull()
  })

  it("labels the When fields above their controls and gives every control the full width", async () => {
    mockApi()
    await newSubscription()
    for (const name of ["Repeat", "Day"]) expect(screen.getByRole("combobox", { name }).className).toContain("w-full")
    expect(screen.getByLabelText("Time")).toBeDefined()
    expect(screen.getByRole("combobox", { name: "Paper" }).className).toContain("w-full")
  })
})
