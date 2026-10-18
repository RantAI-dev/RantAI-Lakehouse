import type { ReportSchedule, ReportSubscription } from "@/services/contracts/reports"

/** ISO weekdays, Monday first, as the API numbers them (1 to 7). */
export const WEEKDAYS = ["Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday", "Sunday"] as const

const two = (n: number) => String(n).padStart(2, "0")

/** `08:05`, the time of day a daily, weekly or monthly report is sent. */
export function clock(hour: number, minute: number): string {
  return `${two(hour)}:${two(minute)}`
}

/** `1st`, `2nd`, `3rd`, `4th`, `11th`, `21st` for the monthly choice. */
export function ordinal(n: number): string {
  const rest = n % 100
  if (rest >= 11 && rest <= 13) return `${n}th`
  return `${n}${({ 1: "st", 2: "nd", 3: "rd" } as Record<number, string>)[n % 10] ?? "th"}`
}

/**
 * A schedule in words (`RPT-1` feature decision 4), for the list and for the
 * form's summary line: "Every Monday at 08:00 (Asia/Jakarta)".
 */
export function describeSchedule(s: ReportSchedule | null, timeZone: string): string {
  if (!s) return "Schedule unknown"
  const zone = ` (${timeZone})`
  switch (s.kind) {
    case "hourly":
      return `Every hour at ${two(s.minute)} minutes past${zone}`
    case "daily":
      return `Every day at ${clock(s.hour, s.minute)}${zone}`
    case "weekly":
      return `Every ${WEEKDAYS[s.weekday - 1] ?? "week"} at ${clock(s.hour, s.minute)}${zone}`
    case "monthly":
      return s.monthDay === "last"
        ? `On the last day of every month at ${clock(s.hour, s.minute)}${zone}`
        : `On the ${ordinal(s.monthDay)} of every month at ${clock(s.hour, s.minute)}${zone}`
  }
}

/** What a report attaches, in words: "PDF and 2 chart files". */
export function describeAttachments(a: ReportSubscription["attachments"]): string {
  const parts: string[] = []
  if (a.pdf) parts.push("PDF")
  if (a.charts.length) parts.push(`${a.charts.length} chart ${a.charts.length === 1 ? "file" : "files"}`)
  return parts.length ? parts.join(" and ") : "Nothing attached"
}

/** The state of a subscription for the list: paused with its reason, or its last send. */
export function describeStatus(s: ReportSubscription): string {
  if (s.paused) return s.pausedReason ? `Paused. ${s.pausedReason}` : "Paused"
  const d = s.lastDelivery
  if (!d) return "Not sent yet"
  const word = d.status === "sent" ? "Sent" : d.status === "skipped" ? "Skipped" : "Failed"
  return d.reason ? `${word}. ${d.reason}` : word
}
