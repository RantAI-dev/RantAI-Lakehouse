import { describe, expect, it } from "bun:test"
import type { ConnectorType, DiscoverResult, IngestRunResult } from "@/services/contracts/connectors"

describe("ingest-tier1 contracts", () => {
  it("ConnectorType round-trips a planned type with no adapter", () => {
    const fixture: ConnectorType = { name: "Kafka", adapter: null, supported: false, docsUrl: null }
    expect(fixture.supported).toBe(false)
  })
  it("IngestRunResult represents CDC's honest unsupported shape without a runId", () => {
    const fixture: IngestRunResult = { supported: false, reason: "CDC ingestion has no separate trigger" }
    expect(fixture.runId).toBeUndefined()
  })
  it("DiscoverResult represents an unsupported adapter with a reason, never fabricated columns", () => {
    const fixture: DiscoverResult = { objects: [], supported: false, reason: "no service account configured" }
    expect(fixture.objects).toEqual([])
  })
})
