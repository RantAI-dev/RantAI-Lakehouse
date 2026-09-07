import type {
  AssetService,
  Asset,
  AssetDetail,
  AssetFilter,
  CatalogNamespace,
} from "../contracts/assets";
import type { Pagination } from "../contracts/pagination";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * AssetService NYATA — katalog data dari lakehouse (bronze_meta + silver) lewat
 * route server `/api/catalog`. Filter di-terapkan di client (katalog kecil).
 */
async function loadCatalog(signal?: AbortSignal): Promise<{ assets: Asset[]; namespaces: CatalogNamespace[] }> {
  const res = await apiFetch("/api/catalog", { signal });
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Katalog gagal dimuat");
  return json;
}

export const clickhouseAssetService: AssetService = {
  async listAssets(filter: AssetFilter, signal) {
    const { assets } = await loadCatalog(signal);
    const term = (filter.search ?? "").trim().toLowerCase();
    return assets.filter((a) => {
      if (filter.tier && filter.tier !== "all" && a.tier !== filter.tier) return false;
      if (filter.layer && filter.layer !== "all" && a.layer !== filter.layer) return false;
      if (filter.type && filter.type !== "all" && a.type !== filter.type) return false;
      if (filter.classification && filter.classification !== "all" && a.classification !== filter.classification)
        return false;
      if (!term) return true;
      return (
        a.name.toLowerCase().includes(term) ||
        a.description.toLowerCase().includes(term) ||
        a.id.toLowerCase().includes(term)
      );
    });
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
    const json = await res.json();
    if (!res.ok) throw new ServiceError("not_found", json?.error ?? "Aset tidak ditemukan");
    return json as AssetDetail;
  },
  async listNamespaces(signal) {
    return (await loadCatalog(signal)).namespaces;
  },
};
