import { afterEach, describe, expect, it, mock } from "bun:test"
import { cleanup, render } from "@testing-library/react"

let pathname = "/dashboards"
const seen: Array<Record<string, unknown>> = []

mock.module("next/navigation", () => ({
  usePathname: () => pathname,
}))
mock.module("@rantai/design-system/components/theme-provider", () => ({
  ThemeProvider: ({ children, ...props }: { children?: React.ReactNode }) => {
    seen.push(props)
    return <>{children}</>
  },
}))

const { ConsoleThemeProvider, isPrintViewPath } = await import("./console-theme-provider")

afterEach(() => {
  cleanup()
  seen.length = 0
})

describe("ConsoleThemeProvider (RPT-3)", () => {
  it("forces the light theme on the print views", () => {
    pathname = "/render/report/x"
    render(<ConsoleThemeProvider><p>x</p></ConsoleThemeProvider>)
    expect(seen.at(-1)?.forcedTheme).toBe("light")
  })

  it("forces nothing elsewhere, so the console keeps its default and the person's choice", () => {
    pathname = "/dashboards"
    render(<ConsoleThemeProvider><p>x</p></ConsoleThemeProvider>)
    expect(seen.at(-1)?.forcedTheme).toBeUndefined()
  })

  it("matches /render and its subpaths only", () => {
    expect(isPrintViewPath("/render/dashboard/t")).toBe(true)
    expect(isPrintViewPath("/render/chart/t")).toBe(true)
    expect(isPrintViewPath("/rendering")).toBe(false)
    expect(isPrintViewPath(null)).toBe(false)
  })
})
