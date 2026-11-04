/**
 * The honest message a connector's detail panel shows in place of a
 * working backfill/initial-snapshot trigger.
 *
 * There is no such trigger for a change-capture source: Debezium's own
 * `snapshot.mode=initial` copies each table once when the connector starts in
 * Kafka Connect (`SRC-4`, `docs/adr/0016-change-capture-per-connector-on-kafka-connect.md`;
 * the earlier reasoning is in `docs/adr/0008-initial-snapshot-backfill.md`) and
 * then follows each change. The API's own `POST /api/connectors/{id}/ingest/run`
 * answer says this per connector (its `reason` is the authoritative wording);
 * this function says the same thing generically so the panel is honest before any
 * request is made. It is never substituted for the API's own `reason`.
 *
 * `driver` is intentionally a plain `string`, not a closed union: the
 * connector's real driver value flows from `CdcDial["driver"]` /
 * `Connector["type"]`, both of which already widen with `| string`
 * elsewhere in `@/services/contracts/connectors` for the same reason —
 * this function must not lie about a driver it has never heard of by
 * refusing to render a message for it.
 */
export function backfillTriggerMessage(driver: string): string {
  // ADR 0008's Oracle addendum: Oracle change capture is not built (`SRC-4` slice 6),
  // so it must never be told the generic story below, which would wrongly imply a
  // first copy runs for it. Oracle ingestion runs through the batch sql adapter.
  if (driver.toLowerCase() === "oracle") {
    return (
      "Oracle change capture is not supported in this build (docs/adr/0008-initial-snapshot-backfill.md, " +
      "Oracle addendum): there is no Debezium LogMiner connector here, so there is no " +
      "first copy for this connector to take. Oracle " +
      "ingestion runs through the batch sql adapter instead."
    )
  }

  return (
    "Change capture has no separate backfill button: when capture starts, each selected table is " +
    `copied once (Debezium's snapshot.mode=initial, docs/adr/0016-change-capture-per-connector-on-kafka-connect.md) ` +
    `and then every change at the ${driver} source is followed. Save the tables to start it.`
  )
}
