import { describe, expect, it } from "bun:test"
import { specWithTopic } from "./kafka-topic"
import { targetChanged } from "./target-identity"
import type { IngestSpec } from "@/services/contracts/connectors"

const spec: IngestSpec = {
  adapter: "kafka",
  ingestMode: "batch",
  dial: {
    bootstrapServers: ["k1.internal:9092"],
    topic: "orders",
    auth: { type: "none" },
    groupId: "g",
    microBatchSeconds: 30,
  },
  sourceObjects: [{ name: "orders", target: "kafka_orders" }],
  scheduleCron: "0 * * * *",
  secretRefs: { primary: "file:/run/secrets/k", secondary: null },
  nextRunAt: null,
}

describe("specWithTopic (SRC-13 decision D4)", () => {
  it("changes the topic and nothing else of the stored spec", () => {
    const input = specWithTopic(spec, "payments")
    expect(input.dial).toEqual({ ...spec.dial, topic: "payments" })
    expect(input.adapter).toBe("kafka")
    expect(input.ingestMode).toBe("batch")
    expect(input.sourceObjects).toEqual(spec.sourceObjects)
    expect(input.scheduleCron).toBe("0 * * * *")
    expect("credential" in input).toBe(false)
  })

  it("saves a spec with no stored ingest mode as a stream, never a batch (SRC-45)", () => {
    expect(specWithTopic({ ...spec, ingestMode: null }, "payments").ingestMode).toBe("stream")
    expect(specWithTopic({ ...spec, adapter: null, ingestMode: null }, "payments").ingestMode).toBe("stream")
  })

  it("is not a change of target, so the save needs no credentials (SEC-14)", () => {
    const input = specWithTopic(spec, "payments")
    expect(
      targetChanged({ storedAdapter: spec.adapter, storedDial: spec.dial, adapter: input.adapter, dial: input.dial })
    ).toBe(false)
  })

  it("leaves the schedule out when there is none", () => {
    expect("scheduleCron" in specWithTopic({ ...spec, scheduleCron: null }, "payments")).toBe(false)
  })
})
