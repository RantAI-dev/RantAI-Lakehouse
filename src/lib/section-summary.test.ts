import { describe, expect, it } from "bun:test"
import { missingSection, NOT_SET, sectionSummaries, type SummaryInput } from "./section-summary"

const base: SummaryInput = {
  kind: "bar", rowsMode: false, source: "mart_visits", joins: 0, dimension: "", grain: "", breakdown: "",
  groupings: [], rowColumns: [], pivot: { rows: [], columns: [], values: [] }, measures: [], mapLabel: "", lat: "", lon: "",
  sort: "Highest first", limit: 20, filters: 0, other: false,
}
const sum = (over: Partial<SummaryInput>) => sectionSummaries({ ...base, ...over })

describe("sectionSummaries: Data", () => {
  it("names the source and counts the joins", () => {
    expect(sum({}).data).toBe("mart_visits")
    expect(sum({ joins: 1 }).data).toBe("mart_visits · 1 join")
    expect(sum({ joins: 2 }).data).toBe("mart_visits · 2 joins")
  })
  it("says nothing is chosen yet when there is no source", () => {
    expect(sum({ source: "" }).data).toBe(NOT_SET)
  })
})

describe("sectionSummaries: What to show", () => {
  it("lists a bar's dimension, breakdown and measures with the labels used elsewhere", () => {
    expect(sum({ dimension: "provinsi", measures: ["visitors", "avg of stay"] }).show).toBe("provinsi · visitors, avg of stay")
    expect(sum({ dimension: "provinsi", breakdown: "category", measures: ["visitors"] }).show).toBe("provinsi · split by category · visitors")
  })
  it("says the grain of a date dimension and writes a joined column as table.column", () => {
    expect(sum({ dimension: "visit_date", grain: "month", measures: ["visitors"] }).show).toBe("visit_date by month · visitors")
    expect(sum({ dimension: "regions__name", measures: ["total visitors"] }).show).toBe("regions.name · total visitors")
  })
  it("lists a grouped data table's groupings, then its measures", () => {
    expect(sum({ kind: "table", groupings: ["provinsi", "category"], measures: ["visitors"] }).show).toBe("provinsi, category · visitors")
  })
  it("counts the columns of a raw table", () => {
    expect(sum({ kind: "table", rowsMode: true, rowColumns: ["a", "b", "c"] }).show).toBe("rows: 3 columns")
    expect(sum({ kind: "table", rowsMode: true, rowColumns: ["a"] }).show).toBe("rows: 1 column")
  })
  it("lists a pivot's rows, columns and values", () => {
    expect(sum({ kind: "pivot", pivot: { rows: ["provinsi"], columns: ["category"], values: ["visitors"] } }).show).toBe("rows provinsi · columns category · values visitors")
    expect(sum({ kind: "pivot", pivot: { rows: ["provinsi"], columns: [], values: [] } }).show).toBe("rows provinsi")
  })
  it("shows a KPI's measure alone", () => {
    expect(sum({ kind: "kpi", measures: ["avg of stay"] }).show).toBe("avg of stay")
  })
  it("shows a region map's map, region column and measure, and a point map's map and coordinates", () => {
    expect(sum({ kind: "geomap", mapLabel: "Indonesia provinces", dimension: "provinsi", measures: ["visitors"] }).show).toBe("Indonesia provinces · provinsi · visitors")
    expect(sum({ kind: "pointmap", mapLabel: "Indonesia provinces", lat: "lat", lon: "lon", measures: ["visitors"] }).show).toBe("Indonesia provinces · lat, lon · visitors")
  })
  it("says nothing is picked yet while a kind has no picks, and has nothing for a text note", () => {
    expect(sum({}).show).toBe(NOT_SET)
    expect(sum({ kind: "text" }).show).toBe(NOT_SET)
  })
})

describe("sectionSummaries: Sort and filter", () => {
  it("joins the sort, the limit and the filters", () => {
    expect(sum({ limit: 5, filters: 1 }).sortAndFilter).toBe("highest first · top 5 · 1 filter")
    expect(sum({ sort: "Lowest first", limit: 20, filters: 3 }).sortAndFilter).toBe("lowest first · top 20 · 3 filters")
  })
  it("says so when the limit follows a parameter, when 'Other' is on, and when there is no filter", () => {
    expect(sum({ limit: "param", other: true }).sortAndFilter).toBe("highest first · top N from a parameter · rest as Other · no filters")
  })
  it("leaves out what the kind does not have", () => {
    expect(sum({ kind: "kpi", sort: "", limit: null, filters: 2 }).sortAndFilter).toBe("2 filters")
    expect(sum({ kind: "kpi", sort: "", limit: null }).sortAndFilter).toBe("no filters")
  })
})

describe("missingSection", () => {
  it("puts the source and the joins in section 1", () => {
    expect(missingSection("Data source")).toBe(1)
    expect(missingSection("Join 2 columns")).toBe(1)
  })
  it("puts the chart's own picks in section 2", () => {
    for (const label of ["Dimension (X)", "Measure (Y)", "Group by", "Latitude", "Columns", "Rows", "Values", "Dimension bin width"]) expect(missingSection(label)).toBe(2)
  })
  it("puts steps, questions and a KPI's comparison in section 4", () => {
    for (const label of ["Step 1 value", "Question 2 measure", "Date column", "Goal"]) expect(missingSection(label)).toBe(4)
  })
})
