import type {
  GovernanceService,
  QualityRule,
  LineageGraph,
  AuditEvent,
  ClassificationRule,
  ResidencyRule,
} from "../contracts/governance";
import { mockGovernanceService } from "../mock/governance";
import { ServiceError } from "../errors";

/**
 * GovernanceService: quality/lineage/audit/classification/residency NYATA dari
 * lakehouse (_silver_meta, Dagster, katalog). Policies + create* masih mock
 * (belum ada policy engine yang menegakkan).
 */
async function get<T>(url: string, signal?: AbortSignal): Promise<T> {
  const res = await fetch(url, { signal });
  const json = await res.json();
  if (!res.ok) throw new ServiceError("unavailable", json?.error ?? "Governance gagal dimuat");
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

  // ── Mock (belum ada policy/masking engine nyata) ────────────────────────
  listPolicies: (s) => mockGovernanceService.listPolicies(s),
  createPolicy: (i, s) => mockGovernanceService.createPolicy(i, s),
  createQualityRule: (i, s) => mockGovernanceService.createQualityRule(i, s),
  createClassificationRule: (i, s) => mockGovernanceService.createClassificationRule(i, s),
  createResidencyRule: (i, s) => mockGovernanceService.createResidencyRule(i, s),
};
