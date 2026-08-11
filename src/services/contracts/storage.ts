import type { EntityStatus, StorageTier } from "@/lib/status"

export type StorageOverview = {
  byTier: Record<StorageTier, { bytes: number; assets: number; growth7d: number }>
  savingsVsAllHot: number
  failedTieringOps: number
  pendingRestores: number
}

export type LifecyclePolicy = {
  id: string
  name: string
  scope: string
  hotDays: number
  warmDays: number
  coldAfterDays: number
  status: Extract<EntityStatus, "ready" | "draft" | "paused">
  estimatedSavings: string
  lastAppliedAt: string
}

export type TieringOp = {
  id: string
  asset: string
  from: StorageTier
  to: StorageTier
  status: Extract<EntityStatus, "running" | "completed" | "failed">
  at: string
  detail: string
}

export interface StorageService {
  getOverview(signal?: AbortSignal): Promise<StorageOverview>
  listPolicies(signal?: AbortSignal): Promise<LifecyclePolicy[]>
  listOperations(signal?: AbortSignal): Promise<TieringOp[]>
}
