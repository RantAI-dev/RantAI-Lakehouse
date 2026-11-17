import { describe, expect, it } from "bun:test"
import {
  NOT_MEASURED_REASON,
  consumerLine,
  consumerState,
  consumerTooltip,
} from "./gold-consumers"
import type { GoldConsumer, GoldConsumers } from "@/services/contracts/gold"

const NOW = Date.parse("2026-10-31T12:00:00Z")

const reader: GoldConsumer = {
  identityId: "11111111-1111-1111-1111-111111111111",
  name: "Finance DuckDB",
  reads: 4,
  lastReadAt: "2026-10-31T10:00:00Z",
}

function answer(over: Partial<GoldConsumers>): GoldConsumers {
  return {
    mart: "sales",
    supported: true,
    windowDays: 7,
    consumers: [],
    ...over,
  }
}

describe("consumerState", () => {
  it("reports not_measured with the route's reason when outside reading is off", () => {
    const state = consumerState(
      answer({
        supported: false,
        consumers: null,
        reason: "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
      }),
    )
    expect(state).toEqual({
      kind: "not_measured",
      reason: "Outside reading is not set up: OUTSIDE_S3_ENDPOINT is not set.",
    })
  })

  it("names the missing setup when the route refuses without a reason", () => {
    const state = consumerState(answer({ supported: false, consumers: null }))
    expect(state).toEqual({ kind: "not_measured", reason: NOT_MEASURED_REASON })
  })

  it("reports no_reads for an empty list, which is a measured zero", () => {
    expect(consumerState(answer({ consumers: [] }))).toEqual({ kind: "no_reads" })
  })

  it("reports readers with the tools the route counted", () => {
    const state = consumerState(answer({ consumers: [reader] }))
    expect(state).toEqual({ kind: "readers", readers: [reader] })
  })

  it("reports unknown, never zero, for a null list under a supported route", () => {
    expect(consumerState(answer({ consumers: null }))).toEqual({ kind: "unknown" })
  })
})

describe("consumerLine", () => {
  it("names the tool, its reads and when it last read", () => {
    expect(consumerLine(reader, NOW)).toBe("Finance DuckDB: 4 reads, last 2h ago")
  })
})

describe("consumerTooltip", () => {
  it("puts one line per tool", () => {
    const second: GoldConsumer = {
      identityId: "22222222-2222-2222-2222-222222222222",
      name: "Analytics",
      reads: 1,
      lastReadAt: "2026-10-28T12:00:00Z",
    }
    expect(consumerTooltip([reader, second], NOW)).toBe(
      "Finance DuckDB: 4 reads, last 2h ago\nAnalytics: 1 reads, last 3d ago",
    )
  })
})
