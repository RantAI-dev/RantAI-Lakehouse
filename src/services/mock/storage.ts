import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type { StorageService } from "../contracts/storage"

export const mockStorageService: StorageService = {
  getOverview(signal) {
    return mockCall(
      () => ({
        byTier: {
          hot: { bytes: 8.4 * 1024 ** 4, assets: 212, growth7d: 0.04 },
          warm: { bytes: 22.1 * 1024 ** 4, assets: 388, growth7d: 0.02 },
          cold: { bytes: 148.6 * 1024 ** 4, assets: 611, growth7d: 0.06 },
          ai: { bytes: 3.2 * 1024 ** 4, assets: 73, growth7d: 0.11 },
        },
        savingsVsAllHot: 0.72,
        failedTieringOps: 1,
        pendingRestores: 2,
      }),
      { signal }
    )
  },
  listPolicies(signal) {
    return mockCall(
      () => [
        {
          id: "lp-1",
          name: "default_analytics_lifecycle",
          scope: "core.* analytical tables",
          hotDays: 14,
          warmDays: 90,
          coldAfterDays: 91,
          status: "ready" as const,
          estimatedSavings: "68% vs all-hot",
          lastAppliedAt: agoIso(300),
        },
        {
          id: "lp-2",
          name: "ai_derivative_retention",
          scope: "ai.* vector datasets",
          hotDays: 0,
          warmDays: 0,
          coldAfterDays: 365,
          status: "draft" as const,
          estimatedSavings: "Rebuildable from lineage",
          lastAppliedAt: agoIso(1000),
        },
      ],
      { signal }
    )
  },
  listOperations(signal) {
    return mockCall(
      () => [
        {
          id: "op-1",
          asset: "lake.sales.orders_history",
          from: "warm" as const,
          to: "cold" as const,
          status: "completed" as const,
          at: agoIso(200),
          detail: "Exported partitions 2024-Q1 → Iceberg snapshot",
        },
        {
          id: "op-2",
          asset: "core.sales.orders_events",
          from: "hot" as const,
          to: "warm" as const,
          status: "failed" as const,
          at: agoIso(40),
          detail: "Checksum mismatch on partition 2026-06-01",
        },
      ],
      { signal }
    )
  },
}
