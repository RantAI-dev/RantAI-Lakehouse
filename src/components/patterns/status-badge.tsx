import { cn } from "@/lib/utils"
import {
  ALERT_STATUS_LABEL,
  APPROVAL_STATUS_LABEL,
  AUDIT_OUTCOME_LABEL,
  AUTONOMY_LABEL,
  CHECK_STATUS_LABEL,
  CLASSIFICATION_LABEL,
  ENTITY_STATUS_DESCRIPTION,
  ENTITY_STATUS_LABEL,
  HEALTH_LABEL,
  SEVERITY_LABEL,
  STORAGE_TIER_LABEL,
  WORKLOAD_STATUS_LABEL,
  type AlertStatus,
  type ApprovalStatus,
  type AuditOutcome,
  type AutonomyLevel,
  type CheckStatus,
  type Classification,
  type EntityStatus,
  type Health,
  type Severity,
  type StorageTier,
  type WorkloadStatus,
} from "@/lib/status"

type Tone =
  | "neutral"
  | "info"
  | "success"
  | "warning"
  | "destructive"
  | "orange"
  | "sky"
  | "violet"

const TONE_CLASS: Record<Tone, string> = {
  neutral: "bg-muted text-muted-foreground",
  info: "bg-primary/10 text-primary",
  success: "bg-emerald-500/12 text-emerald-600 dark:text-emerald-400",
  warning: "bg-amber-500/12 text-amber-600 dark:text-amber-400",
  destructive: "bg-destructive/10 text-destructive",
  orange: "bg-orange-500/12 text-orange-600 dark:text-orange-400",
  sky: "bg-sky-500/12 text-sky-600 dark:text-sky-400",
  violet: "bg-violet-500/12 text-violet-600 dark:text-violet-400",
}

/**
 * Base pill. Text label is always present so color is never the only cue.
 * Exported for domain-local statuses (for example user active/inactive) that
 * do not warrant their own dedicated badge component.
 */
export function Pill({
  tone,
  children,
  title,
  className,
}: {
  tone: Tone
  children: React.ReactNode
  title?: string
  className?: string
}) {
  return (
    <span
      title={title}
      className={cn(
        "inline-flex items-center gap-1 whitespace-nowrap rounded-full px-2 py-0.5 text-xs font-medium leading-4",
        TONE_CLASS[tone],
        className
      )}
    >
      {children}
    </span>
  )
}

const STATUS_TONE: Record<EntityStatus, Tone> = {
  draft: "neutral",
  validating: "info",
  ready: "success",
  scheduled: "info",
  running: "info",
  paused: "neutral",
  degraded: "warning",
  failed: "destructive",
  completed: "success",
  cancelled: "neutral",
  blocked: "warning",
  partial: "warning",
  archived: "neutral",
}

/** Shared lifecycle badge for pipelines, jobs, runs, policies, and agents. */
export function StatusBadge({
  status,
  className,
}: {
  status: EntityStatus
  className?: string
}) {
  return (
    <Pill
      tone={STATUS_TONE[status]}
      title={ENTITY_STATUS_DESCRIPTION[status]}
      className={className}
    >
      {status === "running" ? (
        <span className="size-1.5 animate-pulse rounded-full bg-current" aria-hidden />
      ) : null}
      {ENTITY_STATUS_LABEL[status]}
    </Pill>
  )
}

const HEALTH_TONE: Record<Health, Tone> = {
  healthy: "success",
  degraded: "warning",
  unhealthy: "destructive",
  unknown: "neutral",
}

/** Health pill for services, connectors, and assets. */
export function HealthBadge({
  health,
  className,
}: {
  health: Health
  className?: string
}) {
  return (
    <Pill tone={HEALTH_TONE[health]} className={className}>
      <span className="size-1.5 rounded-full bg-current" aria-hidden />
      {HEALTH_LABEL[health]}
    </Pill>
  )
}

const TIER_TONE: Record<StorageTier, Tone> = {
  hot: "orange",
  warm: "warning",
  cold: "sky",
  ai: "violet",
}

/** Storage tier pill (Hot / Warm / Cold / AI). */
export function TierBadge({
  tier,
  className,
}: {
  tier: StorageTier
  className?: string
}) {
  return (
    <Pill tone={TIER_TONE[tier]} className={className}>
      {STORAGE_TIER_LABEL[tier]}
    </Pill>
  )
}

const CLASSIFICATION_TONE: Record<Classification, Tone> = {
  public: "neutral",
  internal: "info",
  confidential: "warning",
  restricted: "destructive",
}

/** Data classification pill. */
export function ClassificationBadge({
  classification,
  className,
}: {
  classification: Classification
  className?: string
}) {
  return (
    <Pill tone={CLASSIFICATION_TONE[classification]} className={className}>
      {CLASSIFICATION_LABEL[classification]}
    </Pill>
  )
}

const AUTONOMY_TONE: Record<AutonomyLevel, Tone> = {
  L1: "neutral",
  L2: "sky",
  L3: "warning",
  L4: "violet",
}

/** Agent autonomy pill. */
export function AutonomyBadge({
  level,
  className,
}: {
  level: AutonomyLevel
  className?: string
}) {
  return (
    <Pill tone={AUTONOMY_TONE[level]} className={className}>
      {AUTONOMY_LABEL[level]}
    </Pill>
  )
}

const SEVERITY_TONE: Record<Severity, Tone> = {
  critical: "destructive",
  high: "orange",
  medium: "warning",
  low: "sky",
  info: "neutral",
}

/** Severity pill for alerts, quality results, and policy violations. */
export function SeverityBadge({
  severity,
  className,
}: {
  severity: Severity
  className?: string
}) {
  return (
    <Pill tone={SEVERITY_TONE[severity]} className={className}>
      {SEVERITY_LABEL[severity]}
    </Pill>
  )
}

const CHECK_TONE: Record<CheckStatus, Tone> = {
  passed: "success",
  warning: "warning",
  failed: "destructive",
}

/** Check-result pill for quality rules, validations, and asset checks. */
export function CheckBadge({
  status,
  className,
}: {
  status: CheckStatus
  className?: string
}) {
  return (
    <Pill tone={CHECK_TONE[status]} className={className}>
      {CHECK_STATUS_LABEL[status]}
    </Pill>
  )
}

const APPROVAL_TONE: Record<ApprovalStatus, Tone> = {
  pending: "warning",
  approved: "success",
  rejected: "destructive",
}

/** Approval pill for agent runs, tools, and policy submissions. */
export function ApprovalBadge({
  status,
  className,
}: {
  status: ApprovalStatus
  className?: string
}) {
  return (
    <Pill tone={APPROVAL_TONE[status]} className={className}>
      {APPROVAL_STATUS_LABEL[status]}
    </Pill>
  )
}

const OUTCOME_TONE: Record<AuditOutcome, Tone> = {
  success: "success",
  denied: "warning",
  error: "destructive",
}

/** Outcome pill for audited actions after policy evaluation. */
export function OutcomeBadge({
  outcome,
  className,
}: {
  outcome: AuditOutcome
  className?: string
}) {
  return (
    <Pill tone={OUTCOME_TONE[outcome]} className={className}>
      {AUDIT_OUTCOME_LABEL[outcome]}
    </Pill>
  )
}

const ALERT_STATUS_TONE: Record<AlertStatus, Tone> = {
  open: "destructive",
  acknowledged: "warning",
  resolved: "success",
}

/** Alert triage pill (Open / Acknowledged / Resolved). */
export function AlertStatusBadge({
  status,
  className,
}: {
  status: AlertStatus
  className?: string
}) {
  return (
    <Pill tone={ALERT_STATUS_TONE[status]} className={className}>
      {ALERT_STATUS_LABEL[status]}
    </Pill>
  )
}

const WORKLOAD_STATUS_TONE: Record<WorkloadStatus, Tone> = {
  queued: "sky",
  running: "info",
  completed: "success",
  cancelled: "neutral",
}

/** Workload queue lifecycle pill. */
export function WorkloadStatusBadge({
  status,
  className,
}: {
  status: WorkloadStatus
  className?: string
}) {
  return (
    <Pill tone={WORKLOAD_STATUS_TONE[status]} className={className}>
      {status === "running" ? (
        <span className="size-1.5 animate-pulse rounded-full bg-current" aria-hidden />
      ) : null}
      {WORKLOAD_STATUS_LABEL[status]}
    </Pill>
  )
}
