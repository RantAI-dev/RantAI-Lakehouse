import { describe, expect, it } from "bun:test"
import { runDiscovery } from "./run-discovery"
import type { DiscoveryAnswer } from "@/services/contracts/connectors"

const done: DiscoveryAnswer = { kind: "list", state: "done", objects: [{ name: "orders", kind: "collection" }] }

describe("runDiscovery (SRC-13 decisions D1 and D9)", () => {
  it("returns an inline answer without polling", async () => {
    let polled = 0
    const answer = await runDiscovery({
      kind: "list",
      start: async () => done,
      get: async () => {
        polled += 1
        return done
      },
    })
    expect(answer).toEqual(done)
    expect(polled).toBe(0)
  })

  it("polls a running request until it ends and reports the wait", async () => {
    const answers: DiscoveryAnswer[] = [
      { kind: "list", state: "running" },
      { kind: "list", state: "running" },
      done,
    ]
    const waits: number[] = []
    let clock = 0
    const answer = await runDiscovery({
      kind: "list",
      start: async () => ({ kind: "list", state: "running" }),
      get: async () => answers.shift() as DiscoveryAnswer,
      pollMs: 1,
      now: () => (clock += 1000),
      onWaiting: (ms) => waits.push(ms),
    })
    expect(answer).toEqual(done)
    expect(waits.length).toBe(3)
    expect(waits).toEqual([1000, 3000, 5000])
  })

  it("does not show another object's preview as the answer to this one", async () => {
    const answer = await runDiscovery({
      kind: "preview",
      start: async () => ({ kind: "preview", state: "running", object: "orders" }),
      get: async () => ({ kind: "preview", state: "done", object: "customers", columns: [] }),
      pollMs: 1,
    })
    expect(answer.state).toBe("failed")
    expect(answer.columns).toBeUndefined()
    expect(answer.reason).toMatch(/replaced this one/)
  })

  it("gives up with a timeout of its own words when the request never ends", async () => {
    let clock = 0
    const answer = await runDiscovery({
      kind: "list",
      start: async () => ({ kind: "list", state: "running" }),
      get: async () => ({ kind: "list", state: "running" }),
      pollMs: 1,
      maxWaitMs: 5000,
      now: () => (clock += 3000),
    })
    expect(answer.state).toBe("failed")
    expect(answer.errorClass).toBe("timeout")
  })

  it("stops waiting when it is aborted", async () => {
    const controller = new AbortController()
    const run = runDiscovery({
      kind: "list",
      start: async () => ({ kind: "list", state: "running" }),
      get: async () => ({ kind: "list", state: "running" }),
      pollMs: 60_000,
      signal: controller.signal,
    })
    setTimeout(() => controller.abort(), 5)
    await expect(run).rejects.toMatchObject({ name: "AbortError" })
  })
})
