import { describe, expect, it } from "bun:test"

import { KIND_ICON } from "@/features/dashboards/chart-lineage-panel"
import { LINEAGE_STEP_KINDS, STEP_KIND_LABEL } from "./lineage-words"

describe("lineage step kinds (BI-71, owner QA round 1)", () => {
  it("gives every kind the API sends a capitalised label and an icon", () => {
    for (const kind of LINEAGE_STEP_KINDS) {
      expect(STEP_KIND_LABEL[kind]).toBeTruthy()
      expect(STEP_KIND_LABEL[kind]?.[0]).toBe(STEP_KIND_LABEL[kind]?.[0]?.toUpperCase())
      expect(KIND_ICON[kind]).toBeDefined()
    }
  })

  it("names a source and a dataset", () => {
    expect(STEP_KIND_LABEL.source).toBe("Source")
    expect(STEP_KIND_LABEL.dataset).toBe("Dataset")
  })
})
