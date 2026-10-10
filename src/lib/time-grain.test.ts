import assert from "node:assert/strict"
import test from "node:test"

import {
  GRAINS,
  bucketFiltersDashboard,
  bucketLabel,
  bucketListsRecords,
  bucketRange,
  calendarFirstDay,
  columnKindOfType,
  grainChoices,
  grainFits,
  limitAfterGrainPick,
  limitHasEffect,
  switchChoices,
} from "./time-grain"

test("a column type becomes the kind the server decides", () => {
  assert.equal(columnKindOfType("Date"), "date")
  assert.equal(columnKindOfType("Nullable(Date32)"), "date")
  assert.equal(columnKindOfType("LowCardinality(Nullable(String))"), "text")
  assert.equal(columnKindOfType("DateTime('Asia/Jakarta')"), "datetime")
  assert.equal(columnKindOfType("Nullable(DateTime64(3, 'UTC'))"), "datetime")
  assert.equal(columnKindOfType("UInt64"), "number")
  assert.equal(columnKindOfType("Decimal(18, 2)"), "number")
  assert.equal(columnKindOfType("Interval(Day)"), "text")
})

test("the select offers all thirteen on a timestamp and drops the time grains on a date", () => {
  assert.equal(grainChoices("line", "datetime").length, 13)
  const onDate = grainChoices("bar", "date")
  assert.deepEqual(onDate.filter((g) => ["minute", "hour", "hour_of_day"].includes(g)), [])
  assert.equal(onDate.length, 10)
  assert.deepEqual(grainChoices("bar", "text"), [])
  assert.deepEqual(grainChoices("bar", "number"), [])
  assert.deepEqual(grainChoices("bar", undefined), [])
})

test("only the listed chart kinds take a grain, and a calendar takes day only", () => {
  for (const kind of ["scatter", "table", "kpi", "gauge", "text", "boxplot", "geomap", "sankey"]) {
    assert.deepEqual(grainChoices(kind, "datetime"), [], kind)
  }
  assert.deepEqual(grainChoices("calendar", "datetime"), ["day"])
  assert.deepEqual(grainChoices("calendar", "date"), ["day"])
  assert.equal(grainFits("month", "calendar", "date"), false)
})

test("the dashboard switch offers hour and minute only when every grained chart is a timestamp", () => {
  assert.deepEqual(switchChoices(["date", "date"]), ["day", "week", "month", "quarter", "year"])
  assert.deepEqual(switchChoices(["datetime", "date"]), ["day", "week", "month", "quarter", "year"])
  assert.equal(switchChoices(["datetime", "datetime"]).length, 7)
  assert.deepEqual(switchChoices([]), ["day", "week", "month", "quarter", "year"])
})

test("buckets read as month, quarter, year, weekday, week and hour labels", () => {
  assert.equal(bucketLabel("month", "2026-03-01"), "Mar 2026")
  assert.equal(bucketLabel("quarter", "2026-01-01"), "Q1 2026")
  assert.equal(bucketLabel("quarter", "2026-10-01"), "Q4 2026")
  assert.equal(bucketLabel("year", "2026-01-01"), "2026")
  assert.equal(bucketLabel("day", "2026-03-09"), "2026-03-09")
  assert.equal(bucketLabel("hour", "2026-03-09 14:00:00"), "9 Mar 14:00")
  assert.equal(bucketLabel("minute", "2026-03-09 14:05:00"), "9 Mar 14:05")
  assert.equal(bucketLabel("day_of_week", 1), "Mon")
  assert.equal(bucketLabel("day_of_week", "7"), "Sun")
  assert.equal(bucketLabel("month_of_year", 1), "Jan")
  assert.equal(bucketLabel("quarter_of_year", 3), "Q3")
  assert.equal(bucketLabel("hour_of_day", 7), "07:00")
  assert.equal(bucketLabel("week_of_year", 12), "W12")
  assert.equal(bucketLabel("day_of_month", 31), "31")
})

test("a week is labelled by its first day, whichever day the week starts on", () => {
  // The server puts the bucket on the configured first day; the label just reads it.
  assert.equal(bucketLabel("week", "2026-03-09"), "9 Mar 2026") // a Monday
  assert.equal(bucketLabel("week", "2026-03-08"), "8 Mar 2026") // a Sunday
})

test("an empty bucket and text that is not a date are shown, not invented", () => {
  assert.equal(bucketLabel("month", null), "No date")
  assert.equal(bucketLabel("month", ""), "No date")
  assert.equal(bucketLabel("month", "soon"), "soon")
  assert.equal(bucketLabel("day_of_week", 9), "9")
})

test("a month, quarter and year cover their first to their last day", () => {
  assert.deepEqual(bucketRange("month", "2026-03-01"), { from: "2026-03-01", to: "2026-03-31" })
  assert.deepEqual(bucketRange("month", "2026-04-01"), { from: "2026-04-01", to: "2026-04-30" })
  assert.deepEqual(bucketRange("quarter", "2026-01-01"), { from: "2026-01-01", to: "2026-03-31" })
  assert.deepEqual(bucketRange("quarter", "2026-10-01"), { from: "2026-10-01", to: "2026-12-31" })
  assert.deepEqual(bucketRange("year", "2026-01-01"), { from: "2026-01-01", to: "2026-12-31" })
  assert.deepEqual(bucketRange("day", "2026-03-09"), { from: "2026-03-09", to: "2026-03-09" })
})

test("February ends on the 29th in a leap year and the 28th otherwise", () => {
  assert.deepEqual(bucketRange("month", "2028-02-01"), { from: "2028-02-01", to: "2028-02-29" })
  assert.deepEqual(bucketRange("month", "2026-02-01"), { from: "2026-02-01", to: "2026-02-28" })
  assert.deepEqual(bucketRange("month", "2100-02-01"), { from: "2100-02-01", to: "2100-02-28" })
})

test("a week covers seven days from its first day, a Monday or a Sunday, across a month end", () => {
  assert.deepEqual(bucketRange("week", "2026-03-09"), { from: "2026-03-09", to: "2026-03-15" })
  assert.deepEqual(bucketRange("week", "2026-03-08"), { from: "2026-03-08", to: "2026-03-14" })
  assert.deepEqual(bucketRange("week", "2026-12-28"), { from: "2026-12-28", to: "2027-01-03" })
})

test("minute, hour, a part and a non-date have no range, and only a truncation lists records", () => {
  assert.equal(bucketRange("hour", "2026-03-09 14:00:00"), null)
  assert.equal(bucketRange("minute", "2026-03-09 14:05:00"), null)
  assert.equal(bucketRange("day_of_week", 1), null)
  assert.equal(bucketRange("month", "soon"), null)
  assert.equal(bucketRange("month", "2026-02-31"), null)
  assert.equal(bucketFiltersDashboard("month"), true)
  assert.equal(bucketFiltersDashboard("hour"), false)
  assert.equal(bucketListsRecords("hour"), true)
  assert.equal(bucketListsRecords("day_of_week"), false)
  assert.equal(GRAINS.filter(bucketListsRecords).length, 7)
})

test("the calendar's first day follows the setting", () => {
  assert.equal(calendarFirstDay("monday"), 1)
  assert.equal(calendarFirstDay("sunday"), 0)
  assert.equal(calendarFirstDay(undefined), 1)
})

test("choosing a truncation lifts an untouched limit to the grained maximum and keeps a chosen one", () => {
  assert.equal(limitAfterGrainPick("day", 20, false), 1000)
  assert.equal(limitAfterGrainPick("month", 20, true), 20, "the person set it")
  assert.equal(limitAfterGrainPick("month", 50, false), 50)
  assert.equal(limitAfterGrainPick("hour_of_day", 20, false), 20, "a part never needs more")
  assert.equal(limitAfterGrainPick("", 20, false), 20)
})

test("the limit field is hidden for a part ordered by bucket and shown otherwise", () => {
  assert.equal(limitHasEffect("hour_of_day", "none"), false)
  assert.equal(limitHasEffect("day_of_week", "none"), false)
  assert.equal(limitHasEffect("hour_of_day", "desc"), true, "top N keeps its limit")
  assert.equal(limitHasEffect("month", "none"), true)
  assert.equal(limitHasEffect("", "none"), true)
})
