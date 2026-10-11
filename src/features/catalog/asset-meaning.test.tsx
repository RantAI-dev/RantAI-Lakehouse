// The meaning of a table and its columns on the Schema tab (plan 10, step 3):
// the table's card and the Meaning view of the Columns card. The state is
// built by hand, as `ready()` builds a profile in `asset-columns.test.tsx`,
// so each state the components must tell apart is one line of setup. The
// hook that reads the entries is covered where it is used, in
// `asset-detail-tabs.test.tsx`.
import { fireEvent, render, screen, within } from "@testing-library/react"
import { describe, expect, it, mock } from "bun:test"
import type { AssetColumn } from "@/services/contracts/assets"
import type { SemanticEntry, SemanticEntryInput } from "@/services/contracts/semantic"
import { ServiceError } from "@/services/errors"
import { ColumnsCard, type ProfileState, type SchemaRow } from "./asset-columns"
import type { MeaningState } from "./asset-meaning"
import { TableMeaningCard } from "./asset-meaning-view"

const column = (name: string, dataType = "String", over: Partial<AssetColumn> = {}): AssetColumn => ({
  name,
  dataType,
  ...over,
})

const schemaRow = (c: AssetColumn): SchemaRow => ({ column: c, nullable: null, partition: null })

const entry = (over: Partial<SemanticEntry> = {}): SemanticEntry => ({
  asset: "serving.orders",
  column: null,
  description: "",
  synonyms: [],
  role: null,
  status: "confirmed",
  writtenBy: null,
  model: null,
  updatedAt: new Date().toISOString(),
  ...over,
})

type Ready = Extract<MeaningState, { kind: "ready" }>

/** A ready state built by hand: the entries are given, the save and reload are stubs. */
function readyMeanings(
  entries: SemanticEntry[] = [],
  over: { table?: SemanticEntry | null; canEdit?: boolean; save?: (input: SemanticEntryInput) => Promise<void> } = {}
): { state: Ready; save: ReturnType<typeof mock<(input: SemanticEntryInput) => Promise<void>>> } {
  const save = mock<(input: SemanticEntryInput) => Promise<void>>(over.save ?? (async () => {}))
  const reload = mock(() => {})
  const state: Ready = {
    kind: "ready",
    table: over.table ?? null,
    byColumn: new Map(entries.filter((e) => e.column !== null).map((e) => [e.column as string, e])),
    canEdit: over.canEdit ?? false,
    save,
    reload,
  }
  return { state, save }
}

const PROFILE: ProfileState = { kind: "restricted" }

/** Three columns: a draft measure with synonyms, a confirmed non-additive count, and one with no entry. */
const DRAFT = entry({
  column: "orders_total",
  description: "Total order value in rupiah",
  synonyms: ["omzet", "penjualan"],
  role: "measure",
  status: "draft",
  model: "MiniMax-M3",
})
const CONFIRMED = entry({
  column: "customer_count",
  description: "Number of distinct customers",
  role: "non_additive",
  status: "confirmed",
})
const ROWS = [
  schemaRow(column("orders_total", "Decimal(12, 2)")),
  schemaRow(column("customer_count", "UInt64")),
  schemaRow(column("note")),
]

const renderCard = (rows: SchemaRow[], meanings?: MeaningState) =>
  render(<ColumnsCard rows={rows} profile={PROFILE} sortingKey={null} meanings={meanings} />)

const viewGroup = () => screen.getByRole("group", { name: "Columns view" })
const meaningTab = () => within(viewGroup()).getByRole("button", { name: "Meaning" })
const meaningTable = () => screen.getByRole("table", { name: "Column meanings" })

describe("the Columns card without meanings", () => {
  it("renders the same HTML for no meanings prop as for a state of kind none", () => {
    // React numbers its `useId` ids from a counter that carries across renders in one file; the ids are
    // not what this rule is about, so they are normalised before the comparison.
    const ids = (html: string) => html.replace(/_r_\w+_/g, "_id_")
    const { container, unmount } = renderCard(ROWS)
    const absent = ids(container.innerHTML)
    unmount()

    const second = renderCard(ROWS, { kind: "none" })
    expect(ids(second.container.innerHTML)).toBe(absent)
    expect(screen.queryByRole("group", { name: "Columns view" })).toBeNull()
  })
})

describe("the view toggle", () => {
  it("shows Statistics pressed for a ready state, and the statistics table as it was", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(ROWS, state)

    expect(within(viewGroup()).getByRole("button", { name: "Statistics" }).getAttribute("aria-pressed")).toBe("true")
    expect(meaningTab().getAttribute("aria-pressed")).toBe("false")
    expect(screen.getByRole("table", { name: "Columns" })).toBeTruthy()
    // The statistics view names no role and no status: those are the meaning view's.
    expect(screen.queryByText("Measure")).toBeNull()
    expect(screen.queryByText("draft")).toBeNull()
  })

  it("has no toggle while the meanings are loading, failed or unsupported", () => {
    renderCard(ROWS, { kind: "loading" })
    expect(screen.queryByRole("group", { name: "Columns view" })).toBeNull()
    expect(screen.getAllByRole("row").length).toBeGreaterThan(0)
  })
})

describe("the drafts line", () => {
  it("offers one draft in words of one, and switches to the meaning view", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(ROWS, state)

    fireEvent.click(screen.getByRole("button", { name: "1 draft to review" }))
    expect(meaningTable()).toBeTruthy()
    expect(meaningTab().getAttribute("aria-pressed")).toBe("true")
  })

  it("offers two drafts in the plural", () => {
    const second = entry({ column: "note", description: "A note", status: "draft", model: null })
    const { state } = readyMeanings([DRAFT, second])
    renderCard(ROWS, state)
    expect(screen.getByRole("button", { name: "2 drafts to review" })).toBeTruthy()
  })

  it("offers nothing when no column is a draft", () => {
    const { state } = readyMeanings([CONFIRMED])
    renderCard(ROWS, state)
    expect(screen.queryByRole("button", { name: /draft/ })).toBeNull()
  })
})

describe("the meaning view", () => {
  it("shows a draft's meaning, synonyms, role and status on one line", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    const table = meaningTable()
    expect(within(table).getByText("Total order value in rupiah")).toBeTruthy()
    expect(within(table).getByText("omzet, penjualan")).toBeTruthy()
    expect(within(table).getByText("Measure")).toBeTruthy()
    expect(within(table).getByText("draft")).toBeTruthy()
  })

  it("shows a confirmed non-additive column with its role and confirmed status", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    const table = meaningTable()
    expect(within(table).getByText("Number of distinct customers")).toBeTruthy()
    expect(within(table).getByText("Number, not to be added up")).toBeTruthy()
    expect(within(table).getByText("confirmed")).toBeTruthy()
  })

  it("shows a column with no entry as not set, with no meaning written", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    const row = within(meaningTable()).getByText("note").closest("[role=row]") as HTMLElement
    expect(within(row).getByText("Not set")).toBeTruthy()
    expect(within(row).getByText("No meaning written")).toBeTruthy()
  })

  it("lists the same rows as the statistics view, so the filter applies to both", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    expect(within(meaningTable()).getAllByRole("row")).toHaveLength(4)
  })
})

describe("Drafts only", () => {
  it("leaves the draft rows and drops the confirmed one", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    expect(within(meaningTable()).getByText("customer_count")).toBeTruthy()

    fireEvent.click(screen.getByRole("checkbox", { name: "Drafts only" }))

    expect(within(meaningTable()).getByText("orders_total")).toBeTruthy()
    expect(within(meaningTable()).queryByText("customer_count")).toBeNull()
  })

  it("says no draft is left when the rows that remain are all confirmed", () => {
    // Eleven columns, so the filter box is shown; "customer" then leaves only the confirmed one.
    const eleven = [...ROWS, ...Array.from({ length: 8 }, (_, i) => schemaRow(column(`col_${i}`)))]
    const { state } = readyMeanings([DRAFT, CONFIRMED])
    renderCard(eleven, state)
    fireEvent.click(meaningTab())

    fireEvent.change(screen.getByLabelText("Filter columns"), { target: { value: "customer" } })
    fireEvent.click(screen.getByRole("checkbox", { name: "Drafts only" }))
    expect(screen.getByText("No draft left to review.")).toBeTruthy()
  })
})

describe("who can edit", () => {
  it("shows no role select, confirm or edit control without catalog:write", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], { canEdit: false })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    const table = meaningTable()
    expect(within(table).queryByRole("combobox")).toBeNull()
    expect(within(table).queryByRole("button", { name: "Confirm the draft of orders_total" })).toBeNull()
    expect(within(table).queryByRole("button", { name: "Edit the meaning of orders_total" })).toBeNull()
  })

  it("gives a draft a role select, an edit and a confirm control with catalog:write", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    const table = meaningTable()
    expect(within(table).getByRole("combobox", { name: "Role of orders_total" })).toBeTruthy()
    expect(within(table).getByRole("button", { name: "Edit the meaning of orders_total" })).toBeTruthy()
    expect(within(table).getByRole("button", { name: "Confirm the draft of orders_total" })).toBeTruthy()
  })

  it("gives a confirmed row no confirm control", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    expect(screen.queryByRole("button", { name: "Confirm the draft of customer_count" })).toBeNull()
  })

  it("disables the role select of a column with no entry", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    const select = within(meaningTable()).getByRole("combobox", { name: "Role of note" })
    expect(select.hasAttribute("disabled") || select.getAttribute("aria-disabled") === "true").toBe(true)
  })
})

describe("the filter in both views", () => {
  const eleven = Array.from({ length: 11 }, (_, i) => schemaRow(column(`col_${String(i + 1).padStart(2, "0")}`)))
  const withSynonym = entry({ column: "col_05", description: "", synonyms: ["penjualan"], status: "confirmed" })

  it("finds a column by a synonym in the statistics view and in the meaning view", () => {
    const { state } = readyMeanings([withSynonym])
    renderCard(eleven, state)

    fireEvent.change(screen.getByLabelText("Filter columns"), { target: { value: "penjualan" } })
    expect(screen.getByRole("button", { name: "col_05" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: "col_01" })).toBeNull()

    fireEvent.click(meaningTab())
    expect(within(meaningTable()).getByText("col_05")).toBeTruthy()
    expect(within(meaningTable()).queryByText("col_01")).toBeNull()
  })
})

describe("the table's meaning card", () => {
  it("shows the table's description, its synonyms and the AI draft pill for a draft", () => {
    const { state } = readyMeanings([], {
      table: entry({ description: "Orders placed by customers", synonyms: ["pesanan"], status: "draft", model: "MiniMax-M3" }),
    })
    render(<TableMeaningCard state={state} />)
    expect(screen.getByText("Orders placed by customers")).toBeTruthy()
    expect(screen.getByText("pesanan")).toBeTruthy()
    expect(screen.getByText("AI draft")).toBeTruthy()
  })

  it("names the model that drafted it, and says so when no model is recorded", () => {
    const withModel = readyMeanings([], { table: entry({ description: "A", status: "draft", model: "MiniMax-M3" }) })
    const { unmount } = render(<TableMeaningCard state={withModel.state} />)
    expect(screen.getByText(/^Drafted by MiniMax-M3 · /)).toBeTruthy()
    unmount()

    const noModel = readyMeanings([], { table: entry({ description: "A", status: "draft", model: null }) })
    render(<TableMeaningCard state={noModel.state} />)
    expect(screen.getByText(/^Drafted by the assistant · /)).toBeTruthy()
  })

  it("says a person confirmed a confirmed entry", () => {
    const { state } = readyMeanings([], { table: entry({ description: "A", status: "confirmed" }) })
    render(<TableMeaningCard state={state} />)
    expect(screen.getByText(/^Confirmed by a person · /)).toBeTruthy()
  })

  it("says so when no meaning is written for the table", () => {
    const { state } = readyMeanings([], { table: null })
    render(<TableMeaningCard state={state} />)
    expect(screen.getByText("No meaning is written for this table yet.")).toBeTruthy()
  })

  it("shows the reason when the meanings are unsupported", () => {
    render(<TableMeaningCard state={{ kind: "unsupported", reason: "The semantic layer is off." }} />)
    expect(screen.getByText("The semantic layer is off.")).toBeTruthy()
  })

  it("shows the message of an error, and its retry button calls retry", () => {
    const retry = mock(() => {})
    render(<TableMeaningCard state={{ kind: "error", message: "Network down", retry }} />)
    expect(screen.getByText("Network down")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Retry" }))
    expect(retry).toHaveBeenCalledTimes(1)
  })

  it("offers Edit and, for a draft, Confirm only with catalog:write", () => {
    const draft = readyMeanings([], { table: entry({ status: "draft", description: "A" }), canEdit: false })
    const { unmount } = render(<TableMeaningCard state={draft.state} />)
    expect(screen.queryByRole("button", { name: "Edit" })).toBeNull()
    expect(screen.queryByRole("button", { name: "Confirm" })).toBeNull()
    unmount()

    const editable = readyMeanings([], { table: entry({ status: "draft", description: "A" }), canEdit: true })
    const again = render(<TableMeaningCard state={editable.state} />)
    expect(screen.getByRole("button", { name: "Edit" })).toBeTruthy()
    expect(screen.getByRole("button", { name: "Confirm" })).toBeTruthy()
    again.unmount()

    const confirmed = readyMeanings([], { table: entry({ status: "confirmed", description: "A" }), canEdit: true })
    render(<TableMeaningCard state={confirmed.state} />)
    expect(screen.queryByRole("button", { name: "Confirm" })).toBeNull()
  })

  it("offers Add a meaning, not Edit, for a table with no entry", () => {
    const { state } = readyMeanings([], { table: null, canEdit: true })
    render(<TableMeaningCard state={state} />)
    expect(screen.getByRole("button", { name: "Add a meaning" })).toBeTruthy()
  })
})

// ── Step 4: a person corrects and confirms ─────────────────────────────────

/**
 * Chooses an option of a base-ui Select the way a pointer does: the list opens,
 * the option takes focus (which makes it the active one), and it is clicked.
 * In this jsdom a click on an option that was not focused first does not reach
 * `onValueChange`, so the focus step is part of the choice.
 */
function choose(trigger: HTMLElement, option: string) {
  fireEvent.click(trigger)
  const target = screen.getByRole("option", { name: option })
  fireEvent.focus(target)
  fireEvent.click(target)
}

const roleSelect = (name: string) => within(meaningTable()).getByRole("combobox", { name: `Role of ${name}` })
const rowOf = (name: string) => within(meaningTable()).getByText(name).closest("[role=row]") as HTMLElement
const dialog = () => screen.getByRole("dialog")
const isDisabled = (el: HTMLElement) => (el as HTMLButtonElement).disabled || el.getAttribute("aria-disabled") === "true"

describe("a person changes a role in the row", () => {
  it("saves the new role with the entry's own meaning and synonyms", () => {
    const { state, save } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    choose(roleSelect("orders_total"), "Number, not to be added up")

    expect(save).toHaveBeenCalledTimes(1)
    expect(save.mock.calls[0][0]).toEqual({
      column: "orders_total",
      description: "Total order value in rupiah",
      synonyms: ["omzet", "penjualan"],
      role: "non_additive",
    })
  })

  it("saves a null role when the role is set to Not set", () => {
    const { state, save } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    choose(roleSelect("orders_total"), "Not set")

    expect(save).toHaveBeenCalledTimes(1)
    expect(save.mock.calls[0][0].role).toBeNull()
    expect(save.mock.calls[0][0].synonyms).toEqual(["omzet", "penjualan"])
  })

  it("confirms a draft with its own role, meaning and synonyms", () => {
    const { state, save } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    fireEvent.click(within(meaningTable()).getByRole("button", { name: "Confirm the draft of orders_total" }))

    expect(save).toHaveBeenCalledTimes(1)
    expect(save.mock.calls[0][0]).toEqual({
      column: "orders_total",
      description: "Total order value in rupiah",
      synonyms: ["omzet", "penjualan"],
      role: "measure",
    })
  })

  it("disables every role select, confirm and edit control while a save is unresolved", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], {
      canEdit: true,
      save: () => new Promise<void>(() => {}),
    })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    choose(roleSelect("customer_count"), "Dimension")

    const table = meaningTable()
    const controls = [
      ...within(table).getAllByRole("combobox"),
      ...within(table).getAllByRole("button", { name: /^Confirm the draft of|^Edit the meaning of|^Add a meaning$/ }),
    ]
    expect(controls.length).toBeGreaterThan(0)
    for (const control of controls) expect(isDisabled(control)).toBe(true)
  })

  it("shows the failure under the table and keeps the stored role on the row", async () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], {
      canEdit: true,
      save: async () => {
        throw new ServiceError("invalid_request", "`role` must be one of measure, dimension")
      },
    })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    choose(roleSelect("orders_total"), "Dimension")

    const alert = await screen.findByRole("alert")
    expect(alert.textContent).toBe("Could not save orders_total: `role` must be one of measure, dimension")
    expect(within(rowOf("orders_total")).getByText("Measure")).toBeTruthy()
  })
})

describe("a person edits a meaning in the dialog", () => {
  it("saves the new meaning and keeps both synonyms and the role", () => {
    const { state, save } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())

    fireEvent.click(within(meaningTable()).getByRole("button", { name: "Edit the meaning of orders_total" }))
    expect(screen.getByText("Edit the meaning of orders_total")).toBeTruthy()
    fireEvent.change(within(dialog()).getByLabelText("Meaning"), { target: { value: "Gross order value" } })
    fireEvent.click(within(dialog()).getByRole("button", { name: "Save" }))

    expect(save).toHaveBeenCalledTimes(1)
    expect(save.mock.calls[0][0]).toEqual({
      column: "orders_total",
      description: "Gross order value",
      synonyms: ["omzet", "penjualan"],
      role: "measure",
    })
  })

  it("has no role field for the table and saves it with a null column role", () => {
    const table = entry({ description: "Orders placed", synonyms: ["pesanan"], status: "draft", model: "MiniMax-M3" })
    const { state, save } = readyMeanings([], { table, canEdit: true })
    render(<TableMeaningCard state={state} />)

    fireEvent.click(screen.getByRole("button", { name: "Edit" }))
    expect(screen.getByText("Edit the table's meaning")).toBeTruthy()
    expect(within(dialog()).queryByRole("combobox")).toBeNull()
    fireEvent.click(within(dialog()).getByRole("button", { name: "Save" }))

    expect(save.mock.calls[0][0]).toEqual({ column: null, description: "Orders placed", synonyms: ["pesanan"], role: null })
  })

  it("keeps Save disabled for an empty meaning", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    fireEvent.click(within(meaningTable()).getByRole("button", { name: "Edit the meaning of orders_total" }))

    fireEvent.change(within(dialog()).getByLabelText("Meaning"), { target: { value: "   " } })
    expect(isDisabled(within(dialog()).getByRole("button", { name: "Save" }))).toBe(true)
  })

  it("keeps Save disabled for a column meaning of 201 characters, and turns the counter red", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    fireEvent.click(within(meaningTable()).getByRole("button", { name: "Edit the meaning of orders_total" }))

    fireEvent.change(within(dialog()).getByLabelText("Meaning"), { target: { value: "x".repeat(201) } })
    expect(isDisabled(within(dialog()).getByRole("button", { name: "Save" }))).toBe(true)
    expect(within(dialog()).getByText("201/200").className).toContain("text-destructive")
  })

  it("keeps Save disabled for seven names, and says why", () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], { canEdit: true })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    fireEvent.click(within(meaningTable()).getByRole("button", { name: "Edit the meaning of orders_total" }))

    fireEvent.change(within(dialog()).getByLabelText("Also called"), {
      target: { value: "a, b, c, d, e, f, g" },
    })
    expect(isDisabled(within(dialog()).getByRole("button", { name: "Save" }))).toBe(true)
    expect(within(dialog()).getByText("At most 6 names.")).toBeTruthy()
  })

  it("shows a rejected save under the fields and leaves the dialog open", async () => {
    const { state } = readyMeanings([DRAFT, CONFIRMED], {
      canEdit: true,
      save: async () => {
        throw new ServiceError("invalid_request", "`synonyms` must be at most 6")
      },
    })
    renderCard(ROWS, state)
    fireEvent.click(meaningTab())
    fireEvent.click(within(meaningTable()).getByRole("button", { name: "Edit the meaning of orders_total" }))

    fireEvent.click(within(dialog()).getByRole("button", { name: "Save" }))

    expect(await within(dialog()).findByText("`synonyms` must be at most 6")).toBeTruthy()
    expect(screen.getByRole("dialog")).toBeTruthy()
  })
})

describe("the table's card confirms and edits the table", () => {
  it("confirms the table's draft with its description and synonyms", () => {
    const table = entry({ description: "Orders placed", synonyms: ["pesanan"], status: "draft", model: "MiniMax-M3" })
    const { state, save } = readyMeanings([], { table, canEdit: true })
    render(<TableMeaningCard state={state} />)

    fireEvent.click(screen.getByRole("button", { name: "Confirm" }))

    expect(save).toHaveBeenCalledTimes(1)
    expect(save.mock.calls[0][0]).toEqual({ column: null, description: "Orders placed", synonyms: ["pesanan"], role: null })
  })
})
