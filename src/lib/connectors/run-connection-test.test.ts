import { describe, expect, it } from "bun:test"
import { runConnectionTest } from "./run-connection-test"
import type { ConnectorTestResult, DiscoveryAnswer, ProbeHistoryResponse } from "@/services/contracts/connectors"

const PENDING: ConnectorTestResult = {
  ok: false,
  supported: true,
  latencyMs: null,
  message: "The connection test is running in the orchestrator.",
  testedAt: null,
  pending: true,
}

function service(first: ConnectorTestResult, polls: DiscoveryAnswer[], history: ProbeHistoryResponse) {
  const calls: string[] = []
  return {
    calls,
    testConnection: async () => {
      calls.push("test")
      return first
    },
    getDiscovery: async (_id: string, kind: string) => {
      calls.push(`poll ${kind}`)
      return polls.shift() as DiscoveryAnswer
    },
    listProbeHistory: async () => {
      calls.push("history")
      return history
    },
  }
}

describe("runConnectionTest (SRC-4 T1.1)", () => {
  it("returns a test answered in the request as it is, without polling", async () => {
    const answer: ConnectorTestResult = { ok: true, supported: true, latencyMs: 12, message: "Connected.", testedAt: "t" }
    const s = service(answer, [], { results: [] })
    expect(await runConnectionTest(s, "c")).toEqual(answer)
    expect(s.calls).toEqual(["test"])
  })

  it("waits on the test row, then returns the probe the API recorded for this test", async () => {
    const s = service(
      PENDING,
      [
        { kind: "test", state: "running" },
        { kind: "test", state: "done", latencyMs: 212, startedAt: "2026-10-29T10:00:00Z", endedAt: "2026-10-29T10:00:05Z" },
      ],
      {
        results: [
          { testedAt: "2026-10-29T10:00:05Z", ok: true, latencyMs: 212, message: "Connected via Oracle and ran SELECT 1 FROM DUAL." },
        ],
      }
    )
    const result = await runConnectionTest(s, "c", undefined, 1)
    expect(s.calls).toEqual(["test", "poll test", "poll test", "history"])
    expect(result).toEqual({
      ok: true,
      supported: true,
      latencyMs: 212,
      message: "Connected via Oracle and ran SELECT 1 FROM DUAL.",
      testedAt: "2026-10-29T10:00:05Z",
    })
  })

  it("shows the API's reason, with no latency, for a test whose job never reported (an older probe is not its result)", async () => {
    const s = service(
      PENDING,
      [
        {
          kind: "test",
          state: "failed",
          errorClass: "timeout",
          reason: "The source did not answer in time.",
          startedAt: "2026-10-29T10:00:00Z",
        },
      ],
      { results: [{ testedAt: "2026-10-28T09:00:00Z", ok: true, latencyMs: 5, message: "Connected via Oracle and ran SELECT 1 FROM DUAL." }] }
    )
    const result = await runConnectionTest(s, "c", undefined, 1)
    expect(result).toEqual({
      ok: false,
      supported: true,
      latencyMs: null,
      message: "The source did not answer in time.",
      testedAt: null,
    })
  })
  it("carries the setup checks of the finished test to the answer shown (MongoDB change capture, S8-e)", async () => {
    const checks = [
      { setting: "replicaSet", status: "missing", message: "The server's replica set has another name. Use the name in `rs.status().set`." },
      { setting: "primary", status: "ok", message: "This host is the primary." },
    ]
    const s = service(
      PENDING,
      [
        {
          kind: "test",
          state: "done",
          latencyMs: 30,
          startedAt: "2026-10-30T10:00:00Z",
          endedAt: "2026-10-30T10:00:05Z",
          setupChecks: checks,
        },
      ],
      { results: [{ testedAt: "2026-10-30T10:00:05Z", ok: true, latencyMs: 30, message: "Connected via MongoDB and ran ping." }] }
    )
    const result = await runConnectionTest(s, "c", undefined, 1)
    expect(result.ok).toBe(true) // the checks never change ok
    expect(result.setupChecks).toEqual(checks)
    // No checks, no key.
    const plain = service(
      PENDING,
      [{ kind: "test", state: "done", latencyMs: 30, startedAt: "2026-10-30T10:00:00Z" }],
      { results: [{ testedAt: "2026-10-30T10:00:05Z", ok: true, latencyMs: 30, message: "Connected." }] }
    )
    expect("setupChecks" in (await runConnectionTest(plain, "c", undefined, 1))).toBe(false)
  })
})
