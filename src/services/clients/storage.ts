import type {
  StorageService,
  StorageOverview,
  LifecyclePolicy,
  TieringOp,
  CreateLifecyclePolicyInput,
  RestoreAssetInput,
} from "../contracts/storage";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * StorageService NYATA sepenuhnya: getOverview dari ClickHouse/Iceberg;
 * policies/operations/restore dari Postgres (Task 2.6). mock/storage.ts
 * sudah dihapus.
 */
async function getJson<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, { signal });
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? `Gagal (${res.status})`);
  return json as T;
}

async function postJson<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const json = await res.json();
  if (!res.ok) {
    const kind = res.status === 404 ? "not_found" : res.status >= 500 ? "unavailable" : "invalid_request";
    throw new ServiceError(kind, json?.error ?? `Gagal (${res.status})`);
  }
  return json as T;
}

export const clickhouseStorageService: StorageService = {
  getOverview(signal) {
    return getJson<StorageOverview>("/api/storage", signal);
  },
  listPolicies(signal) {
    return getJson<LifecyclePolicy[]>("/api/storage/policies", signal);
  },
  listOperations(signal) {
    return getJson<TieringOp[]>("/api/storage/operations", signal);
  },
  createLifecyclePolicy(input: CreateLifecyclePolicyInput, signal) {
    return postJson<LifecyclePolicy>("/api/storage/policies", input, signal);
  },
  restoreAsset(input: RestoreAssetInput, signal) {
    return postJson<TieringOp>("/api/storage/restore", input, signal);
  },
};
