import { act, cleanup, render } from "@testing-library/react"
import * as React from "react"
import { afterEach, describe, expect, it, mock } from "bun:test"

mock.module("next/navigation", () => ({ usePathname: () => "/dashboards/x" }))
const { CopilotProvider, useCopilot, useCopilotSelect } = await import("./use-copilot")
afterEach(cleanup)

// BI-32 review: a page that reads only a stable part of the assistant is not
// re-rendered when the rest of it changes (a streamed answer changes `messages`, `busy`…).
describe("useCopilotSelect", () => {
  it("re-renders only when the selected value changes", () => {
    let narrow = 0
    let wide = 0
    let mode = ""
    const box: { setMode: (m: "ask" | "build") => void } = { setMode: () => {} }
    function Narrow() { narrow++; useCopilotSelect((c) => c.setMode); return null }
    function Mode() { wide++; mode = useCopilotSelect((c) => c.mode); return null }
    function Driver() { const c = useCopilot(); React.useEffect(() => { box.setMode = c.setMode }); return null }
    render(<CopilotProvider><Narrow /><Mode /><Driver /></CopilotProvider>)
    const n0 = narrow
    act(() => box.setMode("build"))
    act(() => box.setMode("ask"))
    act(() => box.setMode("build"))
    expect(mode).toBe("build")
    expect(wide).toBeGreaterThan(1)
    expect(narrow).toBe(n0)
  })
})
