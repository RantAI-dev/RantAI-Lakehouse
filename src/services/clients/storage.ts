import type { StorageService, StorageOverview } from "../contracts/storage";
import { mockStorageService } from "../mock/storage";
import { ServiceError } from "../errors";

/**
 * StorageService: getOverview NYATA (ukuran tier dari ClickHouse/Iceberg).
 * Policy/operations/restore masih mock (belum ada lifecycle engine nyata).
 */
export const clickhouseStorageService: StorageService = {
  async getOverview(signal) {
    const res = await fetch("/api/storage", { signal });
    const json = await res.json();
    if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Storage gagal dimuat");
    return json as StorageOverview;
  },
  listPolicies: (s) => mockStorageService.listPolicies(s),
  listOperations: (s) => mockStorageService.listOperations(s),
  createLifecyclePolicy: (i, s) => mockStorageService.createLifecyclePolicy(i, s),
  restoreAsset: (i, s) => mockStorageService.restoreAsset(i, s),
};
