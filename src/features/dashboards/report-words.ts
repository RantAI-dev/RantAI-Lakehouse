import { formatDateTime } from "@/lib/format"
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

/**
 * Shown on a subscription that is not being sent for want of e-mail (`RPT-2`,
 * `DEC-18`, QA `LH-RPT-1-06`) — the wording the product decided on for it.
 */
export const NOT_SENT_EMAIL = "Not sent: e-mail is not set up on this installation."

/**
 * Why a subscription is not being sent right now, in one line, or `null` when
 * the list says nothing of the kind — a cause this build does not know
 * included, so an older console keeps showing the status line.
 *
 * The time is the start of the last attempt that failed for this reason, and it
 * is left out when there is none or when [`formatDateTime`] cannot read it.
 */
export function describeNotSent(s: ReportSubscription): string | null {
  if (s.notSent?.cause !== "email_not_set_up") return null
  const at = s.notSent.lastAttemptAt
  if (typeof at !== "string") return NOT_SENT_EMAIL
  const when = formatDateTime(at)
  return when === "—" ? NOT_SENT_EMAIL : `${NOT_SENT_EMAIL} Last attempt: ${when}.`
}

/** The state of a subscription for the list: paused with its reason, or its last send. */
export function describeStatus(s: ReportSubscription): string {
  if (s.paused) return s.pausedReason ? `Paused. ${s.pausedReason}` : "Paused"
  const d = s.lastDelivery
  if (!d) return "Not sent yet"
  const word = d.status === "sent" ? "Sent" : d.status === "skipped" ? "Skipped" : "Failed"
  return d.reason ? `${word}. ${d.reason}` : word
}

/**
 * The short name of `zone` now: `WIB`, `EST`, or `UTC+7` when the runtime has
 * no letters for it. Tried in English first and in Indonesian second, because
 * only the Indonesian data names `Asia/Jakarta` `WIB`.
 */
export function zoneAbbreviation(zone: string): string {
  for (const locale of ["en", "id"]) {
    try {
      const name = new Intl.DateTimeFormat(locale, { timeZone: zone, timeZoneName: "short" })
        .formatToParts(new Date())
        .find((p) => p.type === "timeZoneName")?.value
      if (name && /^[A-Z]{2,5}$/.test(name)) return name
    } catch {
      return zone
    }
  }
  try {
    const name = new Intl.DateTimeFormat("en", { timeZone: zone, timeZoneName: "shortOffset" })
      .formatToParts(new Date())
      .find((p) => p.type === "timeZoneName")?.value
    return name ? name.replace(/^GMT/, "UTC") : zone
  } catch {
    return zone
  }
}

/**
 * A digest's schedule in one short line for the Alerts list (`BI-65`):
 * "Mondays · 08:00 WIB". A digest saved before schedules has none and is sent
 * daily at 08:00 in the report time zone, which the line says.
 */
export function describeDigestSchedule(s: ReportSchedule | null | undefined, timeZone: string | null | undefined): string {
  if (!s) return "Daily · 08:00 (report time zone)"
  if (s.kind === "hourly") return `Hourly · :${two(s.minute)}`
  const at = `${clock(s.hour, s.minute)}${timeZone ? ` ${zoneAbbreviation(timeZone)}` : ""}`
  switch (s.kind) {
    case "daily":
      return `Daily · ${at}`
    case "weekly":
      return `${WEEKDAYS[s.weekday - 1] ?? "Weekly"}s · ${at}`
    case "monthly":
      return `${s.monthDay === "last" ? "Last day of the month" : `Monthly on the ${ordinal(s.monthDay)}`} · ${at}`
  }
}
