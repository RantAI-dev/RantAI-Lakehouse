/**
 * The honest message a connector's detail panel shows in place of a
 * working backfill/initial-snapshot trigger.
 *
 * There is no such trigger for any Debezium-fronted CDC source in this
 * build: Debezium's own `snapshot.mode=initial` runs the initial
 * snapshot automatically the moment the connector's own
 * `debezium-<id>` compose service starts (see
 * `docs/adr/0008-initial-snapshot-backfill.md`). The API's own
 * `POST /api/connectors/{id}/ingest/run` response already says this,
 * per connector, once that route is actually called for a
 * `cdc`-adapter connector (its `reason` field is the authoritative,
 * connector-specific wording). This function exists only to say the
 * same thing, generically and truthfully, so the panel does not need a
 * click against a mutating route before it is honest about the fact
 * that nothing here can be triggered — it is never substituted for the
 * API's own `reason` once that response has actually arrived.
 *
 * `driver` is intentionally a plain `string`, not a closed union: the
 * connector's real driver value flows from `CdcDial["driver"]` /
 * `Connector["type"]`, both of which already widen with `| string`
 * elsewhere in `@/services/contracts/connectors` for the same reason —
 * this function must not lie about a driver it has never heard of by
 * refusing to render a message for it.
 */
export function backfillTriggerMessage(driver: string): string {
  // ADR 0008's Oracle addendum: Oracle CDC has no LogMiner template in
  // this build at all, so it must never be told the generic Debezium
  // story above, which would wrongly imply an initial snapshot runs for
  // it automatically. Oracle ingestion runs through the batch sql
  // adapter instead, and has its own honest `supported: false` refusal
  // at `GET /api/connectors/{id}/debezium-properties`.
  if (driver.toLowerCase() === "oracle") {
    return (
      "Oracle CDC is not supported in this build (docs/adr/0008-initial-snapshot-backfill.md, " +
      "Oracle addendum): there is no Debezium LogMiner connector template here, so there is no " +
      "initial-snapshot mechanism for this connector to trigger, automatic or otherwise. Oracle " +
      "ingestion runs through the batch sql adapter instead."
    )
  }

  return (
    "CDC ingestion has no separate backfill/initial-snapshot trigger: Debezium's own " +
    "snapshot.mode=initial (docs/adr/0008-initial-snapshot-backfill.md) runs the initial " +
    `snapshot automatically the moment this ${driver} connector's own debezium compose service ` +
    "starts, before streaming picks up from there. Bring that service up to start ingestion; " +
    "there is nothing on this page that can trigger it."
  )
}
