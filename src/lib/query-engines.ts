import type { QueryEngine } from "@/services/contracts/queries"

/**
 * The engines a person may choose in Query Studio (`BI-44`).
 *
 * A `bi-only` installation deploys exactly ONE engine, chosen at install
 * time and reported by `GET /api/settings/deployment`; offering the other
 * one would invite a query the API must refuse with a 400. An absent or
 * unknown mode keeps the two historical choices untouched — the console
 * never guesses that an older server installed only one engine. In
 * `bi-only` an absent or unrecognised engine name falls back to
 * `clickhouse`, the API's own default, rather than inventing an engine.
 *
 * Pure by design: the caller supplies the deployment answer, so this can
 * be tested without a session or a fetch.
 */
export function availableQueryEngines(
  installMode?: string,
  queryEngine?: string
): QueryEngine[] {
  if (installMode !== "bi-only") return ["clickhouse", "trino"]
  return queryEngine === "trino" ? ["trino"] : ["clickhouse"]
}
