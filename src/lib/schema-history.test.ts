import { describe, expect, it } from "bun:test"
import type { LakehouseSchemaVersion } from "@/services/contracts/lakehouse"
import { schemaHistory } from "./schema-history"

const field = (id: number, name: string, type = "string", required = false) => ({ id, name, type, required })

const V0: LakehouseSchemaVersion = {
  schemaId: 0,
  sinceMs: 1_000,
  current: false,
  fields: [field(1, "id", "long", true), field(2, "name"), field(3, "legacy_code")],
}

describe("schemaHistory", () => {
  it("lists changes newest first, and says what the first version started with", () => {
    const v1: LakehouseSchemaVersion = {
      schemaId: 1,
      sinceMs: 2_000,
      current: true,
      fields: [field(1, "id", "long", true), field(2, "name"), field(4, "email")],
    }
    expect(schemaHistory([v1, V0])).toEqual([
      { schemaId: 1, sinceMs: 2_000, current: true, changes: ["Added email (string)", "Dropped legacy_code (string)"] },
      { schemaId: 0, sinceMs: 1_000, current: false, changes: ["Created with 3 columns"] },
    ])
  })

  it("reads a rename, a type change and a nullability change off the field id", () => {
    const v1: LakehouseSchemaVersion = {
      schemaId: 1,
      sinceMs: null,
      current: true,
      fields: [field(1, "id", "long", false), field(2, "full_name"), field(3, "legacy_code", "int")],
    }
    expect(schemaHistory([V0, v1])[0].changes).toEqual([
      "Made id nullable",
      "Renamed name to full_name",
      "Changed legacy_code from string to int",
    ])
  })

  it("names a reorder, and admits when nothing about the columns changed", () => {
    const reordered = { ...V0, schemaId: 1, fields: [V0.fields[1], V0.fields[0], V0.fields[2]] }
    expect(schemaHistory([V0, reordered])[0].changes).toEqual(["Reordered columns"])
    expect(schemaHistory([V0, { ...V0, schemaId: 1 }])[0].changes).toEqual(["No column changes"])
  })
})
