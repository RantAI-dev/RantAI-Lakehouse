// BI-65 decision 5: one schedule control for the Subscriptions dialog and the
// digest form. These read what the draft helpers say and render the fields.
import { afterEach, describe, expect, it } from "bun:test"
import { cleanup, render, screen } from "@testing-library/react"
import {
  DEFAULT_SCHEDULE, draftFromSchedule, LEGACY_DIGEST_SCHEDULE, ScheduleFields, scheduleOfDraft, zoneOfDraft,
} from "./schedule-fields"

afterEach(cleanup)

describe("the schedule draft", () => {
  it("round-trips every kind of schedule", () => {
    for (const s of [
      { kind: "hourly", minute: 15 },
      { kind: "daily", hour: 8, minute: 5 },
      { kind: "weekly", weekday: 3, hour: 18, minute: 30 },
      { kind: "monthly", monthDay: "last", hour: 7, minute: 0 },
      { kind: "monthly", monthDay: 12, hour: 7, minute: 0 },
    ] as const) {
      expect(scheduleOfDraft(draftFromSchedule(s, ""))).toEqual(s)
    }
  })
  it("starts every Monday at 08:00 and reads a digest without a schedule as daily at 08:00", () => {
    expect(scheduleOfDraft(draftFromSchedule(DEFAULT_SCHEDULE, ""))).toEqual({ kind: "weekly", weekday: 1, hour: 8, minute: 0 })
    expect(scheduleOfDraft(draftFromSchedule(LEGACY_DIGEST_SCHEDULE, ""))).toEqual({ kind: "daily", hour: 8, minute: 0 })
  })
  it("has no schedule while the time is blank, and an empty zone is the report zone", () => {
    const d = { ...draftFromSchedule(DEFAULT_SCHEDULE, ""), time: "" }
    expect(scheduleOfDraft(d)).toBeNull()
    expect(zoneOfDraft(d, "Asia/Jakarta")).toBe("Asia/Jakarta")
    expect(zoneOfDraft({ ...d, zone: "UTC" }, "Asia/Jakarta")).toBe("UTC")
  })
})

describe("ScheduleFields", () => {
  it("shows Repeat, Day, Time and Time zone for a weekly schedule", () => {
    render(<ScheduleFields id="t" draft={draftFromSchedule(DEFAULT_SCHEDULE, "UTC")} onChange={() => {}} reportZone="UTC" />)
    for (const label of ["Repeat", "Day", "Time", "Time zone"]) expect(screen.getByText(label)).toBeTruthy()
    expect((screen.getByLabelText("Time") as HTMLInputElement).value).toBe("08:00")
  })
  it("asks for the minute, and no time, on an hourly schedule", () => {
    render(<ScheduleFields id="t" draft={draftFromSchedule({ kind: "hourly", minute: 5 }, "UTC")} onChange={() => {}} reportZone="UTC" />)
    expect(screen.getByText("Minutes past the hour")).toBeTruthy()
    expect(screen.queryByText("Day")).toBeNull()
  })
})
