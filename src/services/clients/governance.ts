import type {
  GovernanceService,
  Policy,
  QualityRule,
  LineageGraph,
  AuditEvent,
  ClassificationRule,
  ResidencyRule,
  CreatePolicyInput,
  CreateQualityRuleInput,
  CreateClassificationRuleInput,
  CreateResidencyRuleInput,
} from "../contracts/governance";
import { ServiceError } from "../errors";

/**
 * GovernanceService: quality/lineage/audit/classification/residency NYATA dari
 * lakehouse (_silver_meta, Dagster, katalog) — dibaca lewat
 * `/api/governance/{kind}` / `/api/governance/lineage`, tidak berubah dari
 * Fase 1. Policies (list + create) dan tiga `create*Rule` sekarang NYATA
 * juga, tersimpan di Postgres lewat crate `lakehouse-store` (Fase 2, Task
 * 2.3) — menggantikan seluruh `mock/governance.ts`.
 *
 * Catatan penting: rule yang dibuat lewat `create*Rule` TIDAK muncul di
 * `listQuality`/`listClassifications`/`listResidency` — dua sisi itu
 * sengaja punya sumber data berbeda (config yang ditulis manusia vs. hasil
 * observasi ClickHouse), sesuai desain backend Rust-nya.
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
  const res = await fetch(url, { signal });
  const json = await res.json().catch(() => null);
  if (!res.ok) throw errorFor(res.status, json?.error ?? "Governance gagal dimuat");
  return json as T;
}

async function post<T>(url: string, body: unknown, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(body),
    signal,
  });
  const json = await res.json().catch(() => null);
  if (!res.ok) throw errorFor(res.status, json?.error ?? "Governance gagal disimpan");
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
  async listResidency(signal) {
    return (await get<{ residency: ResidencyRule[] }>("/api/governance/residency", signal)).residency;
  },
  async getLineage(focusId, signal) {
    return get<LineageGraph>(`/api/governance/lineage?focus=${encodeURIComponent(focusId)}`, signal);
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
  createResidencyRule(input: CreateResidencyRuleInput, signal) {
    return post<ResidencyRule>("/api/governance/residency", input, signal);
  },
};
