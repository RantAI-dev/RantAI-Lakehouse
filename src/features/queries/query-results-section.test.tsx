// The grid's tools: the per-column filters, the statistics panel, the cell
// inspector and the copy as TSV, plus the banner a cut result carries.
//
// The shared table renders under happy-dom, so the real section is rendered
// here rather than a stand-in: the filters, the statistics and the copy all
// read the same rows, and only the real section proves that they do.
//
// `next/navigation` is stubbed before any import resolves (`mock.module` is
// hoisted), because the table's URL-backed state reads it.
mock.module("next/navigation", () => ({
  usePathname: () => "/query-studio",
  useSearchParams: () => new URLSearchParams(),
  useRouter: () => ({
    push: () => {},
    replace: () => {},
    refresh: () => {},
    back: () => {},
    forward: () => {},
    prefetch: () => {},
  }),
}))

import { cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, mock, spyOn } from "bun:test"
import { TableProviders } from "@/components/app-shell/table-providers"
import * as notify from "@/lib/notify"
import type { QueryResult } from "@/services/contracts/queries"
import { QueryResultBanner } from "./query-result-banner"
import { QueryResultsSection } from "./query-results-section"

const originalClipboard = Object.getOwnPropertyDescriptor(navigator, "clipboard")
const originalSecure = Object.getOwnPropertyDescriptor(globalThis, "isSecureContext")

/** Everything the clipboard was given, in the order it was given. */
let copied: string[] = []

beforeEach(() => {
  copied = []
  // The clipboard is only used in a secure context; happy-dom does not
  // pretend the test page is one.
  Object.defineProperty(globalThis, "isSecureContext", {
    value: true,
    configurable: true,
  })
  Object.defineProperty(navigator, "clipboard", {
    value: {
      writeText: async (text: string) => {
        copied.push(text)
      },
    },
    configurable: true,
  })
})

afterEach(() => {
  cleanup()
  mock.restore()
  if (originalClipboard) {
    Object.defineProperty(navigator, "clipboard", originalClipboard)
  } else {
    Reflect.deleteProperty(navigator, "clipboard")
  }
  if (originalSecure) {
    Object.defineProperty(globalThis, "isSecureContext", originalSecure)
  }
})

function result(overrides: Partial<QueryResult> = {}): QueryResult {
  return {
    id: "q-1",
    columns: ["city", "amount"],
    rows: [
      { city: "Jakarta", amount: 10 },
      { city: "Bandung", amount: 20 },
      { city: "Jakarta Selatan", amount: 30 },
    ],
    engine: "clickhouse",
    rowCount: 3,
    truncated: false,
    rowLimit: 100,
    metrics: {
      durationMs: 12,
      scannedBytes: 1024,
      costUnits: 0.01,
      engine: "hot-store",
      workloadClass: "hot-analytics",
      cacheHit: null,
      pushdowns: null,
      policyObligations: null,
    },
    plan: null,
    ...overrides,
  }
}

function renderSection(r: QueryResult) {
  return render(
    <TableProviders>
      <QueryResultsSection
        result={r}
        runId="7f0c0f0e-3c7e-4a3e-9d4f-0a1b2c3d4e5f"
        downloadBusy={null}
        onDownload={() => {}}
      />
    </TableProviders>
  )
}

/**
 * The grid alone. The statistics panel shows the same values as the grid —
 * a column's smallest value and its most frequent ones — so a claim about
 * what the grid shows has to look at the grid.
 */
function grid() {
  return within(screen.getByRole("table"))
}

/** The statistics panel alone, for the same reason. */
function panel() {
  return within(screen.getByRole("region", { name: "Column statistics" }))
}

describe("the column filters", () => {
  it("gives every column a field of its own", () => {
    renderSection(result())
    expect(screen.getByLabelText("Filter city")).toBeTruthy()
    expect(screen.getByLabelText("Filter amount")).toBeTruthy()
  })

  it("narrows the grid to the rows that match", () => {
    renderSection(result())
    fireEvent.change(screen.getByLabelText("Filter city"), {
      target: { value: "jakarta" },
    })
    expect(grid().getByText("Jakarta")).toBeTruthy()
    expect(grid().getByText("Jakarta Selatan")).toBeTruthy()
    // `queryAllBy…` and a length, not `queryBy…` and `toBeNull()`: a failed
    // assertion on a DOM node makes the runner print the whole document,
    // which takes minutes on a table this size.
    expect(grid().queryAllByText("Bandung")).toHaveLength(0)
  })

  it("combines two filters with AND", () => {
    renderSection(result())
    fireEvent.change(screen.getByLabelText("Filter city"), {
      target: { value: "jakarta" },
    })
    fireEvent.change(screen.getByLabelText("Filter amount"), {
      target: { value: "10" },
    })
    expect(grid().getByText("Jakarta")).toBeTruthy()
    expect(grid().queryAllByText("Jakarta Selatan")).toHaveLength(0)
    expect(grid().queryAllByText("Bandung")).toHaveLength(0)
  })

  it("shows the clear button only when a filter holds text, and clears every filter", () => {
    renderSection(result())
    expect(screen.queryAllByRole("button", { name: "Clear filters" })).toHaveLength(
      0
    )

    fireEvent.change(screen.getByLabelText("Filter city"), {
      target: { value: "jakarta" },
    })
    fireEvent.change(screen.getByLabelText("Filter amount"), {
      target: { value: "10" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Clear filters" }))

    expect(grid().getByText("Jakarta")).toBeTruthy()
    expect(grid().getByText("Bandung")).toBeTruthy()
    expect(grid().getByText("Jakarta Selatan")).toBeTruthy()
    expect(screen.queryAllByRole("button", { name: "Clear filters" })).toHaveLength(
      0
    )
  })

  it("says the query returned no rows when the result is empty", () => {
    renderSection(result({ rows: [], rowCount: 0 }))
    expect(screen.getByText("The query returned no rows.")).toBeTruthy()
  })

  it("says no row matches the filters when the filter leaves nothing", () => {
    renderSection(result())
    fireEvent.change(screen.getByLabelText("Filter city"), {
      target: { value: "surabaya" },
    })
    expect(screen.getByText("No row matches the filters.")).toBeTruthy()
    expect(screen.queryAllByText("The query returned no rows.")).toHaveLength(0)
  })
})

describe("the statistics panel", () => {
  it("says how many rows the grid shows", () => {
    renderSection(result())
    expect(screen.getByText(/of the 3 rows shown/)).toBeTruthy()
  })

  it("counts the rows that are left after the filter", () => {
    renderSection(result())
    fireEvent.change(screen.getByLabelText("Filter city"), {
      target: { value: "jakarta" },
    })
    expect(screen.getByText(/of the 2 rows shown/)).toBeTruthy()
  })

  it("shows the statistics of the chosen column", () => {
    renderSection(result())
    expect(panel().getByText("Empty cells")).toBeTruthy()
    expect(panel().getByText("Distinct values")).toBeTruthy()
    // The first column is chosen by default, and its smallest and largest
    // values are read in text order.
    expect(panel().getByText("Minimum").parentElement?.textContent).toContain(
      "Bandung"
    )
    expect(panel().getByText("Maximum").parentElement?.textContent).toContain(
      "Jakarta Selatan"
    )
  })

  it("follows the column the reader picks", () => {
    renderSection(result())
    fireEvent.change(screen.getByLabelText("Column"), {
      target: { value: "amount" },
    })
    // The smallest amount is 10 and the largest is 30.
    expect(panel().getByText("Minimum").parentElement?.textContent).toContain(
      "10"
    )
    expect(panel().getByText("Maximum").parentElement?.textContent).toContain(
      "30"
    )
  })

  it("says the histogram of a text column shows the most frequent values", () => {
    renderSection(result())
    // The first column is a text one: its bars are values, not ranges.
    expect(panel().getByText("Most frequent values")).toBeTruthy()
    expect(panel().queryAllByText("Value range")).toHaveLength(0)
  })

  it("says the histogram of a numeric column shows value ranges", () => {
    renderSection(result())
    fireEvent.change(screen.getByLabelText("Column"), {
      target: { value: "amount" },
    })
    expect(panel().getByText("Value range")).toBeTruthy()
    expect(panel().queryAllByText("Most frequent values")).toHaveLength(0)
  })

  it("shows no histogram label when the column holds no value", () => {
    renderSection(result({ rows: [{ city: null, amount: null }] }))
    expect(panel().queryAllByText("Value range")).toHaveLength(0)
    expect(panel().queryAllByText("Most frequent values")).toHaveLength(0)
  })
})

describe("the cell inspector", () => {
  it("names the column and the row in the button, so two rows do not read the same", () => {
    renderSection(result())
    const names = screen
      .getAllByRole("button", { name: /^Inspect city, row \d+$/ })
      .map((button) => button.getAttribute("aria-label"))
    // The rows are numbered the way a reader counts them, so the name of a
    // button points at the row on the screen.
    expect(names).toEqual([
      "Inspect city, row 1",
      "Inspect city, row 2",
      "Inspect city, row 3",
    ])
  })

  it("shows the whole value of a cell", async () => {
    renderSection(
      result({
        rows: [{ city: "a very long city name", amount: 10 }],
      })
    )
    fireEvent.click(screen.getByRole("button", { name: "Inspect city, row 1" }))
    const dialog = await screen.findByRole("dialog")
    expect(within(dialog).getByText("a very long city name")).toBeTruthy()
  })

  it("pretty-prints a cell whose value is JSON", async () => {
    renderSection(
      result({
        rows: [{ city: '{"name":"Ada","age":36}', amount: 10 }],
      })
    )
    fireEvent.click(screen.getByRole("button", { name: "Inspect city, row 1" }))
    const dialog = await screen.findByRole("dialog")
    const shown = within(dialog).getByText(/"name": "Ada"/)
    expect(shown.textContent).toContain("\n")
  })

  it("copies the raw value and says it worked", async () => {
    const success = spyOn(notify, "notifySuccess")
    renderSection(
      result({
        rows: [{ city: '{"name":"Ada","age":36}', amount: 10 }],
      })
    )
    fireEvent.click(screen.getByRole("button", { name: "Inspect city, row 1" }))
    fireEvent.click(await screen.findByRole("button", { name: "Copy value" }))

    await waitFor(() => expect(copied).toEqual(['{"name":"Ada","age":36}']))
    expect(success).toHaveBeenCalledTimes(1)
  })
})

describe("copy as TSV", () => {
  it("copies the rows the grid shows, with a header row", async () => {
    const success = spyOn(notify, "notifySuccess")
    renderSection(result())
    fireEvent.click(screen.getByRole("button", { name: "Copy as TSV" }))

    await waitFor(() =>
      expect(copied).toEqual([
        "city\tamount\r\nJakarta\t10\r\nBandung\t20\r\nJakarta Selatan\t30",
      ])
    )
    expect(success).toHaveBeenCalledTimes(1)
  })

  it("copies the rows that are left after the filter", async () => {
    renderSection(result())
    fireEvent.change(screen.getByLabelText("Filter city"), {
      target: { value: "bandung" },
    })
    fireEvent.click(screen.getByRole("button", { name: "Copy as TSV" }))

    await waitFor(() =>
      expect(copied).toEqual(["city\tamount\r\nBandung\t20"])
    )
  })

  it("says so when the copy did not work", async () => {
    const failure = spyOn(notify, "notifyFailure")
    Object.defineProperty(navigator, "clipboard", {
      value: undefined,
      configurable: true,
    })
    document.execCommand = (() => false) as unknown as typeof document.execCommand

    renderSection(result())
    fireEvent.click(screen.getByRole("button", { name: "Copy as TSV" }))

    await waitFor(() => expect(failure).toHaveBeenCalledTimes(1))
  })

  it("cannot be pressed when there is no row to copy", () => {
    renderSection(result({ rows: [], rowCount: 0 }))
    const button = screen.getByRole("button", { name: "Copy as TSV" })
    expect((button as HTMLButtonElement).disabled).toBe(true)
  })
})

describe("the banner of a cut result", () => {
  it("names the row limit that cut the result", () => {
    render(<QueryResultBanner rowCount={10} rowLimit={64000} cutBy="rows" />)
    expect(
      screen.getByText("Showing the first 10 rows: the limit is 64,000 rows.")
    ).toBeTruthy()
  })

  it("names the byte cap that cut the result", () => {
    render(<QueryResultBanner rowCount={10} rowLimit={64000} cutBy="bytes" />)
    expect(
      screen.getByText("Showing the first 10 rows: the result passed 10 MB.")
    ).toBeTruthy()
  })

  it("says the result was cut when the server does not say which limit cut it", () => {
    render(<QueryResultBanner rowCount={10} rowLimit={64000} />)
    expect(
      screen.getByText("Showing the first 10 rows: the result was cut.")
    ).toBeTruthy()
  })

  it("shows the banner only for a cut result", () => {
    renderSection(result({ truncated: true, cutBy: "rows", rowLimit: 3 }))
    expect(
      screen.getByText("Showing the first 3 rows: the limit is 3 rows.")
    ).toBeTruthy()
  })

  it("does not claim a result is cut when it is not", () => {
    renderSection(result())
    expect(screen.queryAllByText(/Showing the first/)).toHaveLength(0)
  })
})
