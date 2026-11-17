import { describe, expect, it } from "bun:test"
import type { ConnectorType, DiscoveryAnswer, IngestRunResult } from "@/services/contracts/connectors"

describe("ingest-tier1 contracts", () => {
  it("ConnectorType round-trips a planned type with no adapter", () => {
    const fixture: ConnectorType = { name: "Kafka", adapter: null, supported: false, docsUrl: null, unsupportedReason: null }
    expect(fixture.supported).toBe(false)
  })
  it("IngestRunResult represents CDC's honest unsupported shape without a runId", () => {
    const fixture: IngestRunResult = { supported: false, reason: "CDC ingestion has no separate trigger" }
    expect(fixture.runId).toBeUndefined()
  })
  it("DiscoveryAnswer represents a refused listing by its class and fixed reason, never fabricated objects or columns", () => {
    const fixture: DiscoveryAnswer = {
      kind: "list",
      state: "failed",
      errorClass: "not_supported",
      reason: "Listing is not available for this connector type.",
    }
    expect(fixture.objects).toBeUndefined()
    expect(fixture.columns).toBeUndefined()
  })
  it("DiscoveryAnswer keeps an empty preview apart from a failed one", () => {
    const fixture: DiscoveryAnswer = { kind: "preview", state: "done", object: "orders", columns: [] }
    expect(fixture.errorClass).toBeUndefined()
  })
})
