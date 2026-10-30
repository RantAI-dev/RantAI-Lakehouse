import { formatDateTime, formatLagSeconds } from "@/lib/format"
import type { CaptureTableLag, SetupCheck } from "@/services/contracts/connectors"

/** Tone of a table's lag line: a number past the limit is the only warning. */
export type LagTone = "success" | "warning" | "neutral"

/**
 * One table's lag in the API's words (`SRC-4` decision S4-c). A number is shown only when the API sent one: a table
 * nobody reported is "Not known yet", one the orchestrator stopped reporting is "Not known since <time>", and a
 * status this build does not know is shown as sent, never as "up to date".
 */
export function lagWords(lag: Pick<CaptureTableLag, "status" | "seconds" | "since">): string {
  switch (lag.status) {
    case "up_to_date":
      return "Up to date"
    case "behind":
      return typeof lag.seconds === "number" ? `Behind by ${formatLagSeconds(lag.seconds)}` : "Behind"
    case "not_known_yet":
      return "Not known yet"
    case "not_known_since":
      return lag.since ? `Not known since ${formatDateTime(lag.since)}` : "Not known"
    case "paused":
      return "Paused"
    default:
      return lag.status
  }
}

/** The colour of a table's lag: green when up to date, amber when behind or unknown, grey for a pause. */
export function lagTone(lag: Pick<CaptureTableLag, "status">, limitMinutes: number, seconds?: number): LagTone {
  if (lag.status === "up_to_date") return "success"
  if (lag.status === "behind") return typeof seconds === "number" && seconds > limitMinutes * 60 ? "warning" : "neutral"
  if (lag.status === "not_known_since") return "warning"
  return "neutral"
}

/** Whether `value` is a lag limit the API accepts: a whole number of minutes from 1 to 1440 (`SRC-4` S4-d). */
export function isValidLagLimit(value: string): boolean {
  if (!/^\d+$/.test(value.trim())) return false
  const minutes = Number(value)
  return minutes >= 1 && minutes <= 1440
}

const CHECK_ORDER: Record<string, number> = { missing: 0, unchecked: 1, ok: 2 }

/** The setup checks with the missing ones first (what to fix), then the unread ones, then the passing ones. Stable. */
export function orderedChecks(checks: readonly SetupCheck[]): SetupCheck[] {
  return checks
    .map((check, index) => ({ check, index }))
    .sort(
      (a, b) =>
        (CHECK_ORDER[a.check.status] ?? 1) - (CHECK_ORDER[b.check.status] ?? 1) || a.index - b.index
    )
    .map(({ check }) => check)
}
