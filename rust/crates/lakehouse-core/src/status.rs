//! Backend-serialized status taxonomies, ported from `src/lib/status.ts`.
//!
//! Only the discriminated unions the backend actually serializes into JSON
//! responses are ported here. The `*_LABEL` / `*_DESCRIPTION` maps in the
//! TypeScript source are presentation strings for the UI and intentionally
//! stay in TypeScript.

use serde::{Deserialize, Serialize};

/// Lifecycle status shared by pipelines, jobs, policies, agents, and runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EntityStatus {
    /// Not yet validated or deployed.
    Draft,
    /// Configuration checks are in progress.
    Validating,
    /// Validated and available to run.
    Ready,
    /// Waiting for its next scheduled trigger.
    Scheduled,
    /// Currently executing.
    Running,
    /// Stopped by an operator; can be resumed.
    Paused,
    /// Operating with reduced health or delayed output.
    Degraded,
    /// The last execution ended with an error.
    Failed,
    /// Finished successfully.
    Completed,
    /// Stopped before completion.
    Cancelled,
    /// Stopped by policy, quota, or approval gate.
    Blocked,
    /// Finished with some accepted and some rejected work.
    Partial,
    /// Retained for reference; no longer active.
    Archived,
}

/// Health summary for services, connectors, and assets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Health {
    /// Fully operational.
    Healthy,
    /// Operating with reduced health or delayed output.
    Degraded,
    /// Not operational.
    Unhealthy,
    /// Health could not be determined.
    Unknown,
}

/// Storage tiers — the primary physical data story of the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageTier {
    /// Recent, write-heavy analytical data on fast local storage.
    Hot,
    /// Older partitions on object-backed storage with local cache.
    Warm,
    /// Historical open-format data on object storage.
    Cold,
    /// Vectors, features, and multimodal datasets for retrieval.
    Ai,
}

/// Logical modeling layers, retained as a secondary filter dimension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataLayer {
    /// Unmodified source data.
    Raw,
    /// Lightly cleaned, schema-conformed data.
    Bronze,
    /// Conformed, joined, deduplicated data.
    Silver,
    /// Business-level aggregates and marts.
    Gold,
    /// Modeled, governed metrics layer.
    Semantic,
}

/// Data classification levels used by policies and residency rules.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Classification {
    /// No access restrictions.
    Public,
    /// Restricted to the organization.
    Internal,
    /// Restricted to a need-to-know group.
    Confidential,
    /// Subject to the strictest handling requirements.
    Restricted,
}

/// Agent autonomy levels with their product meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AutonomyLevel {
    /// Read-only assist.
    #[serde(rename = "L1")]
    L1,
    /// Propose & simulate.
    #[serde(rename = "L2")]
    L2,
    /// Act with approval.
    #[serde(rename = "L3")]
    L3,
    /// Bounded autonomy.
    #[serde(rename = "L4")]
    L4,
}

/// Workload classes assigned by the access layer when a request is
/// classified.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum WorkloadClass {
    /// Hot analytics.
    HotAnalytics,
    /// Site-facing analytics.
    SiteFacing,
    /// Federated query.
    Federated,
    /// Join-heavy query.
    JoinHeavy,
    /// Retrieval.
    Retrieval,
    /// Telemetry.
    Telemetry,
    /// Ingestion.
    Ingestion,
    /// Agent tool call.
    AgentTool,
}

/// Execution engine categories, kept product-neutral for default UI copy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EngineCategory {
    /// Hot analytical store.
    HotStore,
    /// Federated compute.
    FederatedCompute,
    /// Real-time streaming.
    Streaming,
    /// AI retrieval store.
    AiStore,
    /// Telemetry store.
    TelemetryStore,
}

/// Alert / severity scale shared by alerts, quality results, and
/// violations.
///
/// Ordered most-severe-first: `Critical < ... < Info`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    /// Requires immediate attention.
    Critical,
    /// Significant impact.
    High,
    /// Moderate impact.
    Medium,
    /// Minor impact.
    Low,
    /// Informational only.
    Info,
}

/// Result of a quality, validation, or health check run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    /// The check succeeded.
    Passed,
    /// The check succeeded with caveats.
    Warning,
    /// The check failed.
    Failed,
}

/// Approval lifecycle shared by agent runs, tools, and policy submissions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ApprovalStatus {
    /// Awaiting a decision.
    Pending,
    /// Approved by a reviewer.
    Approved,
    /// Rejected by a reviewer.
    Rejected,
}

/// Who performed an action: a person, a platform service, or a delegated
/// agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ActorKind {
    /// A human user.
    User,
    /// A platform service.
    Service,
    /// A delegated agent.
    Agent,
}

/// Outcome of an audited action after policy evaluation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AuditOutcome {
    /// The action succeeded.
    Success,
    /// The action was denied by policy.
    Denied,
    /// The action failed with an error.
    Error,
}

/// Alert triage lifecycle shared by alerts and incidents.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertStatus {
    /// Newly raised, not yet triaged.
    Open,
    /// Seen and claimed by an operator.
    Acknowledged,
    /// Closed out.
    Resolved,
}

/// Request lifecycle in the workload admission queue.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum WorkloadStatus {
    /// Waiting for admission.
    Queued,
    /// Currently executing.
    Running,
    /// Finished successfully.
    Completed,
    /// Stopped before completion.
    Cancelled,
}

#[cfg(test)]
mod tests {
    #![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

    use super::*;

    #[test]
    fn health_serializes_to_lowercase_tag() {
        assert_eq!(
            serde_json::to_string(&Health::Degraded).unwrap(),
            "\"degraded\""
        );
    }
    #[test]
    fn storage_tier_round_trips() {
        assert_eq!(
            serde_json::from_str::<StorageTier>("\"cold\"").unwrap(),
            StorageTier::Cold
        );
    }
    #[test]
    fn severity_orders_critical_first() {
        assert!(Severity::Critical < Severity::Info);
    }
    #[test]
    fn alert_status_round_trips() {
        assert_eq!(
            serde_json::from_str::<AlertStatus>("\"acknowledged\"").unwrap(),
            AlertStatus::Acknowledged
        );
    }
}
