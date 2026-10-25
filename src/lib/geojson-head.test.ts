import { describe, expect, it } from "bun:test"
import { fileProblem, firstFeatureProperties, guessNameProperty, nameFromFile } from "./geojson-head"

const FC = (props: string) => `{"type":"FeatureCollection","features":[{"type":"Feature","properties":${props},"geometry":{"type":"Polygon","coordinates":[[[0,0],[1,0],[1,1],[0,0]]]}}]}`

describe("firstFeatureProperties (BI-16 C2b)", () => {
  it("reads the properties of the first feature, with braces and quotes inside strings", () => {
    const head = firstFeatureProperties(FC('{"REGION":"North {A}","code":7,"note":"say \\"hi\\" }"}'))
    expect(head).toEqual({ ok: true, properties: { REGION: "North {A}", code: 7, note: 'say "hi" }' }, keys: ["REGION", "code", "note"] })
  })

  it("reads a file whose head is cut off after the first feature", () => {
    expect(firstFeatureProperties(FC('{"name":"A"}') + '{"type":"Feature","prop')).toMatchObject({ ok: true, keys: ["name"] })
  })

  it("says what is wrong with a file that is not a FeatureCollection or has no properties", () => {
    expect(firstFeatureProperties('{"type":"Point","coordinates":[0,0]}')).toEqual({ ok: false, reason: "This does not look like a GeoJSON FeatureCollection." })
    expect(firstFeatureProperties('{"type":"FeatureCollection"}')).toEqual({ ok: false, reason: "The file has no features list." })
    expect(firstFeatureProperties('{"type":"FeatureCollection","features":[{"type":"Feature","geometry":null}]}')).toEqual({ ok: false, reason: "The first feature has no properties to take a name from." })
    expect(firstFeatureProperties('{"type":"FeatureCollection","features":[{"properties":{"name":"cut')).toEqual({ ok: false, reason: "The first feature's properties could not be read." })
  })
})

describe("the choices of the upload form", () => {
  it("guesses the name property from a known word, else the first text property", () => {
    expect(guessNameProperty({ id: 1, NAME: "A" })).toBe("NAME")
    expect(guessNameProperty({ id: 1, label: "A" })).toBe("label")
    expect(guessNameProperty({})).toBe("")
  })

  it("suggests the map's name from the file's", () => {
    expect(nameFromFile("sales_regions-2024.geojson")).toBe("sales regions 2024")
  })

  it("refuses a file over 50 MB, empty, or of another type, before reading it", () => {
    expect(fileProblem({ name: "a.geojson", size: 50 * 1024 * 1024 })).toBeNull()
    expect(fileProblem({ name: "a.geojson", size: 50 * 1024 * 1024 + 1 })).toBe("This file is over 50 MB. Simplify it or split it, then upload again.")
    expect(fileProblem({ name: "a.geojson", size: 0 })).toBe("The file is empty.")
    expect(fileProblem({ name: "a.csv", size: 10 })).toBe("Choose a GeoJSON file (.geojson or .json).")
  })
})
