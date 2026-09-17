import type {
  GovernanceService,
  Policy,
  QualityRule,
  LineageGraph,
  AuditEvent,
  ClassificationRule,
  MaintenanceRun,
  ReplicationSlot,
  CreatePolicyInput,
  CreateQualityRuleInput,
  CreateClassificationRuleInput,
  DatasetSla,
} from "../contracts/governance";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * GovernanceService: quality/lineage/audit/classification/residency are real,
 * read from the lakehouse (_silver_meta, Dagster, catalog) via
 * `/api/governance/{kind}` / `/api/governance/lineage`, unchanged since
 * Phase 1. Policies (list + create) and the three `create*Rule` methods are
 * now real too, stored in Postgres via the `lakehouse-store` crate (Phase 2,
 * Task 2.3) — replacing all of `mock/governance.ts`.
 *
 * Important note: a rule created via `create*Rule` does NOT appear in
 * `listQuality`/`listClassifications` — the two sides deliberately have
 * different data sources (human-authored config vs. ClickHouse observation
 * results), per the Rust backend's design.
 */

/** Map an error response body onto the ServiceError code its status implies. */
function errorFor(status: number, message: string): ServiceError {
  if (status === 404) return new ServiceError("not_found", message);
  if (status === 400 || status === 409 || status === 422)
    return new ServiceError("invalid_request", message);
  if (status === 401 || status === 403)
    return new ServiceError("permission_denied", message);
  return new ServiceError("unavailable", message);
}

async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, { signal });
  const json = await res.json().catch(() => null);
  if (!res.ok) throw errorFor(res.status, json?.error ?? "Failed to load governance");
  return json as T;
}

async function post<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) throw errorFor(res.status, json?.error ?? "Failed to save governance");
  return json as T;
}

// WS5 item E1 (judge amendment 2): the plan's `putDatasetSla` calls a
// `put` helper this file never had — only `get`/`post` existed. Mirrors
// `post` exactly (same `apiFetch` usage, same `errorFor` mapping) rather
// than routing a real PUT through `post`'s method string, which would
// ship a 405 against `PUT /api/governance/sla`.
async function put<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await apiFetch(url, {
    method: "PUT",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) throw errorFor(res.status, json?.error ?? "Failed to save governance");
  return json as T;
}

export const clickhouseGovernanceService: GovernanceService = {
  async listQuality(signal) {
    return (await get<{ quality: QualityRule[] }>("/api/governance/quality", signal)).quality;
  },
  async listAudit(signal) {
    return (await get<{ audit: AuditEvent[] }>("/api/governance/audit", signal)).audit;
  },
  async listClassifications(signal) {
    return (await get<{ classifications: ClassificationRule[] }>("/api/governance/classification", signal)).classifications;
  },
  async getLineage(focusId, signal) {
    return get<LineageGraph>(`/api/governance/lineage?focus=${encodeURIComponent(focusId)}`, signal);
  },
  // P6: Bronze maintenance (P4) and CDC replication-slot health (P5),
  // surfaced through the same `bronze_meta.*`-backed governance endpoints
  // the Rust side already exposes — no new backend route added here.
  async listMaintenanceRuns(signal) {
    return (await get<{ maintenance: MaintenanceRun[] }>("/api/governance/maintenance", signal)).maintenance;
  },
  async listReplicationSlots(signal) {
    return (await get<{ replicationSlots: ReplicationSlot[] }>("/api/governance/replication", signal)).replicationSlots;
  },

  // ── Postgres (authored config) ──────────────────────────────────────────
  listPolicies(signal) {
    return get<Policy[]>("/api/governance/policies", signal);
  },
  createPolicy(input: CreatePolicyInput, signal) {
    return post<Policy>("/api/governance/policies", input, signal);
  },
  createQualityRule(input: CreateQualityRuleInput, signal) {
    return post<QualityRule>("/api/governance/quality", input, signal);
  },
  createClassificationRule(input: CreateClassificationRuleInput, signal) {
    return post<ClassificationRule>("/api/governance/classification", input, signal);
  },
  listDatasetSla(signal) {
    return get<DatasetSla[]>("/api/governance/sla", signal);
  },
  putDatasetSla(input: DatasetSla, signal) {
    return put<DatasetSla>("/api/governance/sla", input, signal);
  },
};
