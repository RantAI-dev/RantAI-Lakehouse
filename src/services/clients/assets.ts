import type {
  AssetService,
  Asset,
  AssetDetail,
  AssetFilter,
  CatalogNamespace,
} from "../contracts/assets";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * AssetService is real — the data catalog from the lakehouse (bronze_meta +
 * silver) via the server route `/api/catalog`. The free-text term (`q`) is
 * filtered server-side (`rust/crates/lakehouse-api/src/routes/catalog.rs`'s
 * `filter_assets_by_query`) rather than in the browser, so there is one
 * implementation of the term match, not two (WS2 §13, Task E2 pre-dispatch
 * fix E2-2). Facet filters (`tier`/`layer`/`type`/`classification`) stay
 * client-side, applied to the returned list below.
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
};
