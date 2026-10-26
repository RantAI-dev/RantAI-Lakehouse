import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ChartRenderSpec } from "@/lib/dashboard-specs"
import type { Rows } from "./tile-dialogs"

// BI-17 part B, T4 (D5): a scatter or bubble tile colours its points by cluster.
const drawn: { option: { series?: { name?: string; data?: unknown[] }[]; legend?: unknown } }[] = []
mock.module("./echart", () => ({
  EChart: (props: (typeof drawn)[number]) => {
    drawn.push(props)
    return <div data-testid="chart" />
  },
}))
const { TileBody } = await import("./tile-body")

afterEach(() => { cleanup(); drawn.length = 0 })

const spec = (kind: "scatter" | "bubble", y: string[], clusters?: unknown): ChartRenderSpec =>
  ({ id: "c", title: "Stores", kind, mart: "m", x: "store", y, source: "ui", def: clusters ? { clusters } : undefined }) as unknown as ChartRenderSpec

/** Three tight blobs of five stores each. */
const rows = [[0, 0], [10, 0], [5, 9]].flatMap(([cx, cy], b) =>
  Array.from({ length: 5 }, (_, i) => ({ store: `S${b}${i}`, sales: cx + (i % 3) * 0.1, visits: cy + (i % 2) * 0.1, size: 10 + b })))
const cell = { columns: ["store", "sales", "visits", "size"], rows } as unknown as Rows

describe("a scatter tile with clusters", () => {
  it("draws one series per cluster, named with its size and centre, and says how many points were used", () => {
    render(<TileBody spec={spec("scatter", ["sales", "visits"], { k: "auto" })} cell={cell} dark={false} loading={false} />)
    const option = drawn[drawn.length - 1].option
    expect(option.series?.length).toBe(3)
    expect(option.series?.every((s) => /^Cluster \d \(n = 5; x̄ = .+, ȳ = .+\)$/.test(s.name ?? ""))).toBe(true)
    expect(option.series?.reduce((a, s) => a + (s.data?.length ?? 0), 0)).toBe(15)
    expect(option.legend).toBeTruthy()
    expect(screen.getByText("Clusters of the 15 points shown; 3 groups chosen automatically.")).toBeTruthy()
  })

  it("honours a fixed number of groups", () => {
    render(<TileBody spec={spec("scatter", ["sales", "visits"], { k: 2 })} cell={cell} dark={false} loading={false} />)
    expect(drawn[drawn.length - 1].option.series?.length).toBe(2)
    expect(screen.getByText("Clusters of the 15 points shown.")).toBeTruthy()
  })

  it("groups a bubble chart on its three measures", () => {
    render(<TileBody spec={spec("bubble", ["sales", "visits", "size"], { k: 3 })} cell={cell} dark={false} loading={false} />)
    expect(drawn[drawn.length - 1].option.series?.[0].name).toMatch(/z̄ = /)
  })

  it("says why there are no clusters and draws the points as they were (refusal, not a guess)", () => {
    const few = { columns: cell.columns, rows: rows.slice(0, 4) } as unknown as Rows
    render(<TileBody spec={spec("scatter", ["sales", "visits"], { k: "auto" })} cell={few} dark={false} loading={false} />)
    const option = drawn[drawn.length - 1].option
    expect(option.series?.length).toBe(1)
    expect(option.series?.[0].name).toBeUndefined()
    expect(screen.getByText("Clusters need at least 6 points; this chart has 4.")).toBeTruthy()
  })

  it("leaves a scatter tile without the setting as it was", () => {
    render(<TileBody spec={spec("scatter", ["sales", "visits"])} cell={cell} dark={false} loading={false} />)
    const option = drawn[drawn.length - 1].option
    expect(option.series?.length).toBe(1)
    expect(option.legend).toBeUndefined()
    expect(screen.queryByText(/Clusters/)).toBeNull()
  })
})
