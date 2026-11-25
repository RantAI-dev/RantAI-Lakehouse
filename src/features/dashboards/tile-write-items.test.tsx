import { describe, expect, it, mock } from "bun:test"
import { chartWriteItems } from "./tile-write-items"

// `BI-48` (QA `LH-BI-6-40`): "Duplicate" sits after "Edit chart" for exactly
// the charts that can be edited — the dashboard's own, with a definition —
// and no one else sees it.

describe("chartWriteItems", () => {
  const base = { own: true, hasDef: true, onEdit: () => {}, onDuplicate: () => {}, onDelete: () => {} }

  it("a person who may edit gets Duplicate after Edit chart", () => {
    const onDuplicate = mock<() => void>(() => {})
    const items = chartWriteItems({ ...base, canWrite: true, onDuplicate })
    expect(items.edit.map((i) => i.label)).toEqual(["Edit chart", "Duplicate"])
    items.edit[1].onSelect?.()
    expect(onDuplicate).toHaveBeenCalledTimes(1)
  })

  it("a person who may not edit gets no Duplicate", () => {
    expect(chartWriteItems({ ...base, canWrite: false })).toEqual({ edit: [], remove: [] })
  })

  it("a built-in tile and a chart with no definition get no Duplicate", () => {
    expect(chartWriteItems({ ...base, canWrite: true, own: false })).toEqual({ edit: [], remove: [] })
    const noDef = chartWriteItems({ ...base, canWrite: true, hasDef: false })
    expect(noDef.edit).toEqual([])
    expect(noDef.remove.map((i) => i.label)).toEqual(["Delete chart"])
  })
})
