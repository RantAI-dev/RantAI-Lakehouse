import type {
  AssetService,
  Asset,
  AssetDetail,
  AssetFilter,
  CatalogNamespace,
} from "../contracts/assets";
import { ServiceError } from "../errors";

/**
 * AssetService NYATA — katalog data dari lakehouse (bronze_meta + silver) lewat
 * route server `/api/catalog`. Filter di-terapkan di client (katalog kecil).
 */
async function loadCatalog(signal?: AbortSignal): Promise<{ assets: Asset[]; namespaces: CatalogNamespace[] }> {
  const res = await fetch("/api/catalog", { signal });
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
  async getAsset(id, signal) {
    const res = await fetch(`/api/catalog/${encodeURIComponent(id)}`, { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("not_found", json?.error ?? "Aset tidak ditemukan");
    return json as AssetDetail;
  },
  async listNamespaces(signal) {
    return (await loadCatalog(signal)).namespaces;
  },
};
