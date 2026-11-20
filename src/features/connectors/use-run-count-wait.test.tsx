import { act, cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, mock } from "bun:test"
import type { ConnectorDetail, ConnectorRun } from "@/services/contracts/connectors"
import { useRunCountWait } from "./use-run-count-wait"

// SRC-46 (QA LH-SRC-6-43), review round 3, BLOCKER 3: real timers with tiny values and no fetch (`reload` is a mock,
// the detail changes by `rerender`); fake timers with stubbed fetch hung on CI.
afterEach(cleanup)

const POLL = 20
const MAX = 200
const sleep = (ms: number) => act(async () => void (await new Promise((r) => setTimeout(r, ms))))

const detail = (extra: Partial<ConnectorDetail> = {}) =>
  ({ lastRunSuccessAt: null, lastRunFailureAt: null, ...extra }) as ConnectorDetail
const run = (status: string) => ({ runId: "run-live-00001", status }) as ConnectorRun

function setup() {
  const reload = mock(() => {})
  const hook = renderHook(({ d }: { d: ConnectorDetail }) => useRunCountWait(d, reload, { pollMs: POLL, maxMs: MAX }), {
    initialProps: { d: detail() },
  })
  return { reload, ...hook }
}

describe("useRunCountWait", () => {
  it("reads at once, then every poll, until the failure is counted, then stops", async () => {
    const { reload, result, rerender } = setup()
    act(() => result.current(run("failed")))
    expect(reload).toHaveBeenCalledTimes(1)
    // Not counted yet: the detail still has no failure time.
    await sleep(POLL * 3 + 10)
    expect(reload.mock.calls.length).toBeGreaterThan(2)

    rerender({ d: detail({ lastRunFailureAt: "2026-11-19T08:28:40.000Z" }) })
    await sleep(10)
    const atCount = reload.mock.calls.length
    await sleep(POLL * 4)
    expect(reload.mock.calls.length).toBe(atCount)
  })

  it("counts a succeeded run by lastRunSuccessAt", async () => {
    const { reload, result, rerender } = setup()
    act(() => result.current(run("succeeded")))
    // A failure time changing is not this run's outcome.
    rerender({ d: detail({ lastRunFailureAt: "2026-11-19T08:28:40.000Z" }) })
    await sleep(POLL * 3 + 10)
    expect(reload.mock.calls.length).toBeGreaterThan(2)
    rerender({ d: detail({ lastRunSuccessAt: "2026-11-19T08:34:13.000Z" }) })
    await sleep(10)
    const atCount = reload.mock.calls.length
    await sleep(POLL * 4)
    expect(reload.mock.calls.length).toBe(atCount)
  })

  it("stops asking at the cap when the run is never counted", async () => {
    const { reload, result } = setup()
    act(() => result.current(run("failed")))
    await sleep(MAX + POLL * 3)
    const atCap = reload.mock.calls.length
    expect(atCap).toBeGreaterThan(3)
    await sleep(POLL * 5)
    expect(reload.mock.calls.length).toBe(atCap)
  })

  it("reads once, and does not poll, for a paused run", async () => {
    const { reload, result } = setup()
    act(() => result.current(run("paused")))
    await sleep(POLL * 6)
    expect(reload).toHaveBeenCalledTimes(1)
  })

  it("reads nothing after unmount", async () => {
    const { reload, result, unmount } = setup()
    act(() => result.current(run("failed")))
    await sleep(POLL * 2 + 10)
    unmount()
    const atLeave = reload.mock.calls.length
    await sleep(POLL * 5)
    expect(reload.mock.calls.length).toBe(atLeave)
  })
})
