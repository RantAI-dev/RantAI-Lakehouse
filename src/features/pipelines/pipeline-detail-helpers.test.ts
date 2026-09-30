import { describe, expect, test } from "bun:test"
import { describeTransform } from "@/lib/transform-draft"
import type { PipelineRun } from "@/services/contracts/pipelines"
import { describeCron, localizeScheduleTime, parseSchedule } from "./pipeline-schedule"
import { runDurationMs, summarizeRuns } from "./pipeline-run-stats"
import { failureLines, filterLogLines, formatLogOffset, type LogLine } from "./run-logs"

function run(status: PipelineRun["status"], seconds: number | null, id: string = status): PipelineRun {
  return {
    id,
    pipelineId: "p",
    status,
    startedAt: "2026-09-27T04:00:00.000Z",
    endedAt: seconds === null ? undefined : new Date(Date.parse("2026-09-27T04:00:00.000Z") + seconds * 1000).toISOString(),
    processed: null,
    accepted: null,
    rejected: null,
    retried: null,
    costUnits: null,
    durationSeconds: seconds,
  }
}

describe("parseSchedule", () => {
  test("a Dagster schedule label splits into its cron and its state", () => {
    expect(parseSchedule("cron: */15 * * * * (RUNNING)")).toEqual({
      kind: "cron",
      cron: "*/15 * * * *",
      state: "RUNNING",
    })
  })
  test("manual and empty schedules both read as manual", () => {
    expect(parseSchedule("manual")).toEqual({ kind: "manual" })
    expect(parseSchedule("")).toEqual({ kind: "manual" })
  })
  test("a bare five-field cron has no state, and free text stays raw", () => {
    expect(parseSchedule("0 3 * * *")).toEqual({ kind: "cron", cron: "0 3 * * *", state: null })
    expect(parseSchedule("hourly-ish")).toEqual({ kind: "other", raw: "hourly-ish" })
  })
})

describe("describeCron", () => {
  test("the patterns the shipped schedules use read as words", () => {
    expect(describeCron("*/15 * * * *")).toBe("Every 15 minutes")
    expect(describeCron("0 3 * * *")).toBe("Daily at 03:00")
    expect(describeCron("0 * * * *")).toBe("Hourly")
    expect(describeCron("30 */6 * * *")).toBe("Every 6 hours at :30")
    expect(describeCron("0 9 * * 1")).toBe("Weekly on Monday at 09:00")
    expect(describeCron("0 9 1 * *")).toBe("Monthly on the 1st at 09:00")
  })
  test("a pattern it does not recognise returns null instead of a guess", () => {
    expect(describeCron("0 3 * 1 *")).toBeNull()
    expect(describeCron("5,35 * * * *")).toBeNull()
    expect(describeCron("not cron")).toBeNull()
  })
})

describe("summarizeRuns", () => {
  test("the success rate counts only finished runs, and is null before any finish", () => {
    expect(summarizeRuns([run("running", null)]).successRate).toBeNull()
    const s = summarizeRuns([run("failed", 4, "a"), run("completed", 10, "b"), run("cancelled", 2, "c")])
    expect(s.finished).toBe(2)
    expect(s.successRate).toBe(0.5)
  })
  test("durations are percentiles of completed runs only", () => {
    const s = summarizeRuns([run("completed", 10, "a"), run("completed", 20, "b"), run("failed", 999, "c")])
    expect(s.p50Ms).toBe(10_000)
    expect(s.p95Ms).toBe(20_000)
  })
  test("the streak counts how many newest runs share the newest status", () => {
    const s = summarizeRuns([run("failed", 1, "a"), run("failed", 1, "b"), run("completed", 1, "c")])
    expect(s.streak).toEqual({ status: "failed", count: 2 })
  })
  test("a live run is measured up to now and a finished run without timestamps is unknown", () => {
    const live = { ...run("running", null), startedAt: "2026-09-27T04:00:00.000Z" }
    expect(runDurationMs(live, Date.parse("2026-09-27T04:00:30.000Z"))).toBe(30_000)
    expect(runDurationMs({ ...run("failed", null), startedAt: "" })).toBeNull()
  })
  test("a finished run is measured from its exact timestamps, not the rounded seconds", () => {
    const r = { ...run("completed", 14), endedAt: "2026-09-27T04:00:13.700Z", durationSeconds: 14 }
    expect(runDurationMs(r)).toBe(13_700)
    expect(runDurationMs({ ...r, startedAt: "" })).toBe(14_000)
  })
})

describe("run logs", () => {
  const line = (level: string, message: string, stepKey: string | null = null, ts = 0): LogLine => ({
    ts,
    level,
    stepKey,
    message,
  })
  test("the op's own error comes before the engine's generic failure lines", () => {
    const lines = [
      line("ERROR", "export of 'x' failed: 401 Client Error", "run_gold_export"),
      line("ERROR", 'Execution of step "run_gold_export" failed.', "run_gold_export"),
      line("ERROR", 'Execution of run for "gold_export_job" failed. Steps failed: [\'run_gold_export\'].'),
    ]
    expect(failureLines(lines).map((l) => l.message)).toEqual(["export of 'x' failed: 401 Client Error"])
  })
  test("a log with only generic errors still shows them", () => {
    const lines = [line("ERROR", 'Execution of step "s" failed.')]
    expect(failureLines(lines)).toHaveLength(1)
  })
  test("filtering drops empty lifecycle markers and honours level, step and text", () => {
    const lines = [
      line("INFO", ""),
      line("DEBUG", "launching", "s1"),
      line("INFO", "exporting 1 mart", "s1"),
      line("ERROR", "boom", "s2"),
    ]
    expect(filterLogLines(lines, { level: "all", stepKey: null, query: "" })).toHaveLength(3)
    expect(filterLogLines(lines, { level: "info", stepKey: null, query: "" })).toHaveLength(2)
    expect(filterLogLines(lines, { level: "all", stepKey: "s1", query: "" })).toHaveLength(2)
    expect(filterLogLines(lines, { level: "all", stepKey: null, query: "BOOM" })).toHaveLength(1)
  })
  test("offsets are measured from the run's start", () => {
    expect(formatLogOffset(3214, 0)).toBe("+00:03.214")
    expect(formatLogOffset(3_723_000, 0)).toBe("+1:02:03.000")
    expect(formatLogOffset(-5, 0)).toBe("+00:00.000")
  })
})

describe("describeTransform", () => {
  test("each verb reads back as a short phrase, and unknown text returns null", () => {
    expect(describeTransform("dedupe(nama_event)")).toEqual({ verb: "dedupe", detail: "one row per nama_event" })
    expect(describeTransform("rename(a,b)")).toEqual({ verb: "rename", detail: "a → b" })
    expect(describeTransform("cast(tahun,Int32)")).toEqual({ verb: "cast", detail: "tahun as Int32" })
    expect(describeTransform("select(tahun,nama_event)")).toEqual({ verb: "select", detail: "tahun, nama_event" })
    expect(describeTransform("filter(tahun >= '2020')")).toEqual({ verb: "filter", detail: "tahun >= '2020'" })
    expect(describeTransform("drop(x)")).toBeNull()
  })
})

describe("localizeScheduleTime", () => {
  test("the clock time comes from the next fire instant, read locally", () => {
    const next = "2026-09-28T04:00:00.000Z"
    const d = new Date(next)
    const local = `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`
    expect(localizeScheduleTime("Daily at 04:00", next)).toBe(`Daily at ${local}`)
  })
  test("without a next run, or with no clock time, the phrase is unchanged", () => {
    expect(localizeScheduleTime("Daily at 04:00", null)).toBe("Daily at 04:00")
    expect(localizeScheduleTime("Every 15 minutes", "2026-09-28T04:00:00.000Z")).toBe("Every 15 minutes")
    expect(localizeScheduleTime("Weekly on Monday at 22:00", "2026-09-28T22:00:00.000Z")).toBe(
      "Weekly on Monday at 22:00"
    )
  })
})
