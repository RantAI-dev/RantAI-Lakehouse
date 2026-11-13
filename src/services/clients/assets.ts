import type {
  AssetService,
  Asset,
  AssetDetail,
  AssetGrant,
  AssetRestoreStatus,
  AssetDropStatus,
  AssetDropResult,
  DroppedTableEntry,
  AssetProfile,
  AssetFilter,
  CatalogNamespace,
  DecideAccessRequestResult,
  RequestAccessInput,
} from "../contracts/assets";
import type { Pagination } from "../contracts/pagination";
import { apiFetch } from "../http";
import { ServiceError, deniedFor } from "../errors";

/**
 * Maps an HTTP status to the `ServiceError` code pages branch on. A 401 or
 * 403 goes through `deniedFor` (WEB-11): the refusal reads "You don't have
 * access" with the permission the API named (`QA LH-DATA-12-09`, `TC-1724`).
 * Every other status keeps this table: 404 is "not_found", 400 and 409 are
 * "invalid_request" (the request is wrong, not the service), the rest
 * "unavailable".
 */
function errorFor(status: number, body: { error?: string } | null, fallback: string): ServiceError {
  const denied = deniedFor(status, body);
  if (denied) return denied;
  const message = body?.error ?? fallback;
  if (status === 404) return new ServiceError("not_found", message);
  if (status === 400 || status === 409) return new ServiceError("invalid_request", message);
  return new ServiceError("unavailable", message);
}

/**
 * AssetService is real — the data catalog from the lakehouse (bronze_meta +
 * silver) via the server route `/api/catalog`. The free-text term (`q`) is
 * matched server-side (`rust/crates/lakehouse-api/src/routes/catalog_search.rs`,
 * the one matcher the Data Explorer's `search` also uses, `DATA-11`) rather
 * than in the browser, so there is one implementation of the term match, not
 * two (WS2 §13). The server's rank order is kept. Facet filters
 * (`tier`/`layer`/`type`/`classification`) stay client-side, applied to the
 * returned list below.
 */
/**
 * Loads the catalog, or the search results for `q`.
 *
 * `limit` is how many assets the caller wants (`DATA-41` D3): the server
 * ranks every match, returns the first `limit` of them, and says how many
 * matched in all. Without it the answer is every match, as it was.
 */
async function loadCatalog(
  q: string | undefined,
  limit: number | undefined,
  signal?: AbortSignal,
): Promise<{ assets: Asset[]; namespaces: CatalogNamespace[]; total?: number }> {
  const params = new URLSearchParams();
  if (q) params.set("q", q);
  if (limit !== undefined) params.set("limit", String(limit));
  const url = params.size > 0 ? `/api/catalog?${params}` : "/api/catalog";
  const res = await apiFetch(url, { signal });
  const json = await res.json();
  // DATA-11 review SHOULD-FIX 4: a refusal (`403`) keeps its own code so the
  // ⌘K box can tell "no access" from an outage (`errorFor`, as the other
  // methods of this file do).
  if (!res.ok) throw errorFor(res.status, json, "Failed to load catalog");
  // DATA-11 review SHOULD-FIX 4: a search answered `supported: false` (the
  // tenant refusal, `200`) carries a fixed `reason`. It is thrown with the
  // status it came with, `200`, which no failed request has, so the palette
  // can show it and nothing else. Only a search throws: the namespace and
  // asset lists of other pages keep reading the empty answer as before.
  if (q && json?.supported === false) {
    throw new ServiceError("unavailable", json.reason ?? "Catalog search is unavailable", res.status);
  }
  return json;
}

export const clickhouseAssetService: AssetService = {
  async listAssets(filter: AssetFilter, signal) {
    const term = (filter.search ?? "").trim();
    const { assets } = await loadCatalog(term || undefined, filter.limit, signal);
    return assets.filter((a) => {
      if (filter.tier && filter.tier !== "all" && a.tier !== filter.tier) return false;
      if (filter.layer && filter.layer !== "all" && a.layer !== filter.layer) return false;
      if (filter.type && filter.type !== "all" && a.type !== filter.type) return false;
      if (filter.classification && filter.classification !== "all" && a.classification !== filter.classification)
        return false;
      return true;
    });
  },
  async searchAssets(term, limit, signal) {
    const { assets, total } = await loadCatalog(term.trim(), limit, signal);
    // `DATA-41` D3: the server counts every match; a server that answers no
    // count is read as "all of them", which is what its answer holds.
    return { assets, total: typeof total === "number" ? total : assets.length };
  },
  async listAssetsPage(query, signal) {
    // Only non-empty params are sent: the backend applies its own
    // defaults, and a URL full of `filters=` blanks makes the React Query
    // cache key noisier than the state it represents.
    const params = new URLSearchParams();
    params.set("page", String(query.page));
    params.set("pageSize", String(query.pageSize));
    if (query.search?.trim()) params.set("search", query.search.trim());
    if (query.sort) params.set("sort", query.sort);
    if (query.filters) params.set("filters", query.filters);
    if (query.joinOperator) params.set("joinOperator", query.joinOperator);
    if (query.groupBy) params.set("groupBy", query.groupBy);
    if (query.skipListMeta) params.set("skipListMeta", "true");

    const res = await apiFetch(`/api/catalog/query?${params}`, { signal });
    const json = await res.json();
    if (!res.ok) {
      // QA LH-DATA-12-09, TC-1724 (WEB-11): a refusal is not an outage; the
      // Catalog said "Service unavailable" with Retry.
      const denied = deniedFor(res.status, json);
      if (denied) throw denied;
      // 400 means this client sent a field the server does not allow —
      // a bug here, not a transient outage, so it is not "unavailable".
      throw new ServiceError(
        res.status === 400 ? "invalid_request" : "unavailable",
        json?.error ?? "Katalog gagal dimuat"
      );
    }
    return json as Pagination<Asset>;
  },
  async getAsset(id, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}`, { signal });
    // QA LH-DATA-13-03 / LH-DATA-15-03 F4 (plan D8): every failure used to
    // be "not_found", so a down engine or a missing permission read as
    // "Not found". The status picks the kind, and a body that is not JSON
    // must not hide it.
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Asset not found");
    return json as AssetDetail;
  },
  async getAssetProfile(id, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/profile`, { signal });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to profile asset");
    return json as AssetProfile;
  },
  async getAssetSample(id, limit, signal) {
    const res = await apiFetch(
      `/api/catalog/${encodeURIComponent(id)}/sample?limit=${encodeURIComponent(limit)}`,
      { signal }
    );
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to load sample rows");
    return (json?.rows ?? []) as Record<string, string | null>[];
  },
  async updateAnnotation(id, input, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/annotation`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      signal,
    });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to save the asset's details");
  },
  async setCertification(id, input, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/certification`, {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      signal,
    });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to save the certification");
  },
  async listNamespaces(signal) {
    return (await loadCatalog(undefined, undefined, signal)).namespaces;
  },
  async requestAccess(catalogId, input: RequestAccessInput, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(catalogId)}/access-request`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      signal,
    });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to request access");
  },
  async decideAccessRequest(approvalId, decision, comment, signal) {
    const res = await apiFetch(
      `/api/catalog/access-requests/${encodeURIComponent(approvalId)}/decide`,
      {
        method: "POST",
        headers: { "content-type": "application/json" },
        body: JSON.stringify({ decision, comment }),
        signal,
      }
    );
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to decide access request");
    return json as DecideAccessRequestResult;
  },
  async listGrants(id, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/grants`, { signal });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to load the grants");
    return (json?.grants ?? []) as AssetGrant[];
  },
  async grantAccess(id, input, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/grants`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      signal,
    });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to grant access");
  },
  async getRestoreStatus(id, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/restore`, { signal });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to check whether the table can be restored");
    return json as AssetRestoreStatus;
  },
  async restoreTable(id, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/restore`, {
      method: "POST",
      signal,
    });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to restore the table");
  },
  // `DATA-55` (QA `LH-DATA-15-06`, `TC-1731`): the drop, its status read and
  // the list of dropped tables. Every sentence of a refusal comes from the API
  // (`errorFor`), and the count is only asked for when the dialog needs it.
  async getDropStatus(id, options, signal) {
    const query = options?.withRowCount ? "?withRowCount=true" : ""
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/drop${query}`, { signal })
    const json = await res.json().catch(() => null)
    if (!res.ok) throw errorFor(res.status, json, "Failed to check whether the table can be dropped")
    return json as AssetDropStatus
  },
  async dropTable(id, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}/drop`, {
      method: "POST",
      signal,
    })
    const json = await res.json().catch(() => null)
    if (!res.ok) throw errorFor(res.status, json, "Failed to drop the table")
    return json as AssetDropResult
  },
  async listDroppedTables(signal) {
    const res = await apiFetch("/api/catalog/dropped/tables", { signal })
    const json = await res.json().catch(() => null)
    if (!res.ok) throw errorFor(res.status, json, "Failed to list the dropped tables")
    // `supported: false` (outside the tenant that owns the shared catalog)
    // carries an empty list, and so does an answer that is not shaped as one.
    return Array.isArray(json?.tables) ? (json.tables as DroppedTableEntry[]) : []
  },
  async revokeGrant(id, grantId, signal) {
    const res = await apiFetch(
      `/api/catalog/${encodeURIComponent(id)}/grants/${encodeURIComponent(grantId)}`,
      { method: "DELETE", signal }
    );
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json, "Failed to revoke the grant");
  },
};
