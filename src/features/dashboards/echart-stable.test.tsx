import { cleanup, render } from "@testing-library/react"
import { afterAll, afterEach, describe, expect, it, mock } from "bun:test"
import * as React from "react"

// BI-32: a chart stays still while the assistant answers. The page re-renders
// with each streamed message and hands the chart an equal option with a new
// identity; it must not be applied again (that replays the reference line's entrance).
const real = await import("echarts")
const setOption = mock(() => {})
const fake = { setOption, on: () => {}, resize: () => {}, dispose: () => {}, getOption: () => ({}) }
mock.module("echarts", () => ({ ...real, init: () => fake }))
const { EChart } = await import("./echart")

;(globalThis as { ResizeObserver?: unknown }).ResizeObserver ??= class { observe() {} disconnect() {} unobserve() {} }
afterEach(() => { cleanup(); setOption.mockClear() })
afterAll(() => { mock.module("echarts", () => real) })

const build = (target: number) => ({
  series: [{ type: "bar", data: [1, 2], markLine: { data: [{ yAxis: target, label: { formatter: (p: { value: number }) => `Target ${p.value}` } }] } }],
})

function Tile({ target, tick }: { target: number; tick: number }) {
  return <div data-tick={tick}><EChart option={build(target) as never} /></div>
}

describe("EChart applies an option only when it changed", () => {
  it("does not re-apply an equal option rebuilt on every render", () => {
    const { rerender } = render(<Tile target={1500000} tick={0} />)
    expect(setOption).toHaveBeenCalledTimes(1)
    for (let i = 1; i <= 5; i++) rerender(<Tile target={1500000} tick={i} />)
    expect(setOption).toHaveBeenCalledTimes(1)
  })

  it("applies again when the chart really changes", () => {
    const { rerender } = render(<Tile target={1500000} tick={0} />)
    rerender(<Tile target={2000000} tick={1} />)
    expect(setOption).toHaveBeenCalledTimes(2)
  })
})
