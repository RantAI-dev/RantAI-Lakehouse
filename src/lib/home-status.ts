/**
 * The one-line status at the top of Home.
 *
 * It answers "is anything wrong right now?" from four reads (pipelines,
 * alerts, sources, datasets) and nothing else: counts of what needs attention, never
 * totals, throughput or trends. Those are Monitoring → Health; Home had a
 * wall of platform metrics once and it was moved there on purpose.
 *
 * A read that failed or has not finished is never counted as healthy.
 * "All clear" is only said about the reads that were made, and only when
 * each of them succeeded and found nothing.
 *
 * Only the checks that were made are spoken of. A check the person may not
 * see is not passed in at all, so it is named nowhere: not as fine, not as
 * a problem, not as unread, and it does not hold the line in "loading". With
 * no check at all there is no status (`null`) and Home shows no line
 * (ADM-13 review round 2, R2-B1).
 *
 * `datasets` is the overview's `staleAssets`: catalog datasets with no
 * synced rows at all (`routes/overview.rs`), not data that is old. The
 * API has no age-based freshness yet (`freshnessLagSeconds` is null), so
 * the wording says "have no data", never "stale" or "out of date".
 */
export type CheckState = "loading" | "ok" | "error"

export type StatusCheck = {
  state: CheckState
  /** Items needing attention; only meaningful when `state` is "ok". */
  count: number
}

export type HomeStatusInput = {
  /** A check that was not made is left out entirely (see the module doc). */
  pipelines?: StatusCheck
  alerts?: StatusCheck
  sources?: StatusCheck
  datasets?: StatusCheck
}

export type StatusTone = "loading" | "attention" | "unknown" | "clear"

export type HomeStatus = {
  tone: StatusTone
  headline: string
  /** Short phrases joined by the caller: the problems when there are any, otherwise what was found healthy. */
  parts: string[]
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)

/** A check that was made, with the words it uses. */
type NamedCheck = {
  check: StatusCheck
  name: string
  problem: (n: number) => string
  healthy: string
}

export function homeStatus(input: HomeStatusInput): HomeStatus | null {
  // ADM-13 review round 2, R2-B1: only the checks that were made are named.
  // Passing a check the person may not see as "ok" with a count of 0 said
  // "all clear" about a read that never happened.
  const made: NamedCheck[] = []
  const consider = (
    check: StatusCheck | undefined,
    name: string,
    problem: (n: number) => string,
    healthy: string,
  ) => {
    if (check) made.push({ check, name, problem, healthy })
  }
  consider(input.pipelines, "pipelines", (n) => `${n} ${plural(n, "pipeline", "pipelines")} failing`, "no failing pipelines")
  consider(input.alerts, "alerts", (n) => `${n} ${plural(n, "alert", "alerts")} open`, "no open alerts")
  consider(input.sources, "sources", (n) => `${n} ${plural(n, "source", "sources")} unhealthy`, "all sources healthy")
  consider(input.datasets, "datasets", (n) => `${n} ${plural(n, "dataset has", "datasets have")} no data`, "every dataset has data")

  // Nothing was read: there is no status, and Home draws no line.
  if (made.length === 0) return null

  if (made.some(({ check }) => check.state === "loading")) {
    return { tone: "loading", headline: "Checking the lakehouse…", parts: [] }
  }

  const problems: string[] = []
  const fine: string[] = []
  const unread: string[] = []
  let total = 0

  for (const { check, name, problem, healthy } of made) {
    if (check.state === "error") unread.push(name)
    else if (check.count > 0) {
      total += check.count
      problems.push(problem(check.count))
    } else fine.push(healthy)
  }

  const unreadPart = unread.length ? [`could not check ${unread.join(", ")}`] : []

  if (problems.length > 0) {
    // With a problem on screen, what is healthy is noise: only the problems
    // (and any read that failed) are named. The full list is for when the
    // answer is "nothing".
    return {
      tone: "attention",
      headline: `${total} ${plural(total, "thing needs", "things need")} your attention`,
      parts: [...problems, ...unreadPart],
    }
  }
  if (unread.length > 0) {
    return { tone: "unknown", headline: "Status is incomplete", parts: [...fine, ...unreadPart] }
  }
  return { tone: "clear", headline: "All clear", parts: fine }
}
