/**
 * Reading the catalog's refusal out of an asset detail.
 *
 * The tenant gate answers `GET /api/catalog/{id}` with `200` and
 * `{ "supported": false, "reason": "…" }` and no `schema`, so a caller that
 * reads `detail.schema` as if the request had succeeded throws on a refusal.
 * The contract's `AssetDetail` has no such field, which is why the body is
 * read as `unknown` here.
 */

/** What the caller shows when the refusal carries no reason of its own. */
const NO_REASON = "The catalog did not return this table's columns."

/**
 * The refusal the body holds, or `null` for a body that is a normal detail.
 *
 * Never throws: it is called on every detail, refused or not.
 */
export function catalogRefusal(detail: unknown): { reason: string } | null {
  if (typeof detail !== "object" || detail === null) return null
  const body = detail as { supported?: unknown; reason?: unknown }
  if (body.supported !== false) return null
  return { reason: typeof body.reason === "string" && body.reason !== "" ? body.reason : NO_REASON }
}
