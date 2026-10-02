/**
 * Reading a pipeline's `schedule` string for people.
 *
 * The API sends one of three shapes (`routes::pipelines::schedule_label`
 * for a Dagster job, the author's own text for a `pl-` pipeline):
 * `"cron: <expr> (<STATE>)"`, a bare five-field cron expression, or
 * `"manual"`. This splits the string into those parts and describes the
 * common cron patterns in words. A pattern it does not recognise returns
 * `null` and the page shows the raw expression. A guessed sentence that is
 * wrong is worse than the expression itself.
 */

export type ParsedSchedule =
  | { kind: "manual" }
  | { kind: "cron"; cron: string; state: string | null }
  | { kind: "other"; raw: string }

const CRON_FIELDS = /^\S+(\s+\S+){4}$/
const LABELLED = /^cron:\s*(.+?)\s*\((\w+)\)\s*$/

export function parseSchedule(raw: string | null | undefined): ParsedSchedule {
  const text = (raw ?? "").trim()
  if (text === "" || text.toLowerCase() === "manual") return { kind: "manual" }
  const labelled = LABELLED.exec(text)
  if (labelled) return { kind: "cron", cron: labelled[1], state: labelled[2] }
  if (CRON_FIELDS.test(text)) return { kind: "cron", cron: text, state: null }
  return { kind: "other", raw: text }
}

const DAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"]

function isInt(field: string): boolean {
  return /^\d+$/.test(field)
}

function hhmm(hour: string, minute: string): string {
  return `${hour.padStart(2, "0")}:${minute.padStart(2, "0")}`
}

function ordinal(n: number): string {
  const tens = n % 100
  if (tens >= 11 && tens <= 13) return `${n}th`
  return `${n}${["th", "st", "nd", "rd"][n % 10] ?? "th"}`
}

/**
 * "Every 15 minutes", "Daily at 03:00", … for the patterns schedules here
 * actually use; `null` for anything else. Times are in the schedule's own
 * timezone, which the API does not send, so callers show the exact next
 * run (from the orchestrator) beside this.
 */
export function describeCron(cron: string): string | null {
  const parts = cron.trim().split(/\s+/)
  if (parts.length !== 5) return null
  const [min, hour, dom, month, dow] = parts
  if (month !== "*") return null

  if (min === "*" && hour === "*" && dom === "*" && dow === "*") return "Every minute"
  const everyMin = /^\*\/(\d+)$/.exec(min)
  if (everyMin && hour === "*" && dom === "*" && dow === "*") {
    return `Every ${everyMin[1]} minutes`
  }
  if (!isInt(min)) return null

  if (hour === "*" && dom === "*" && dow === "*") {
    return min === "0" ? "Hourly" : `Hourly at :${min.padStart(2, "0")}`
  }
  const everyHour = /^\*\/(\d+)$/.exec(hour)
  if (everyHour && dom === "*" && dow === "*") {
    return `Every ${everyHour[1]} hours at :${min.padStart(2, "0")}`
  }
  if (!isInt(hour)) return null

  const at = hhmm(hour, min)
  if (dom === "*" && dow === "*") return `Daily at ${at}`
  if (dom === "*" && isInt(dow) && Number(dow) <= 7) {
    return `Weekly on ${DAYS[Number(dow) % 7]} at ${at}`
  }
  if (dom === "*" && dow === "1-5") return `Weekdays at ${at}`
  if (isInt(dom) && dow === "*") return `Monthly on the ${ordinal(Number(dom))} at ${at}`
  return null
}

/** Whether an orchestrator schedule state means "will fire". */
export function isScheduleRunning(state: string | null): boolean {
  return state === "RUNNING"
}

/**
 * `describeCron`'s clock time is in the schedule's own timezone, which the
 * API does not send, while every other time on the page is local. When the
 * orchestrator gave the next fire time, that instant is the schedule's
 * time read in local time, so the phrase takes the local clock time from
 * it. Only "Daily at" is rewritten: a weekly or monthly schedule can
 * cross midnight between timezones, so its day name would then be wrong,
 * and it keeps the schedule's own time. With no next run, the phrase is
 * left as is.
 */
export function localizeScheduleTime(words: string, nextRunAt: string | null | undefined): string {
  if (!nextRunAt) return words
  const next = new Date(nextRunAt)
  if (Number.isNaN(next.getTime())) return words
  const local = `${String(next.getHours()).padStart(2, "0")}:${String(next.getMinutes()).padStart(2, "0")}`
  return words.replace(/^Daily at \d{2}:\d{2}$/, `Daily at ${local}`)
}
