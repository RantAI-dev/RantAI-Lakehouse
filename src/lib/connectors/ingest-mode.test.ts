import { describe, expect, it } from "bun:test"
import { ingestModeFor } from "./ingest-mode"

// SRC-44 (QA LH-SRC-6-41): the loader runs a Kafka connector only as a stream.
describe("ingestModeFor", () => {
  it("runs a Kafka connector as a stream", () => {
    expect(ingestModeFor("kafka")).toBe("stream")
  })

  it("runs a change-capture connector as cdc", () => {
    expect(ingestModeFor("cdc")).toBe("cdc")
  })

  it("runs every other adapter, and an unknown one, as a batch", () => {
    expect(ingestModeFor("sql")).toBe("batch")
    expect(ingestModeFor("rest")).toBe("batch")
    expect(ingestModeFor(undefined)).toBe("batch")
    expect(ingestModeFor(null)).toBe("batch")
  })
})
