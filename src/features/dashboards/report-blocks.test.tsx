import { cleanup, render, screen } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock } from "bun:test"
import * as realTileBody from "./tile-body"
import type { ReportChartEntry } from "@/services/contracts/dashboard-reports"

// BI-62 (T3): the one renderer of a report, for every block kind. The chart
// body is the dashboard's own `TileBody`, replaced by a marker; `mock.module`
// outlives this file in `bun test src`, so the real exports go back after.
const tileBody = { ...realTileBody }
mock.module("./tile-body", () => ({
  ...tileBody,
  TileBody: ({ spec }: { spec: { id: string } }) => <div data-testid={`tile-${spec.id}`} />,
}))
afterAll(() => {
  mock.module("./tile-body", () => tileBody)
})

const { ReportBlocks, InlineText } = await import("./report-blocks")

afterEach(() => cleanup())

// Only the fields the card reads; the body is the marker above.
const entry = (id: string) =>
  ({
    spec: { id, title: `Chart ${id}`, kind: "bar", source: "ui" },
    cell: { columns: [], rows: [] },
    board: "b_1",
    boardName: "Sales",
  }) as unknown as ReportChartEntry

describe("ReportBlocks (BI-62)", () => {
  it("draws bold, italic and a safe inline link as elements, built from the text", () => {
    const { container } = render(
      <ReportBlocks blocks={[{ type: "paragraph", text: "Visits **rose** in *June*, see [the method](https://a.example/m)." }]} charts={{}} dark={false} />,
    )
    expect(container.querySelector("strong")?.textContent).toBe("rose")
    expect(container.querySelector("em")?.textContent).toBe("June")
    const link = screen.getByRole("link", { name: "the method" })
    expect(link.getAttribute("href")).toBe("https://a.example/m")
    expect(link.getAttribute("target")).toBe("_blank")
    expect(link.getAttribute("rel")).toBe("noopener noreferrer")
    expect(container.querySelector("p")?.textContent).toBe("Visits rose in June, see the method.")
  })

  it("keeps a console link in the console and shows a link that could run code as words", () => {
    const { container } = render(
      <ReportBlocks
        blocks={[
          { type: "paragraph", text: "[home](/dashboards/b_1) and [run](javascript:alert(1)) and <b>x</b>" },
        ]}
        charts={{}}
        dark={false}
      />,
    )
    const home = screen.getByRole("link", { name: "home" })
    expect(home.getAttribute("href")).toBe("/dashboards/b_1")
    expect(home.getAttribute("target")).toBeNull()
    expect(screen.queryByRole("link", { name: "run" })).toBeNull()
    expect(container.querySelector("b")).toBeNull()
    expect(container.textContent).toContain("<b>x</b>")
  })

  it("draws headings by level, lists with marks, a quote and a divider", () => {
    const { container } = render(
      <ReportBlocks
        title="Q3 visitors"
        blocks={[
          { type: "heading", text: "Arrivals", level: 1 },
          { type: "heading", text: "By *month*", level: 2 },
          { type: "heading", text: "Old small", level: 3 },
          { type: "list", items: ["**North** up", "South flat"], ordered: true },
          { type: "list", items: ["a"] },
          { type: "quote", text: "A *quoted* line" },
          { type: "divider" },
        ]}
        charts={{}}
        dark={false}
      />,
    )
    expect(screen.getByRole("heading", { level: 1, name: "Q3 visitors" })).toBeTruthy()
    expect(screen.getByRole("heading", { level: 2, name: "Arrivals" }).getAttribute("data-level")).toBe("1")
    expect(screen.getByRole("heading", { level: 3, name: "By month" }).querySelector("em")?.textContent).toBe("month")
    expect(screen.getByRole("heading", { level: 4, name: "Old small" }).getAttribute("data-level")).toBe("3")
    expect(container.querySelector("ol li strong")?.textContent).toBe("North")
    expect(container.querySelectorAll("ul li")).toHaveLength(1)
    expect(container.querySelector("blockquote em")?.textContent).toBe("quoted")
    expect(container.querySelectorAll("hr")).toHaveLength(1)
  })

  it("draws a chart at its width, live, with its dashboard", () => {
    const { container } = render(
      <ReportBlocks
        blocks={[
          { type: "chart", chart: "c_1", width: "wide" },
          { type: "chart", chart: "c_2" },
          { type: "chart", chart: "c_gone" },
        ]}
        charts={{ c_1: entry("c_1"), c_2: entry("c_2"), c_gone: { unavailable: "This chart is not available." } }}
        dark={false}
      />,
    )
    const blocks = container.querySelectorAll(".rd-chart-block")
    expect(blocks).toHaveLength(3)
    expect(blocks[0].getAttribute("data-width")).toBe("wide")
    expect(blocks[1].getAttribute("data-width")).toBe("text")
    expect(screen.getByTestId("tile-c_1")).toBeTruthy()
    expect(screen.getByTestId("tile-c_2")).toBeTruthy()
    expect(screen.getAllByRole("link", { name: /from Sales/ })[0].getAttribute("href")).toBe("/dashboards/b_1")
    expect(screen.getByText("This chart is not available.")).toBeTruthy()
  })

  it("prints the same blocks, with no dashboard link and wide charts kept to the paper", () => {
    const { container } = render(
      <ReportBlocks
        print
        blocks={[{ type: "paragraph", text: "**Bold**" }, { type: "chart", chart: "c_1", width: "wide" }]}
        charts={{ c_1: entry("c_1") }}
        dark={false}
      />,
    )
    expect(container.querySelector(".rd-doc")?.classList.contains("rd-print")).toBe(true)
    expect(container.querySelector("strong")?.textContent).toBe("Bold")
    expect(screen.queryByRole("link", { name: /from Sales/ })).toBeNull()
    expect(screen.getByText("from Sales")).toBeTruthy()
  })

  it("keeps a report made before BI-62 as it read: a link block on its own line", () => {
    render(<ReportBlocks blocks={[{ type: "link", text: "Method **x**", url: "https://example.com/m" }]} charts={{}} dark={false} />)
    // A link block's words were always plain text, and still are.
    expect(screen.getByRole("link", { name: "Method **x**" }).getAttribute("href")).toBe("https://example.com/m")
  })
})

describe("InlineText", () => {
  it("joins the runs of one link into one anchor", () => {
    const { container } = render(
      <p>
        <InlineText source="[**bold** words](https://a.example)" />
      </p>,
    )
    expect(container.querySelectorAll("a")).toHaveLength(1)
    expect(container.querySelector("a strong")?.textContent).toBe("bold")
  })
})
