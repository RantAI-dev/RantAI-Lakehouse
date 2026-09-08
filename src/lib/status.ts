/**
 * Shared product taxonomies.
 *
 * Every module must use these unions and label maps instead of inventing
 * page-local status strings, so badges, filters, and copy stay consistent.
 */

/** Lifecycle status shared by pipelines, jobs, policies, agents, and runs. */
export type EntityStatus =
  | "draft"
  | "validating"
  | "ready"
  | "scheduled"
  | "running"
  | "paused"
  | "degraded"
  | "failed"
  | "completed"
  | "cancelled"
  | "blocked"
  | "partial"
  | "archived"

export const ENTITY_STATUS_LABEL: Record<EntityStatus, string> = {
  draft: "Draft",
  validating: "Validating",
  ready: "Ready",
  scheduled: "Scheduled",
  running: "Running",
  paused: "Paused",
  degraded: "Degraded",
  failed: "Failed",
  completed: "Completed",
  cancelled: "Cancelled",
  blocked: "Blocked",
  partial: "Partial success",
  archived: "Archived",
}

export const ENTITY_STATUS_DESCRIPTION: Record<EntityStatus, string> = {
  draft: "Not yet validated or deployed.",
  validating: "Configuration checks are in progress.",
  ready: "Validated and available to run.",
  scheduled: "Waiting for its next scheduled trigger.",
  running: "Currently executing.",
  paused: "Stopped by an operator; can be resumed.",
  degraded: "Operating with reduced health or delayed output.",
  failed: "The last execution ended with an error.",
  completed: "Finished successfully.",
  cancelled: "Stopped before completion.",
  blocked: "Stopped by policy, quota, or approval gate.",
  partial: "Finished with some accepted and some rejected work.",
  archived: "Retained for reference; no longer active.",
}

/** Health summary for services, connectors, and assets. */
export type Health = "healthy" | "degraded" | "unhealthy" | "unknown"

export const HEALTH_LABEL: Record<Health, string> = {
  healthy: "Healthy",
  degraded: "Degraded",
  unhealthy: "Unhealthy",
  unknown: "Unknown",
}

/** Storage tiers — primary physical data story of the platform. */
export type StorageTier = "hot" | "warm" | "cold" | "ai"

export const STORAGE_TIER_LABEL: Record<StorageTier, string> = {
  hot: "Hot",
  warm: "Warm",
  cold: "Cold",
  ai: "AI",
}

export const STORAGE_TIER_DESCRIPTION: Record<StorageTier, string> = {
  hot: "Recent, write-heavy analytical data on fast local storage.",
  warm: "Older partitions on object-backed storage with local cache.",
  cold: "Historical open-format data on object storage.",
  ai: "Vectors, features, and multimodal datasets for retrieval.",
}

/** Logical modeling layers, retained as a secondary filter dimension. */
export type DataLayer = "raw" | "bronze" | "silver" | "gold" | "semantic"

export const DATA_LAYER_LABEL: Record<DataLayer, string> = {
  raw: "Raw",
  bronze: "Bronze",
  silver: "Silver",
  gold: "Gold",
  semantic: "Semantic",
}

/** Data classification levels used by policies and residency rules. */
export type Classification =
  | "public"
  | "internal"
  | "confidential"
  | "restricted"

export const CLASSIFICATION_LABEL: Record<Classification, string> = {
  public: "Public",
  internal: "Internal",
  confidential: "Confidential",
  restricted: "Restricted",
}

/** Agent autonomy levels with their product meaning. */
export type AutonomyLevel = "L1" | "L2" | "L3" | "L4"

export const AUTONOMY_LABEL: Record<AutonomyLevel, string> = {
  L1: "L1 · Read-only assist",
  L2: "L2 · Propose & simulate",
  L3: "L3 · Act with approval",
  L4: "L4 · Bounded autonomy",
}

export const AUTONOMY_DESCRIPTION: Record<AutonomyLevel, string> = {
  L1: "Reads governed data, retrieves context, and drafts recommendations.",
  L2: "Runs what-if analysis and creates proposals in isolated branches.",
  L3: "Prepares actions that commit only after an explicit approval.",
  L4: "Executes a predefined action set under hard budget and scope limits.",
}

/**
 * Workload classes assigned by the access layer when a request is classified.
 * Presented with product-neutral names; engine details live in advanced views.
 */
export type WorkloadClass =
  | "hot-analytics"
  | "site-facing"
  | "federated"
  | "join-heavy"
  | "retrieval"
  | "telemetry"
  | "ingestion"
  | "agent-tool"

export const WORKLOAD_CLASS_LABEL: Record<WorkloadClass, string> = {
  "hot-analytics": "Hot analytics",
  "site-facing": "Site-facing analytics",
  federated: "Federated query",
  "join-heavy": "Join-heavy query",
  retrieval: "Retrieval",
  telemetry: "Telemetry",
  ingestion: "Ingestion",
  "agent-tool": "Agent tool call",
}

/** Execution engine categories, kept product-neutral for default UI copy. */
export type EngineCategory =
  | "hot-store"
  | "federated-compute"
  | "ai-store"
  | "telemetry-store"

export const ENGINE_CATEGORY_LABEL: Record<EngineCategory, string> = {
  "hot-store": "Hot analytical store",
  "federated-compute": "Federated compute",
  "ai-store": "AI retrieval store",
  "telemetry-store": "Telemetry store",
}

/** Alert / severity scale shared by alerts, quality results, and violations. */
export type Severity = "critical" | "high" | "medium" | "low" | "info"

export const SEVERITY_LABEL: Record<Severity, string> = {
  critical: "Critical",
  high: "High",
  medium: "Medium",
  low: "Low",
  info: "Info",
}

/** Result of a quality, validation, or health check run. */
export type CheckStatus = "passed" | "warning" | "failed"

export const CHECK_STATUS_LABEL: Record<CheckStatus, string> = {
  passed: "Passed",
  warning: "Warning",
  failed: "Failed",
}

/** Approval lifecycle shared by agent runs, tools, and policy submissions. */
export type ApprovalStatus = "pending" | "approved" | "rejected"

export const APPROVAL_STATUS_LABEL: Record<ApprovalStatus, string> = {
  pending: "Pending",
  approved: "Approved",
  rejected: "Rejected",
}

/** Who performed an action: a person, a platform service, or a delegated agent. */
export type ActorKind = "user" | "service" | "agent"

export const ACTOR_KIND_LABEL: Record<ActorKind, string> = {
  user: "User",
  service: "Service",
  agent: "Agent",
}

/** Outcome of an audited action after policy evaluation. */
export type AuditOutcome = "success" | "denied" | "error"

export const AUDIT_OUTCOME_LABEL: Record<AuditOutcome, string> = {
  success: "Success",
  denied: "Denied",
  error: "Error",
}

/** Alert triage lifecycle shared by alerts and incidents. */
export type AlertStatus = "open" | "acknowledged" | "resolved"

export const ALERT_STATUS_LABEL: Record<AlertStatus, string> = {
  open: "Open",
  acknowledged: "Acknowledged",
  resolved: "Resolved",
}

/** Request lifecycle in the workload admission queue. */
export type WorkloadStatus = "queued" | "running" | "completed" | "cancelled"

export const WORKLOAD_STATUS_LABEL: Record<WorkloadStatus, string> = {
  queued: "Queued",
  running: "Running",
  completed: "Completed",
  cancelled: "Cancelled",
}
