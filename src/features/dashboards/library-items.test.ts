import { describe, expect, it } from "bun:test"

import {
  boardItem, countLabel, factOf, filterItems, reportItem, sortItems, templateItem, type LibraryItem, type SortColumn,
} from "./library-items"

// BI-59 T2: one list of dashboards, reports and templates.

const main = boardItem({ id: "default", name: "Main", builtin: true, chartCount: 13 })
const qa = boardItem({
  id: "b1", name: "QA BI", createdBy: "Rina", updatedAt: "2026-11-13T10:00:00Z", chartCount: 3,
  folderId: "f_sales", publicToken: "tok",
})
const auto = boardItem({
  id: "b2", name: "qa-auto embed", createdBy: "Ana", updatedAt: "2026-11-12T10:00:00Z", chartCount: 1, embedEnabled: true,
})
const report = reportItem({
  id: "r1", title: "Wisman 2020-2024", folderId: "f_sales", createdBy: "Bima", updatedBy: "Bima",
  updatedAt: "2026-11-14T08:00:00Z", blockCount: 4, chartCount: 2,
})
const template = templateItem({
  id: "t1", name: "Sales overview", sourceBoard: "b1", sourceBoardName: "QA BI",
  chartCount: 3, createdBy: "Rina", createdAt: "2026-11-11T00:00:00Z",
})
const ALL: LibraryItem[] = [main, qa, auto, report, template]
const pathOf = (id: string) => (id === "f_sales" ? "Sales" : "")
const names = (items: LibraryItem[]) => items.map((i) => i.name)

describe("filterItems", () => {
  it("keeps only the chosen kind", () => {
    expect(names(filterItems(ALL, { kind: "dashboard", folder: null, query: "" }, pathOf))).toEqual(["Main", "QA BI", "qa-auto embed"])
    expect(names(filterItems(ALL, { kind: "report", folder: null, query: "" }, pathOf))).toEqual(["Wisman 2020-2024"])
    expect(names(filterItems(ALL, { kind: "template", folder: null, query: "" }, pathOf))).toEqual(["Sales overview"])
    expect(filterItems(ALL, { kind: "all", folder: null, query: "" }, pathOf)).toHaveLength(5)
  })

  it("narrows to one folder, or to no folder, and drops templates under either", () => {
    expect(names(filterItems(ALL, { kind: "all", folder: "f_sales", query: "" }, pathOf))).toEqual(["QA BI", "Wisman 2020-2024"])
    expect(names(filterItems(ALL, { kind: "all", folder: "", query: "" }, pathOf))).toEqual(["Main", "qa-auto embed"])
  })

  it("searches names, owners, folder paths, types and a template's source", () => {
    expect(names(filterItems(ALL, { kind: "all", folder: null, query: "bima" }, pathOf))).toEqual(["Wisman 2020-2024"])
    expect(names(filterItems(ALL, { kind: "all", folder: null, query: "sales" }, pathOf))).toEqual(["QA BI", "Wisman 2020-2024", "Sales overview"])
    expect(names(filterItems(ALL, { kind: "all", folder: null, query: " REPORT " }, pathOf))).toEqual(["Wisman 2020-2024"])
  })
})

describe("sortItems", () => {
  const by = (column: SortColumn, dir: "asc" | "desc") => names(sortItems(ALL, { column, dir }, pathOf))

  it("sorts newest first by Updated, the built-in board (no time) last either way", () => {
    expect(by("updated", "desc")).toEqual(["Wisman 2020-2024", "QA BI", "qa-auto embed", "Sales overview", "Main"])
    expect(by("updated", "asc")).toEqual(["Sales overview", "qa-auto embed", "QA BI", "Wisman 2020-2024", "Main"])
  })

  it("sorts every other column, empty values last and the name breaking ties", () => {
    expect(by("name", "asc")).toEqual(["Main", "QA BI", "qa-auto embed", "Sales overview", "Wisman 2020-2024"])
    expect(by("name", "desc")).toEqual(["Wisman 2020-2024", "Sales overview", "qa-auto embed", "QA BI", "Main"])
    expect(by("kind", "asc")).toEqual(["Main", "QA BI", "qa-auto embed", "Wisman 2020-2024", "Sales overview"])
    expect(by("folder", "asc")).toEqual(["QA BI", "Wisman 2020-2024", "Main", "qa-auto embed", "Sales overview"])
    expect(by("owner", "asc")).toEqual(["qa-auto embed", "Wisman 2020-2024", "QA BI", "Sales overview", "Main"])
    expect(by("sharing", "asc")).toEqual(["Main", "qa-auto embed", "QA BI", "Sales overview", "Wisman 2020-2024"])
  })

  it("combines with the folder filter", () => {
    const inSales = filterItems(ALL, { kind: "all", folder: "f_sales", query: "" }, pathOf)
    expect(names(sortItems(inSales, { column: "name", dir: "desc" }, pathOf))).toEqual(["Wisman 2020-2024", "QA BI"])
  })
})

describe("the words of a row", () => {
  it("counts in the item's own unit", () => {
    expect(countLabel(main)).toBe("13 tiles")
    expect(countLabel(auto)).toBe("1 tile")
    expect(countLabel(report)).toBe("2 charts")
    expect(countLabel({ kind: "dashboard", count: 0 })).toBe("No tiles yet")
  })

  it("never says No description.: the description, a template's source, or nothing", () => {
    expect(factOf(boardItem({ id: "b", name: "n", description: "  Weekly arrivals  " }))).toBe("Weekly arrivals")
    expect(factOf(template)).toBe("Saved from QA BI")
    expect(factOf(qa)).toBeNull()
    expect(factOf(report)).toBeNull()
  })
})
