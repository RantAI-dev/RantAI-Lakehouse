import type { IngestSpec, IngestSpecInput } from "@/services/contracts/connectors"

/**
 * The ingest-spec save that makes `topic` the topic a Kafka connector reads
 * (`SRC-13` decision D4: picking a topic from the broker's list writes the dial's
 * `topic`; one topic per connector).
 *
 * It is the existing spec save (`PUT .../ingest-spec`), with the stored dial and
 * everything else of the stored spec sent back unchanged, so the topic is the only
 * thing the save changes. It carries no credential on purpose: `SEC-14`'s re-point
 * rule (`Dial::target_identity`, `rust/crates/lakehouse-store/src/ingest_spec.rs`)
 * compares a Kafka connector's `bootstrapServers` only, so a topic change is NOT a
 * change of target and the server accepts it without the credentials entered again.
 * `target-identity.test.ts` and the store test
 * `a_kafka_dial_is_re_pointed_by_its_bootstrap_servers_only` pin that; if the rule
 * ever starts to include the topic, this save would answer 409 and the console would
 * have to ask for the credentials, which is why the test for this function asserts
 * it with the console's own copy of the rule too.
 */
export function specWithTopic(spec: IngestSpec, topic: string): IngestSpecInput {
  return {
    // The caller only offers topics for a saved Kafka connector, whose spec has both.
    adapter: spec.adapter ?? "kafka",
    ingestMode: spec.ingestMode ?? "batch",
    dial: { ...spec.dial, topic },
    sourceObjects: spec.sourceObjects,
    ...(spec.scheduleCron ? { scheduleCron: spec.scheduleCron } : {}),
  }
}
