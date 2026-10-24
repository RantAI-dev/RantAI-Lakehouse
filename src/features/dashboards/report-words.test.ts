import { describe, expect, it } from "bun:test"
import type { ReportSubscription } from "@/services/contracts/reports"
import { clock, describeAttachments, describeSchedule, describeStatus, ordinal } from "./report-words"

describe("describeSchedule", () => {
  it("puts every kind in words with its time zone", () => {
    const zone = "Asia/Jakarta"
    expect(describeSchedule({ kind: "hourly", minute: 5 }, zone)).toBe("Every hour at 05 minutes past (Asia/Jakarta)")
    expect(describeSchedule({ kind: "daily", hour: 8, minute: 0 }, zone)).toBe("Every day at 08:00 (Asia/Jakarta)")
    expect(describeSchedule({ kind: "weekly", weekday: 1, hour: 8, minute: 0 }, zone)).toBe("Every Monday at 08:00 (Asia/Jakarta)")
    expect(describeSchedule({ kind: "weekly", weekday: 7, hour: 18, minute: 30 }, "UTC")).toBe("Every Sunday at 18:30 (UTC)")
    expect(describeSchedule({ kind: "monthly", monthDay: 1, hour: 9, minute: 15 }, zone)).toBe("On the 1st of every month at 09:15 (Asia/Jakarta)")
    expect(describeSchedule({ kind: "monthly", monthDay: "last", hour: 23, minute: 30 }, zone)).toBe("On the last day of every month at 23:30 (Asia/Jakarta)")
    expect(describeSchedule(null, zone)).toBe("Schedule unknown")
  })
  it("writes ordinals, with the teens as th", () => {
    expect([1, 2, 3, 4, 11, 12, 13, 21, 22, 23, 28].map(ordinal)).toEqual(["1st", "2nd", "3rd", "4th", "11th", "12th", "13th", "21st", "22nd", "23rd", "28th"])
    expect(clock(7, 5)).toBe("07:05")
  })
})

const base: ReportSubscription = {
  id: "s1", board: "b", ownerId: "u", schedule: null, timeZone: "UTC",
  attachments: { pdf: { paper: "a4", orientation: "portrait" }, charts: [{ chart: "c1", format: "xlsx" }, { chart: "c2", format: "csv" }] },
  filters: null, params: null, grain: null, skipWhenEmpty: false, paused: false, pausedReason: null,
  consecutiveFailures: 0, nextRunAt: "", createdAt: "", recipients: [],
}

describe("describeAttachments and describeStatus", () => {
  it("counts what is attached", () => {
    expect(describeAttachments(base.attachments)).toBe("PDF and 2 chart files")
    expect(describeAttachments({ pdf: null, charts: [{ chart: "c", format: "csv" }] })).toBe("1 chart file")
    expect(describeAttachments({ charts: [] })).toBe("Nothing attached")
  })
  it("says paused with the reason, else the last send with its reason", () => {
    expect(describeStatus(base)).toBe("Not sent yet")
    expect(describeStatus({ ...base, paused: true, pausedReason: "Paused after 5 failed sends in a row." })).toBe("Paused. Paused after 5 failed sends in a row.")
    expect(describeStatus({ ...base, lastDelivery: { id: "d", startedAt: "", finishedAt: "", status: "failed", origin: "schedule", recipientCount: 0, reason: "The PDF took too long to make." } })).toBe("Failed. The PDF took too long to make.")
    expect(describeStatus({ ...base, lastDelivery: { id: "d", startedAt: "", finishedAt: "", status: "sent", origin: "manual", recipientCount: 2, reason: null } })).toBe("Sent")
  })
})
