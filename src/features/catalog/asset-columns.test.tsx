// The Schema tab's column explorer (plan
// `docs/superpowers/plans/2026-10-05-schema-tab-column-explorer.md`): a
// compact line per column with a bar of its most frequent values, and the
// column opened in place on demand. These tests drive `ColumnsCard` with a
// profile built by hand so each state the card must tell apart (ready,
// loading, restricted, unsupported, failed) is one line of setup; the tab
// that feeds it is covered in `asset-detail-tabs.test.tsx`.
//
// happy-dom applies no CSS, so what the container queries shed at a narrow
// width is not visible here: every cell of a row is in the DOM. What can be
// asserted is the structure that makes the narrow layout possible (no
// `<table>`, hence no sideways scroller) and what each cell says.
import { fireEvent, render, screen, within } from "@testing-library/react"
import { describe, expect, it } from "bun:test"
import type { AssetColumn, AssetProfile, ColumnProfile } from "@/services/contracts/assets"
import { ColumnsCard, type ProfileState, type SchemaRow } from "./asset-columns"

const column = (name: string, dataType = "String", over: Partial<AssetColumn> = {}): AssetColumn => ({
  name,
  dataType,
  ...over,
})

const schemaRow = (c: AssetColumn, nullable: boolean | null = null, partition: string | null = null): SchemaRow => ({
  column: c,
  nullable,
  partition,
})

const stat = (name: string, over: Partial<ColumnProfile> = {}): ColumnProfile => ({
  name,
  dataType: "String",
  profiled: true,
  nullCount: 0,
  nullFraction: 0,
  distinctCount: 3,
  topValues: [],
  ...over,
})

function ready(
  columns: ColumnProfile[],
  over: Partial<Extract<AssetProfile, { supported: true }>> = {}
): ProfileState {
  return {
    kind: "ready",
    profile: {
      supported: true,
      source: "silver.orders",
      sourceKind: "clickhouse",
      rowsProfiled: 2000,
      rowLimit: 100000,
      sampled: false,
      columnsCapped: false,
      columns,
      ...over,
    },
    byName: new Map(columns.map((c) => [c.name, c])),
  }
}

/** 60% Jakarta, 25% Bandung, 5% the empty string, 5% null, and 5% left over. */
const CITY = stat("city", {
  nullCount: 100,
  nullFraction: 0.05,
  distinctCount: 4,
  topValues: [
    { value: "Jakarta", count: 1200 },
    { value: "Bandung", count: 500 },
    { value: "", count: 100 },
  ],
})

const AMOUNT = stat("amount", {
  dataType: "Decimal(12, 2)",
  nullCount: 500,
  nullFraction: 0.25,
  distinctCount: 40,
  min: "1.5",
  max: "99",
})

const ROWS = [schemaRow(column("city", "Nullable(String)")), schemaRow(column("amount", "Decimal(12, 2)"), true)]

const renderCard = (rows: SchemaRow[], profile: ProfileState, sortingKey?: string | null) =>
  render(<ColumnsCard rows={rows} profile={profile} sortingKey={sortingKey} />)

const nameButton = (name: string) => screen.getByRole("button", { name })
const rowOf = (name: string) => nameButton(name).closest("[role=row]") as HTMLElement
const panelOf = (name: string) =>
  document.getElementById(nameButton(name).getAttribute("aria-controls") ?? "") as HTMLElement | null
/** What a row says where a bar would be, when no value is listed. */
const NO_VALUES = /Mostly unique|All null|Many distinct values/
/** The row's name line, and its type line (the second line below `@lg`): the two places a mark is rendered. */
const nameLine = (row: HTMLElement) => within(row).getByRole("button").parentElement as HTMLElement
const typeLine = (row: HTMLElement, dataType: string) => within(row).getByText(dataType).parentElement as HTMLElement
/** A mark is rendered beside the name and on the type's line; CSS shows one of them. */
function expectMark(row: HTMLElement, dataType: string, text: string) {
  expect(within(nameLine(row)).getByText(text)).toBeTruthy()
  expect(within(typeLine(row, dataType)).getByText(text)).toBeTruthy()
}
/** The value shown for a fact of the opened column. */
const fact = (panel: HTMLElement, label: string) => within(panel).getByText(label).nextElementSibling?.textContent

describe("a column's row", () => {
  it("is one line of name, type, commonest value with its share, null share and distinct count, and no chip list", () => {
    renderCard(ROWS, ready([CITY, AMOUNT]))

    const row = rowOf("city")
    expect(within(row).getByText("Nullable(String)")).toBeTruthy()
    expect(within(row).getByText("Jakarta")).toBeTruthy()
    expect(within(row).getByText("60.0%")).toBeTruthy()
    expect(within(row).getByText("5.0%")).toBeTruthy()
    expect(within(row).getByText("≈ 4")).toBeTruthy()
    // The chips named every listed value and its count; the row names the commonest only.
    expect(within(row).queryByText("Bandung")).toBeNull()
    expect(within(row).queryByText("1.2K")).toBeNull()
    expect(within(row).queryByText("500")).toBeNull()
    // The range of a text column is empty, and the number column's reads as before.
    expect(within(rowOf("amount")).getByText("1.5 – 99")).toBeTruthy()
    expect(within(rowOf("amount")).getByText("25.0%")).toBeTruthy()
    expect(within(rowOf("amount")).getByText("≈ 40")).toBeTruthy()
  })

  it("draws a bar with one segment per listed value, as wide as its share, and none for the rest", () => {
    renderCard(ROWS, ready([CITY, AMOUNT]))

    const bar = within(rowOf("city")).getByRole("img", { name: /^Most frequent values/ })
    const segments = [...bar.children] as HTMLElement[]
    expect(segments.map((s) => s.style.width)).toEqual(["60%", "25%", "5%"])
    // The label says what the colours cannot, and a segment's tooltip names its value.
    expect(bar.getAttribute("aria-label")).toBe("Most frequent values: Jakarta 60.0%, Bandung 25.0%, (empty) 5.0%")
    expect(segments.map((s) => s.getAttribute("title"))).toEqual([
      "Jakarta · 1,200 rows (60.0%)",
      "Bandung · 500 rows (25.0%)",
      "(empty) · 100 rows (5.0%)",
    ])
    // One hue at stepped strengths, the commonest strongest.
    expect(segments[0].classList.contains("bg-chart-3")).toBe(true)
    expect(segments[1].classList.contains("bg-chart-3/80")).toBe(true)
    expect(segments[2].classList.contains("bg-chart-3/60")).toBe(true)
  })

  // A bar's length is compared down the list, so its track must be as long on every row. What
  // guarantees it is that the bar and its label are two tracks of fixed proportions, neither
  // sized by its content; layout itself is CSS, so this asserts the structure that does it.
  it("gives the bar a track of its own that does not depend on the label, for a short value and a long one", () => {
    const LONG = "a commonest value that goes on and on, far past any label there could be room for"
    renderCard(
      [schemaRow(column("short")), schemaRow(column("long"))],
      ready([
        stat("short", { topValues: [{ value: "a", count: 2000 }] }),
        stat("long", { topValues: [{ value: LONG, count: 2000 }] }),
      ])
    )
    const bars = ["short", "long"].map(
      (name) => within(rowOf(name)).getByRole("img", { name: /^Most frequent values/ })
    )
    const [shortGrid, longGrid] = bars.map((b) => b.parentElement as HTMLElement)

    // The same two tracks on both rows, each `minmax(0, 1fr)`: no `auto`, no content-sized track.
    expect(shortGrid.className).toBe(longGrid.className)
    expect(shortGrid.classList.contains("grid")).toBe(true)
    expect(shortGrid.classList.contains("@2xl:grid-cols-[minmax(0,1fr)_minmax(0,1fr)]")).toBe(true)
    expect(shortGrid.className).not.toMatch(/auto|content/)
    for (const bar of bars) {
      // The bar fills its own track; it is not what is left after the label (`flex-1`) nor has a floor.
      expect(bar.classList.contains("w-full")).toBe(true)
      expect(bar.classList.contains("flex-1")).toBe(false)
      expect([...bar.classList].some((c) => c.startsWith("min-w-") || c.startsWith("max-w-"))).toBe(false)
    }
    // The label is the second track: the value truncates inside it, the share keeps the right edge.
    const label = bars[1].nextElementSibling as HTMLElement
    expect(label.classList.contains("min-w-0")).toBe(true)
    const [value, share] = [...label.children] as HTMLElement[]
    expect(value.textContent).toBe(LONG)
    expect(value.classList.contains("truncate")).toBe(true)
    expect(share.textContent).toBe("100.0%")
    expect(share.classList.contains("shrink-0")).toBe(true)
    expect(share.classList.contains("text-right")).toBe(true)
    expect(share.classList.contains("tabular-nums")).toBe(true)
    // A bar still stands for its own share of the track.
    expect((bars[1].firstElementChild as HTMLElement).style.width).toBe("100%")
  })

  // One number, one spelling: a share under 0.1% reads "<0.1%" in the row's meter, in the opened
  // column's list and in its facts, never "0.1%" in one and "<0.1%" in another.
  it("writes a null share under 0.1% as <0.1% in the row, the list and the facts alike", () => {
    renderCard(
      [schemaRow(column("material_group"))],
      ready(
        [
          stat("material_group", {
            nullCount: 28,
            // 0.056%: `formatPercent` alone rounds this up to "0.1%".
            nullFraction: 0.00056,
            topValues: [{ value: "A", count: 40000 }],
          }),
        ],
        { rowsProfiled: 50000 }
      )
    )
    expect(within(rowOf("material_group")).getByText("<0.1%")).toBeTruthy()

    fireEvent.click(nameButton("material_group"))
    const panel = panelOf("material_group") as HTMLElement
    expect(within(panel).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "A40,00080.0%",
      "Other values9,97219.9%",
      "Null28<0.1%",
    ])
    expect(fact(panel, "Nulls")).toBe("28 (<0.1%)")
    expect(within(panel).queryByText(/\(0\.1%\)/)).toBeNull()
  })

  it("keeps a null share of exactly zero as 0.0%, and an ordinary one as it was", () => {
    renderCard(
      [schemaRow(column("none")), schemaRow(column("some"))],
      ready([stat("none"), stat("some", { nullCount: 500, nullFraction: 0.25 })])
    )
    expect(within(rowOf("none")).getByText("0.0%")).toBeTruthy()
    expect(within(rowOf("some")).getByText("25.0%")).toBeTruthy()
    fireEvent.click(nameButton("none"))
    expect(fact(panelOf("none") as HTMLElement, "Nulls")).toBe("0 (0.0%)")
  })

  it("writes the empty string as (empty) when it is the commonest value", () => {
    renderCard(
      [schemaRow(column("note"))],
      ready([stat("note", { topValues: [{ value: "", count: 1500 }] })])
    )
    const row = rowOf("note")
    expect(within(row).getByText("(empty)")).toBeTruthy()
    expect(within(row).getByText("75.0%")).toBeTruthy()
  })

  // An empty list is not "unique": the route lists a value only where its count is exact, so it
  // means more distinct values than the profile counts exactly. The label says what the numbers
  // support, in place of the bar.
  it("says a column is mostly unique where its distinct count is nearly every non-null row", () => {
    renderCard(
      [schemaRow(column("order_no"))],
      ready([stat("order_no", { distinctCount: 1990, nullCount: 0, nullFraction: 0 })])
    )
    const row = rowOf("order_no")
    const label = within(row).getByText("Mostly unique")
    expect(label.getAttribute("title")).toBe(
      "No value is listed: the profile states a value's count only where it is exact."
    )
    expect(within(row).queryByRole("img", { name: /^Most frequent values/ })).toBeNull()
  })

  it("says a column has many distinct values, not that it is unique, when few of its rows differ", () => {
    // `AMOUNT`: about 40 distinct values among 1,500 non-null rows, and still no listed value.
    renderCard(ROWS, ready([CITY, AMOUNT]))
    const label = within(rowOf("amount")).getByText("Many distinct values")
    expect(label.getAttribute("title")).toContain("No value is listed")
    expect(within(rowOf("amount")).queryByText("Mostly unique")).toBeNull()
    expect(within(rowOf("amount")).queryByRole("img", { name: /^Most frequent values/ })).toBeNull()
  })

  it("says many distinct values, not mostly unique, when the profile gave no distinct count", () => {
    renderCard([schemaRow(column("code"))], ready([stat("code", { distinctCount: null })]))
    expect(within(rowOf("code")).getByText("Many distinct values")).toBeTruthy()
  })

  it("says a column is all null when every row profiled is null", () => {
    renderCard(
      [schemaRow(column("legacy"))],
      ready([stat("legacy", { nullCount: 2000, nullFraction: 1, distinctCount: 0 })])
    )
    expect(within(rowOf("legacy")).getByText("All null")).toBeTruthy()
    fireEvent.click(nameButton("legacy"))
    // The opened column says the same, and the null line shows where the rows are.
    const panel = panelOf("legacy") as HTMLElement
    expect(within(panel).getByText(/No value is listed/)).toBeTruthy()
    expect(within(panel).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Null2,000100.0%"])
  })

  it("draws no bar when no rows were profiled, rather than a bar of nothing", () => {
    renderCard(ROWS, ready([stat("city", { topValues: [{ value: "Jakarta", count: 5 }] }), AMOUNT], { rowsProfiled: 0 }))
    const row = rowOf("city")
    expect(within(row).queryByRole("img", { name: /^Most frequent values/ })).toBeNull()
    expect(within(row).queryByText(NO_VALUES)).toBeNull()
  })

  it("says a column that could not be profiled was not, and a column the profile lacks is a dash", () => {
    renderCard(
      [schemaRow(column("tags", "Array(String)")), schemaRow(column("gone"))],
      ready([stat("tags", { dataType: "Array(String)", profiled: false })])
    )
    expect(within(rowOf("tags")).getByText("Not profiled (type not supported)")).toBeTruthy()
    expect(within(rowOf("gone")).getByText("—")).toBeTruthy()
    expect(within(rowOf("gone")).queryByText(/Not profiled/)).toBeNull()
  })

  it("gives each type a glyph with the family as its name", () => {
    renderCard(
      [
        schemaRow(column("a", "Int64")),
        schemaRow(column("b", "Nullable(String)")),
        schemaRow(column("c", "DateTime64(3)")),
        schemaRow(column("d", "Bool")),
        schemaRow(column("e", "Array(String)")),
        schemaRow(column("f", "Point")),
      ],
      ready([])
    )
    const glyph = (name: string) => within(rowOf(name)).getAllByRole("img")[0].getAttribute("aria-label")
    expect(["a", "b", "c", "d", "e", "f"].map(glyph)).toEqual([
      "Number",
      "Text",
      "Date or time",
      "Boolean",
      "Nested",
      "Other type",
    ])
  })

  it("marks masked, classified and partitioned columns as before, and the sort key's column", () => {
    renderCard(
      [
        schemaRow(column("email", "String", { masked: true, classification: "restricted" })),
        schemaRow(column("day", "Date"), null, "day"),
        schemaRow(column("plnt")),
      ],
      ready([]),
      "plnt"
    )
    const email = rowOf("email")
    expectMark(email, "String", "masked")
    expectMark(email, "String", "Restricted")
    expectMark(rowOf("day"), "Date", "partition · day")
    expectMark(rowOf("plnt"), "String", "sort key")
    expect(within(email).queryByText("sort key")).toBeNull()
  })

  it("marks a column the source no longer has as inactive since its date, and no other column", () => {
    renderCard(
      [
        schemaRow(column("qty", "Int64", { inactiveSince: "2026-10-09T12:00:00.000Z" })),
        schemaRow(column("id", "Int64", { inactiveSince: null })),
        schemaRow(column("note")),
      ],
      ready([])
    )
    expectMark(rowOf("qty"), "Int64", "inactive since Oct 9, 2026")
    expect(
      within(nameLine(rowOf("qty"))).getByTitle("The source no longer has this column. Its old values are kept.")
    ).toBeTruthy()
    expect(within(rowOf("id")).queryByText(/inactive since/)).toBeNull()
    expect(within(rowOf("note")).queryByText(/inactive since/)).toBeNull()
  })

  it("marks a column the sorting key names, but not one that is only inside an expression", () => {
    renderCard(
      [schemaRow(column("plnt")), schemaRow(column("material")), schemaRow(column("d", "Date"))],
      ready([]),
      "toYYYYMM(d), plnt, material"
    )
    expectMark(rowOf("plnt"), "String", "sort key")
    expectMark(rowOf("material"), "String", "sort key")
    expect(within(rowOf("d")).queryByText("sort key")).toBeNull()
  })

  it("keeps the marks off the name's line below @lg, on the type's line after the type", () => {
    renderCard([schemaRow(column("plnt")), schemaRow(column("plain"))], ready([]), "plnt")
    const row = rowOf("plnt")

    // Beside the name only from @lg: below it the name's line is the button alone, and the
    // button is what is left to truncate, with the whole track.
    const marksBesideName = nameLine(row).lastElementChild as HTMLElement
    expect(marksBesideName.textContent).toBe("sort key")
    expect(marksBesideName.classList.contains("hidden")).toBe(true)
    expect(marksBesideName.classList.contains("@lg:flex")).toBe(true)
    expect(marksBesideName.classList.contains("flex")).toBe(false)

    // On the type's line below @lg, after the type, and gone from @lg up. The type is what
    // truncates; the marks do not shrink.
    const line = typeLine(row, "String")
    const [type, marksUnderName] = [...line.children] as HTMLElement[]
    expect(type.textContent).toBe("String")
    expect(type.classList.contains("truncate")).toBe(true)
    expect(type.classList.contains("min-w-0")).toBe(true)
    expect(marksUnderName.textContent).toBe("sort key")
    expect(marksUnderName.classList.contains("flex")).toBe(true)
    expect(marksUnderName.classList.contains("@lg:hidden")).toBe(true)
    expect(marksUnderName.classList.contains("shrink-0")).toBe(true)

    // Both lines are as tall for a marked column as for an unmarked one, so rows stay of one height.
    const plain = rowOf("plain")
    for (const r of [row, plain]) {
      expect(nameLine(r).classList.contains("h-5")).toBe(true)
      expect(typeLine(r, "String").classList.contains("h-5")).toBe(true)
    }
    // A column with no marks has no empty wrapper to take a gap.
    expect(typeLine(plain, "String").children).toHaveLength(1)
    expect(nameLine(plain).children).toHaveLength(1)
  })

  it("marks no column when the table has no sorting key", () => {
    renderCard([schemaRow(column("plnt"))], ready([]), null)
    expect(screen.queryByText("sort key")).toBeNull()
  })
})

describe("opening a column", () => {
  it("is done by a real button that says whether it is open and what it opens", () => {
    renderCard(ROWS, ready([CITY, AMOUNT]))

    const button = nameButton("city")
    expect(button.tagName).toBe("BUTTON")
    expect(button.getAttribute("type")).toBe("button")
    expect(button.getAttribute("aria-expanded")).toBe("false")
    expect(panelOf("city")).toBeNull()

    fireEvent.click(button)
    expect(button.getAttribute("aria-expanded")).toBe("true")
    const panel = panelOf("city")
    expect(panel).toBeTruthy()
    expect(panel?.closest("[role=row]")).toBe(rowOf("city").nextElementSibling)
  })

  it("lists every value with its count and share, then the rest and the nulls, and the column's facts", () => {
    renderCard(
      [schemaRow(column("city", "Nullable(String)"), true, null)],
      ready([CITY])
    )
    fireEvent.click(nameButton("city"))
    const panel = panelOf("city") as HTMLElement

    expect(within(panel).getAllByRole("listitem").map((li) => li.textContent)).toEqual([
      "Jakarta1,20060.0%",
      "Bandung50025.0%",
      "(empty)1005.0%",
      "Other values1005.0%",
      "Null1005.0%",
    ])
    // A value's bar is as wide as its share, at the step its segment has.
    const bars = within(panel)
      .getAllByRole("listitem")
      .map((li) => li.querySelector("[aria-hidden] > span") as HTMLElement)
    expect(bars.slice(0, 3).map((b) => b.style.width)).toEqual(["60%", "25%", "5%"])
    expect(bars[0].classList.contains("bg-chart-3")).toBe(true)
    expect(bars[1].classList.contains("bg-chart-3/80")).toBe(true)
    // The rest and the nulls are in the plain tone, not a step.
    expect(bars[3].classList.contains("bg-foreground/25")).toBe(true)
    expect(bars[4].classList.contains("bg-foreground/25")).toBe(true)

    expect(fact(panel, "Type")).toBe("Nullable(String)")
    expect(fact(panel, "Can be null")).toBe("Yes")
    expect(fact(panel, "Nulls")).toBe("100 (5.0%)")
    expect(fact(panel, "Distinct")).toBe("≈ 4")
    expect(within(panel).queryByText("Range")).toBeNull()
  })

  it("leaves out 'Other values' and 'Null' when there are none, and says why no value is listed", () => {
    renderCard([schemaRow(column("amount", "Decimal(12, 2)"), false)], ready([{ ...AMOUNT, nullCount: 0, nullFraction: 0 }]))
    fireEvent.click(nameButton("amount"))
    const panel = panelOf("amount") as HTMLElement

    expect(within(panel).queryByText("Null")).toBeNull()
    // Everything that is not null is "other" when nothing is listed.
    expect(within(panel).getAllByRole("listitem").map((li) => li.textContent)).toEqual(["Other values2,000100.0%"])
    expect(within(panel).getByText(/No value is listed/)).toBeTruthy()
    expect(fact(panel, "Can be null")).toBe("No")
    expect(fact(panel, "Range")).toBe("1.5 – 99")
  })

  it("closes on a second press, and two columns can be open together", () => {
    renderCard(ROWS, ready([CITY, AMOUNT]))

    fireEvent.click(nameButton("city"))
    fireEvent.click(nameButton("amount"))
    expect(panelOf("city")).toBeTruthy()
    expect(panelOf("amount")).toBeTruthy()

    fireEvent.click(nameButton("city"))
    expect(panelOf("city")).toBeNull()
    expect(nameButton("city").getAttribute("aria-expanded")).toBe("false")
    expect(panelOf("amount")).toBeTruthy()
  })

  it("opens from anywhere on the line, once", () => {
    renderCard(ROWS, ready([CITY, AMOUNT]))

    fireEvent.click(within(rowOf("city")).getByText("≈ 4"))
    expect(nameButton("city").getAttribute("aria-expanded")).toBe("true")
    // A press on the button reaches the row too; it must not toggle twice.
    fireEvent.click(nameButton("amount"))
    expect(nameButton("amount").getAttribute("aria-expanded")).toBe("true")
    // A press inside the opened column is not a press on the row.
    fireEvent.click(within(panelOf("city") as HTMLElement).getByText("Most frequent values"))
    expect(nameButton("city").getAttribute("aria-expanded")).toBe("true")
  })

  it("says a column is not known to be nullable when nothing settles it", () => {
    renderCard([schemaRow(column("legacy", "string"), null)], ready([stat("legacy")]))
    fireEvent.click(nameButton("legacy"))
    expect(fact(panelOf("legacy") as HTMLElement, "Can be null")).toBe("Not known")
  })

  it("puts the description, the partition, the sort key, the masking and the classification among the facts", () => {
    renderCard(
      [
        schemaRow(
          column("plnt", "String", { description: "Plant code", masked: true, classification: "confidential" }),
          false,
          "identity"
        ),
      ],
      ready([stat("plnt")]),
      "plnt"
    )
    expect(screen.queryByText("Plant code")).toBeNull()

    fireEvent.click(nameButton("plnt"))
    const panel = panelOf("plnt") as HTMLElement
    expect(fact(panel, "Description")).toBe("Plant code")
    expect(fact(panel, "Partitioned by")).toBe("This column (identity)")
    expect(fact(panel, "Sorted by")).toBe("This column")
    expect(fact(panel, "Masked")).toBe("Yes")
    expect(fact(panel, "Classification")).toBe("Confidential")
  })

  it("says the profile was cut when it was, in the words the note uses", () => {
    renderCard(ROWS, ready([CITY, AMOUNT], { sampled: true }))
    fireEvent.click(nameButton("city"))
    expect(within(panelOf("city") as HTMLElement).getByText(/first 100,000 only/)).toBeTruthy()
  })

  it("does not say it for a profile that was not cut", () => {
    renderCard(ROWS, ready([CITY, AMOUNT]))
    fireEvent.click(nameButton("city"))
    expect(within(panelOf("city") as HTMLElement).queryByText(/first 100,000 only/)).toBeNull()
  })

  it("says why there is nothing to list for a column that was not profiled, or is not in the profile", () => {
    renderCard(
      [schemaRow(column("tags", "Array(String)")), schemaRow(column("gone"))],
      ready([stat("tags", { dataType: "Array(String)", profiled: false })])
    )
    fireEvent.click(nameButton("tags"))
    fireEvent.click(nameButton("gone"))
    expect(within(panelOf("tags") as HTMLElement).getByText("Not profiled (type not supported)")).toBeTruthy()
    expect(within(panelOf("gone") as HTMLElement).getByText("This column is not in the profile.")).toBeTruthy()
    // The facts that need no profile are still there.
    expect(fact(panelOf("tags") as HTMLElement, "Type")).toBe("Array(String)")
  })

  it("shows a skeleton for the values while the profile loads, and the facts that need none", () => {
    renderCard(ROWS, { kind: "loading" })
    fireEvent.click(nameButton("city"))
    const panel = panelOf("city") as HTMLElement
    expect(panel.querySelectorAll("[data-slot=skeleton]").length).toBeGreaterThan(0)
    expect(fact(panel, "Type")).toBe("Nullable(String)")
  })
})

describe("what the card shows without a profile", () => {
  const states: [string, ProfileState, string][] = [
    ["restricted", { kind: "restricted" }, "Column statistics are read from the data, so they need the query:read permission."],
    ["unsupported", { kind: "unsupported", reason: "no table to read" }, "No column statistics: no table to read"],
    ["failed", { kind: "error", message: "boom", retry: () => {} }, "Column statistics failed to load: boom"],
  ]

  for (const [name, state, note] of states) {
    it(`keeps the name, type and glyph, drops the statistics, and says why when the profile is ${name}`, () => {
      renderCard(ROWS, state)

      expect(screen.getByText(note, { exact: false })).toBeTruthy()
      expect(screen.queryByRole("columnheader", { name: "Nulls" })).toBeNull()
      expect(screen.queryByRole("columnheader", { name: "Top values" })).toBeNull()
      const row = rowOf("city")
      expect(within(row).getByText("Nullable(String)")).toBeTruthy()
      expect(within(row).getAllByRole("img")[0].getAttribute("aria-label")).toBe("Text")
      expect(within(row).queryByText(NO_VALUES)).toBeNull()
    })

    it(`still opens a column, on the facts that need no profile, when the profile is ${name}`, () => {
      renderCard(ROWS, state)

      fireEvent.click(nameButton("amount"))
      const panel = panelOf("amount") as HTMLElement
      expect(fact(panel, "Type")).toBe("Decimal(12, 2)")
      expect(fact(panel, "Can be null")).toBe("Yes")
      expect(within(panel).queryByText("Most frequent values")).toBeNull()
      expect(within(panel).queryByText("Nulls")).toBeNull()
    })
  }

  it("offers a retry that asks again when the profile failed", () => {
    let retried = 0
    renderCard(ROWS, { kind: "error", message: "boom", retry: () => (retried += 1) })
    fireEvent.click(screen.getByRole("button", { name: "Retry" }))
    expect(retried).toBe(1)
  })

  it("shows a skeleton the height of the row, not a dash, in the statistic cells while the profile loads", () => {
    renderCard(ROWS, { kind: "loading" })
    const row = rowOf("city")
    expect(row.querySelectorAll("[data-slot=skeleton]").length).toBeGreaterThanOrEqual(2)
    expect(within(row).queryByText(NO_VALUES)).toBeNull()
    expect(within(row).queryByText("—")).toBeNull()
    expect(screen.getByText("Profiling columns…")).toBeTruthy()
    expect(screen.getByRole("columnheader", { name: "Nulls" })).toBeTruthy()
  })
})

describe("the filter and the page size", () => {
  const named = (n: number) => Array.from({ length: n }, (_, i) => schemaRow(column(`col_${String(i + 1).padStart(2, "0")}`)))

  it("puts the filter box on a table of 11 columns and not on one of 10", () => {
    const { unmount } = renderCard(named(10), ready([]))
    expect(screen.queryByLabelText("Filter columns")).toBeNull()
    unmount()

    renderCard(named(11), ready([]))
    expect(screen.getByLabelText("Filter columns")).toBeTruthy()
    // 11 columns fit the first page, so there is no page size to pick.
    expect(screen.queryByRole("group", { name: "Columns to show" })).toBeNull()
  })

  it("keeps the page sizes for a table past 25 columns", () => {
    renderCard(named(26), ready([]))
    const sizes = screen.getByRole("group", { name: "Columns to show" })
    expect(within(sizes).getAllByRole("button").map((b) => b.textContent)).toEqual(["25", "All"])
    expect(screen.getAllByRole("row")).toHaveLength(26)
  })

  it("finds a column by a word of its description, which the row no longer shows", () => {
    const rows = [...named(11), schemaRow(column("net_wt", "Float64", { description: "Net weight in kilograms" }))]
    renderCard(rows, ready([]))
    expect(screen.queryByText("Net weight in kilograms")).toBeNull()

    fireEvent.change(screen.getByLabelText("Filter columns"), { target: { value: "kilogram" } })
    expect(screen.getAllByRole("row")).toHaveLength(2)
    expect(nameButton("net_wt")).toBeTruthy()
    expect(screen.getByText('1 of 12 columns match "kilogram".')).toBeTruthy()

    fireEvent.click(nameButton("net_wt"))
    expect(fact(panelOf("net_wt") as HTMLElement, "Description")).toBe("Net weight in kilograms")
  })

  it("keeps a column open while the filter narrows the list and widens it again", () => {
    renderCard(named(12), ready([]))
    fireEvent.click(nameButton("col_03"))

    fireEvent.change(screen.getByLabelText("Filter columns"), { target: { value: "col_07" } })
    expect(screen.queryByRole("button", { name: "col_03" })).toBeNull()
    fireEvent.change(screen.getByLabelText("Filter columns"), { target: { value: "" } })

    expect(nameButton("col_03").getAttribute("aria-expanded")).toBe("true")
    expect(panelOf("col_03")).toBeTruthy()
  })

  it("still says no column matches, and leaves the box to clear it", () => {
    renderCard(named(12), ready([]))
    fireEvent.change(screen.getByLabelText("Filter columns"), { target: { value: "nothing-like-this" } })
    expect(screen.getByText('No column matches "nothing-like-this"')).toBeTruthy()
    expect(screen.getByLabelText("Filter columns")).toBeTruthy()
  })

  it("says there are no columns for a table that registered none", () => {
    renderCard([], ready([]))
    expect(screen.getByText("No columns registered")).toBeTruthy()
  })
})

describe("the list's structure", () => {
  // The shared `Table` wraps its table in an `overflow-x-auto` scroller, which is what scrolled the
  // old card sideways at 390 px. A list of `div`s with the ARIA table roles has no such scroller;
  // that the cells of a row cannot widen it is CSS, checked in a browser, not here.
  it("is a table by role and has no <table>, so nothing in it can scroll sideways", () => {
    const { container } = renderCard(ROWS, ready([CITY, AMOUNT]))

    expect(container.querySelector("table")).toBeNull()
    expect(container.querySelector("[data-slot=table-container]")).toBeNull()
    expect(container.querySelector(".overflow-x-auto")).toBeNull()
    const table = screen.getByRole("table", { name: "Columns" })
    expect(within(table).getAllByRole("columnheader").map((h) => h.textContent)).toEqual([
      "Kind",
      "Column",
      "Type",
      "Top values",
      "Nulls",
      "Distinct",
      "Range",
      "Details",
    ])
    // The header, then one row per column, each of the same cells.
    expect(within(table).getAllByRole("row")).toHaveLength(3)
  })

  // A date column's range is "2019-01-20 – 2025-09-23": 23 characters of mono `text-xs`, and a
  // type such as `Nullable(Float64)` is 17. Each has to show whole where it shows at all, and
  // where the card is short of room it is the range that waits: the type matters more here. So
  // the type has a fixed track from `@3xl` and the range appears, on a fixed track, only from
  // `@5xl`. Layout is CSS; this asserts the tracks.
  it("holds the type whole from @3xl and shows the range, whole, only from @5xl", () => {
    const { container } = renderCard(ROWS, ready([CITY, AMOUNT]))
    const template = (el: HTMLElement, at: string) =>
      [...el.classList].find((c) => c.startsWith(`${at}:grid-cols-[`))
    const tracks = (el: HTMLElement, at: string) =>
      (template(el, at) ?? "").slice(`${at}:grid-cols-[`.length, -1).split("_")
    const header = screen.getAllByRole("row")[0]
    // The header and every row share the one template, so the label stays over its cells.
    for (const at of ["@2xl", "@3xl", "@5xl"]) {
      expect(template(rowOf("city"), at)).toBe(template(header, at))
      expect(template(rowOf("amount"), at)).toBe(template(header, at))
    }
    const rem = (track: string) => {
      expect(track).toMatch(/^\d+(\.\d+)?rem$/)
      return parseFloat(track)
    }
    const CHAR = 0.6 * 0.75 // rem: 0.6em a character of a 0.75rem font

    // glyph, name, type, bar and label, nulls, distinct, chevron: the type is a fraction.
    const at2xl = tracks(header, "@2xl")
    expect(at2xl).toHaveLength(7)
    expect(at2xl[2]).toBe("minmax(0,1fr)")
    // From @3xl it is fixed, and holds `Nullable(Float64)`.
    const at3xl = tracks(header, "@3xl")
    expect(at3xl).toHaveLength(7)
    expect(rem(at3xl[2])).toBeGreaterThanOrEqual(17 * CHAR)
    // The bar and its label still share one `minmax(0, 2fr)` values track on every row.
    expect(at3xl[3]).toBe("minmax(0,2fr)")

    // From @5xl, glyph, name, type, bar and label, nulls, distinct, range, chevron.
    const at5xl = tracks(header, "@5xl")
    expect(at5xl).toHaveLength(8)
    expect(at5xl[2]).toBe(at3xl[2])
    expect(rem(at5xl[6])).toBeGreaterThanOrEqual(23 * CHAR)

    // No range below @5xl: its header and its cell are hidden until then, and nothing else
    // names the old @4xl.
    const rangeHeader = screen.getByRole("columnheader", { name: "Range" })
    const rangeCell = within(rowOf("amount")).getAllByRole("cell")[6]
    for (const el of [rangeHeader, rangeCell]) {
      expect(el.classList.contains("hidden")).toBe(true)
      expect(el.classList.contains("@5xl:block")).toBe(true)
    }
    expect(container.innerHTML).not.toContain("@4xl")
  })

  it("gives a row one cell under each header, so a number is read under its column", () => {
    renderCard(ROWS, ready([CITY, AMOUNT]))
    // glyph, name, type, bar, nulls, distinct, range, chevron
    expect(within(rowOf("city")).getAllByRole("cell")).toHaveLength(8)
  })
})
