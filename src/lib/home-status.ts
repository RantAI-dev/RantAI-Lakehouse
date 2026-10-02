/**
 * The one-line status at the top of Home.
 *
 * It answers "is anything wrong right now?" from four reads (pipelines,
 * alerts, sources, datasets) and nothing else: counts of what needs attention, never
 * totals, throughput or trends. Those are Monitoring → Health; Home had a
 * wall of platform metrics once and it was moved there on purpose.
 *
 * A read that failed or has not finished is never counted as healthy.
 * "All clear" is only said when every read succeeded and found nothing.
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
  pipelines: StatusCheck
  alerts: StatusCheck
  sources: StatusCheck
  datasets: StatusCheck
}

export type StatusTone = "loading" | "attention" | "unknown" | "clear"

export type HomeStatus = {
  tone: StatusTone
  headline: string
  /** Short phrases joined by the caller: the problems when there are any, otherwise what was found healthy. */
  parts: string[]
}

const plural = (n: number, one: string, many: string) => (n === 1 ? one : many)

export function homeStatus({ pipelines, alerts, sources, datasets }: HomeStatusInput): HomeStatus {
  const checks = [pipelines, alerts, sources, datasets]
  if (checks.some((c) => c.state === "loading")) {
    return { tone: "loading", headline: "Checking the lakehouse…", parts: [] }
  }

  const problems: string[] = []
  const fine: string[] = []
  const unread: string[] = []
  let total = 0

  const add = (check: StatusCheck, name: string, problem: (n: number) => string, healthy: string) => {
    if (check.state === "error") unread.push(name)
    else if (check.count > 0) {
      total += check.count
      problems.push(problem(check.count))
    } else fine.push(healthy)
  }
  add(pipelines, "pipelines", (n) => `${n} ${plural(n, "pipeline", "pipelines")} failing`, "no failing pipelines")
  add(alerts, "alerts", (n) => `${n} ${plural(n, "alert", "alerts")} open`, "no open alerts")
  add(sources, "sources", (n) => `${n} ${plural(n, "source", "sources")} unhealthy`, "all sources healthy")
  add(datasets, "datasets", (n) => `${n} ${plural(n, "dataset has", "datasets have")} no data`, "every dataset has data")

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
