import type {
  AssetService,
  Asset,
  AssetDetail,
  AssetFilter,
  CatalogNamespace,
  DecideAccessRequestResult,
  RequestAccessInput,
} from "../contracts/assets";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * Maps an HTTP status to the `ServiceError` code pages branch on. Mirrors
 * `postgresAgentService`'s own `errorFor` (`clients/agents.ts`) — `403`
 * becomes `"permission_denied"` so a self-approval refusal
 * (`routes::catalog::decide_access_request`, WS7 item E3) surfaces the
 * backend's own message rather than the generic `"unavailable"` this
 * file's other methods fell back to before this task.
 */
function errorFor(status: number, message: string): ServiceError {
  if (status === 404) return new ServiceError("not_found", message);
  if (status === 400 || status === 409) return new ServiceError("invalid_request", message);
  if (status === 401 || status === 403) return new ServiceError("permission_denied", message);
  return new ServiceError("unavailable", message);
}

/**
 * AssetService is real — the data catalog from the lakehouse (bronze_meta +
 * silver) via the server route `/api/catalog`. The free-text term (`q`) is
 * filtered server-side (`rust/crates/lakehouse-api/src/routes/catalog.rs`'s
 * `filter_assets_by_query`) rather than in the browser, so there is one
 * implementation of the term match, not two (WS2 §13). Facet filters
 * (`tier`/`layer`/`type`/`classification`) stay client-side, applied to the
 * returned list below.
 */
async function loadCatalog(
  q: string | undefined,
  signal?: AbortSignal,
): Promise<{ assets: Asset[]; namespaces: CatalogNamespace[] }> {
  const url = q ? `/api/catalog?q=${encodeURIComponent(q)}` : "/api/catalog";
  const res = await apiFetch(url, { signal });
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Failed to load catalog");
  return json;
}

export const clickhouseAssetService: AssetService = {
  async listAssets(filter: AssetFilter, signal) {
    const term = (filter.search ?? "").trim();
    const { assets } = await loadCatalog(term || undefined, signal);
    return assets.filter((a) => {
      if (filter.tier && filter.tier !== "all" && a.tier !== filter.tier) return false;
      if (filter.layer && filter.layer !== "all" && a.layer !== filter.layer) return false;
      if (filter.type && filter.type !== "all" && a.type !== filter.type) return false;
      if (filter.classification && filter.classification !== "all" && a.classification !== filter.classification)
        return false;
      return true;
    });
  },
  async getAsset(id, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(id)}`, { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("not_found", json?.error ?? "Asset not found");
    return json as AssetDetail;
  },
  async listNamespaces(signal) {
    return (await loadCatalog(undefined, signal)).namespaces;
  },
  async requestAccess(catalogId, input: RequestAccessInput, signal) {
    const res = await apiFetch(`/api/catalog/${encodeURIComponent(catalogId)}/access-request`, {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(input),
      signal,
    });
    const json = await res.json().catch(() => null);
    if (!res.ok) throw errorFor(res.status, json?.error ?? "Failed to request access");
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
    if (!res.ok) throw errorFor(res.status, json?.error ?? "Failed to decide access request");
    return json as DecideAccessRequestResult;
  },
};
