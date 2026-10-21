import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { Rows } from "./tile-dialogs"
import { ReportingProvider } from "./reporting-context"

// The chart itself needs a canvas; the option it is handed (what is drawn) does not.
const drawn: { option: Record<string, unknown> }[] = []
mock.module("./echart", () => ({
  EChart: (props: { option: Record<string, unknown> }) => {
    drawn.push(props)
    return <div data-testid="chart" />
  },
}))
const { TileBody } = await import("./tile-body")

afterEach(() => { cleanup(); drawn.length = 0 })

// happy-dom would otherwise fetch the page a frame points at, over the network. Loading is switched off (it logs
// "Iframe page loading is disabled" for each frame); the test reads the frame's attributes only.
const happyDOM = (window as unknown as { happyDOM?: { settings: { disableIframePageLoading: boolean } } }).happyDOM
if (happyDOM) happyDOM.settings.disableIframePageLoading = true

const SITES = ["https://img.example"]
const tile = (spec: Record<string, unknown>, cell: Rows | undefined, sites: string[] = SITES) =>
  render(
    <ReportingProvider reporting={{ timeZone: "UTC", weekStart: "monday", contentSites: sites }}>
      <TileBody spec={{ id: "c_1", title: "Tile", mart: "mart_x", x: "", y: "v", source: "ui", ...spec } as unknown as ChartRenderSpec} cell={cell} dark={false} loading={false} />
    </ReportingProvider>,
  )
const rows = (r: Record<string, unknown>[], extra: Partial<Rows> = {}): Rows => ({ columns: Object.keys(r[0] ?? {}), rows: r, ...extra })

describe("a progress bar", () => {
  it("shows the value against its goal and the percentage reached", () => {
    tile({ kind: "progress", y: "v", def: { goal: 1000, measures: ["visitors"] } }, rows([{ v: 250 }]))
    expect(screen.getByText("25%")).toBeTruthy()
    expect(screen.getByText("250 of 1,000")).toBeTruthy()
    const bar = screen.getByRole("progressbar")
    expect(bar.getAttribute("aria-valuenow")).toBe("25")
    expect(bar.getAttribute("aria-valuetext")).toBe("25% of the goal")
  })

  it("is full above its goal and says the real percentage", () => {
    tile({ kind: "progress", y: "v", def: { goal: 1000 } }, rows([{ v: 1120 }]))
    expect(screen.getByText("112%")).toBeTruthy()
    expect(screen.getByRole("progressbar").getAttribute("aria-valuenow")).toBe("100")
    expect(screen.getByText("Above the goal")).toBeTruthy()
  })

  it("says there is nothing to compare with when the goal is zero or less", () => {
    for (const goal of [0, -3]) {
      cleanup()
      tile({ kind: "progress", y: "v", def: { goal } }, rows([{ v: 40 }]))
      expect(screen.getByText("No goal to compare with")).toBeTruthy()
      expect(screen.queryByRole("progressbar")).toBeNull()
    }
  })

  it("reads a goal that is a second value from the same row, and keeps a small percentage to one decimal", () => {
    tile({ kind: "progress", y: ["v", "g"], def: { goal: { measure: "target" }, measures: ["visitors", "target"] } }, rows([{ v: 9, g: 200 }]))
    expect(screen.getByText("4.5%")).toBeTruthy()
    expect(screen.getByText("9 of 200")).toBeTruthy()
  })

  it("uses the number format of the value", () => {
    tile({ kind: "progress", y: "v", def: { goal: 2000000, format: { style: "compact" } } }, rows([{ v: 500000 }]))
    expect(screen.getByText("500 K of 2 M")).toBeTruthy()
  })
})

describe("a histogram", () => {
  const spec = { kind: "histogram", x: "price", y: "__count", def: { dimension: "price" } }

  it("draws touching bars, labels each range and shows an empty range between two filled ones as zero", () => {
    tile(spec, rows([{ price: 0, __count: 3 }, { price: 2000, __count: 7 }], { bins: { price: { mode: "count", width: 500, min: 0, max: 2400 } } }))
    const option = drawn[0].option as { xAxis: { data: string[] }; series: { data: number[]; barCategoryGap: number }[] }
    expect(option.xAxis.data).toEqual(["0 – 500", "500 – 1,000", "1,000 – 1,500", "1,500 – 2,000", "2,000 – 2,500"])
    expect(option.series[0].data).toEqual([3, 0, 0, 0, 7])
    expect(option.series[0].barCategoryGap).toBe(0)
  })

  it("shows the lower edges as they are when the width is unknown, and fills nothing", () => {
    tile(spec, rows([{ price: 0, __count: 3 }, { price: 2000, __count: 7 }]))
    const option = drawn[0].option as { xAxis: { data: string[] }; series: { data: number[] }[] }
    expect(option.xAxis.data).toEqual(["0", "2,000"])
    expect(option.series[0].data).toEqual([3, 7])
  })

  it("says so instead of drawing a stripe when the width is far too small for the range", () => {
    tile(spec, rows([{ price: 0, __count: 3 }, { price: 100000, __count: 7 }], { bins: { price: { mode: "width", width: 1, min: 0, max: 100000 } } }))
    expect(drawn).toHaveLength(0)
    expect(screen.getByRole("status").textContent).toContain("Too many ranges")
  })

  it("says when the result is as long as the server's cap and may have been cut", () => {
    const full = Array.from({ length: 100 }, (_, i) => ({ price: i * 10, __count: 1 }))
    tile(spec, rows(full, { bins: { price: { mode: "width", width: 10, min: 0, max: 1000 } } }))
    expect(screen.getByText(/Only the first 100 ranges are shown/)).toBeTruthy()
    cleanup()
    tile(spec, rows(full.slice(0, 99), { bins: { price: { mode: "width", width: 10, min: 0, max: 1000 } } }))
    expect(screen.queryByText(/Only the first 100 ranges/)).toBeNull()
  })

  it("is no data without rows", () => {
    tile(spec, rows([]))
    expect(screen.getByText("No data.")).toBeTruthy()
  })

  it("says how many rows have no value, instead of losing them", () => {
    tile(spec, rows([{ price: 0, __count: 3 }, { price: null, __count: 1 }, { price: 2000, __count: 7 }]))
    expect(screen.getByText("1 row has no value")).toBeTruthy()
    cleanup()
    tile(spec, rows([{ price: 0, __count: 3 }, { price: null, __count: 1 }, { price: null, __count: 2 }]))
    expect(screen.getByText("2 rows have no value")).toBeTruthy()
  })
})

describe("a bullet graph", () => {
  const categories = (n: number) => Array.from({ length: n }, (_, i) => ({ province: `P${i + 1}`, visitors: 100 - i, target: 80 }))

  it("draws the value, one fixed target line and three bands in greys, light to dark", () => {
    tile({ kind: "bullet", x: "province", y: "visitors", def: { dimension: "province", goal: 90, bands: [50, 80, 120] } }, rows(categories(3)))
    const option = drawn[0].option as { series: { type: string; markLine?: { data: { xAxis: number }[] }; markArea?: { data: { xAxis: number; itemStyle?: { color: string } }[][] } }[]; yAxis: { data: string[] } }
    expect(option.yAxis.data).toEqual(["P1", "P2", "P3"])
    expect(option.series).toHaveLength(1)
    expect(option.series[0].markLine?.data).toEqual([{ xAxis: 90 }])
    const bands = option.series[0].markArea?.data ?? []
    expect(bands.map(([from, to]) => [from.xAxis, to.xAxis])).toEqual([[0, 50], [50, 80], [80, 120]])
    expect(bands.map(([from]) => from.itemStyle?.color)).toEqual(["#f4f4f5", "#e4e4e7", "#d4d4d8"])
  })

  it("marks a target that is a second value per row", () => {
    tile({ kind: "bullet", x: "province", y: ["visitors", "target"], def: { dimension: "province", goal: { measure: "target" }, bands: [100] } }, rows(categories(2)))
    const option = drawn[0].option as { series: { type: string; data: unknown[]; markLine?: unknown }[] }
    expect(option.series.map((s) => s.type)).toEqual(["bar", "scatter"])
    expect(option.series[1].data).toEqual([[80, 0], [80, 1]])
    expect(option.series[0].markLine).toBeUndefined()
  })

  it("draws twenty and says how many more are not shown", () => {
    tile({ kind: "bullet", x: "province", y: "visitors", def: { dimension: "province", bands: [50] } }, rows(categories(23)))
    const option = drawn[0].option as { yAxis: { data: string[] } }
    expect(option.yAxis.data).toHaveLength(20)
    expect(screen.getByText(/3 more not shown/)).toBeTruthy()
  })

  it("draws one bullet for a single row with no category", () => {
    tile({ kind: "bullet", x: "", y: ["v", "g"], def: { goal: { measure: "target" }, measures: ["visitors", "target"], bands: [10, 20] } }, rows([{ v: 12, g: 18 }]))
    const option = drawn[0].option as { yAxis: { data: string[] }; series: { data: unknown[] }[] }
    expect(option.yAxis.data).toEqual(["visitors"])
    expect(option.series[0].data).toEqual([12])
    expect(option.series[1].data).toEqual([[18, 0]])
  })
})

describe("an image card", () => {
  const spec = { kind: "image", title: "Logo", def: { url: "https://img.example/logo.png", alt: "The company logo" } }

  it("shows the picture with its description and no referrer", () => {
    const { container } = tile(spec, undefined)
    const img = container.querySelector("img")
    expect(img?.getAttribute("src")).toBe("https://img.example/logo.png")
    expect(img?.getAttribute("alt")).toBe("The company logo")
    expect(img?.getAttribute("referrerpolicy")).toBe("no-referrer")
    expect(img?.getAttribute("loading")).toBe("lazy")
  })

  it("says the image could not be loaded without repeating its address", async () => {
    const { container } = tile(spec, undefined)
    const { fireEvent } = await import("@testing-library/react")
    fireEvent.error(container.querySelector("img") as HTMLImageElement)
    expect(screen.getByText("The image could not be loaded")).toBeTruthy()
    expect(container.querySelector("img")).toBeNull()
    expect(container.textContent).not.toContain("img.example")
  })

  it("renders the message and no image when its site was removed from the list", () => {
    const { container } = tile(spec, undefined, ["https://other.example"])
    expect(screen.getByText("This site is no longer allowed")).toBeTruthy()
    expect(container.querySelector("img")).toBeNull()
  })

  it("renders no image for an address that is not https or hides another host", () => {
    for (const url of ["http://img.example/logo.png", "https://img.example@evil.example/logo.png", "javascript:alert(1)"]) {
      cleanup()
      const { container } = tile({ ...spec, def: { url, alt: "x" } }, undefined)
      expect(container.querySelector("img"), url).toBeNull()
    }
  })
})

describe("a web-page card", () => {
  const spec = { kind: "web", title: "Status page", def: { url: "https://img.example/status" } }

  it("frames the page in a sandbox with scripts and pop-ups only, no referrer, and always offers a new tab", () => {
    const { container } = tile(spec, undefined)
    const frame = container.querySelector("iframe")
    expect(frame?.getAttribute("src")).toBe("https://img.example/status")
    expect(frame?.getAttribute("sandbox")).toBe("allow-scripts allow-popups")
    expect(frame?.getAttribute("sandbox")).not.toContain("allow-same-origin")
    expect(frame?.getAttribute("sandbox")).not.toContain("allow-top-navigation")
    expect(frame?.getAttribute("referrerpolicy")).toBe("no-referrer")
    expect(frame?.getAttribute("title")).toBe("Status page")
    const link = screen.getByRole("link", { name: "Open in a new tab" })
    expect(link.getAttribute("target")).toBe("_blank")
    expect(link.getAttribute("rel")).toBe("noopener noreferrer")
    expect(link.getAttribute("href")).toBe("https://img.example/status")
  })

  it("renders the message and no frame, no link, when its site was removed from the list", () => {
    const { container } = tile(spec, undefined, [])
    expect(screen.getByText("This site is no longer allowed")).toBeTruthy()
    expect(container.querySelector("iframe")).toBeNull()
    expect(screen.queryByRole("link")).toBeNull()
  })

  it("is not given a page on another port or scheme of an allowed host", () => {
    for (const url of ["https://img.example:8443/status", "http://img.example/status"]) {
      cleanup()
      const { container } = tile({ ...spec, def: { url } }, undefined)
      expect(container.querySelector("iframe"), url).toBeNull()
    }
  })
})

describe("the reporting context", () => {
  it("carries the content sites of the payload, and none when the payload has none", async () => {
    const { useReporting } = await import("./reporting-context")
    function Probe() {
      return <p data-testid="sites">{JSON.stringify(useReporting().contentSites)}</p>
    }
    const sitesOf = (reporting: Parameters<typeof ReportingProvider>[0]["reporting"]) => {
      const { unmount } = render(<ReportingProvider reporting={reporting}><Probe /></ReportingProvider>)
      const text = screen.getByTestId("sites").textContent
      unmount()
      return text
    }
    expect(sitesOf({ timeZone: "UTC", weekStart: "monday", contentSites: ["https://a.example", "https://b.example:8443"] })).toBe('["https://a.example","https://b.example:8443"]')
    expect(sitesOf({ timeZone: "UTC", weekStart: "monday" })).toBe("[]")
    expect(sitesOf(undefined)).toBe("[]")
  })
})
