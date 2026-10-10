import assert from "node:assert/strict"
import test from "node:test"

import { previewKey, type PreviewInputs } from "./preview-key"

const base: PreviewInputs = {
  title: "t", source: "mart:m", kind: "line", dimension: "d", measure: "v", measure2: "", measure3: "",
  breakdown: "", mapId: "", lat: "", lon: "", aggregate: "sum", span: 1, caption: "", target: "",
  text: "", order: "none", limit: 20, targetBoard: "default", grain: "", tables: "",
}

test("the preview is requested again when Group by changes", () => {
  assert.notEqual(previewKey({ ...base, grain: "month" }), previewKey(base))
  assert.notEqual(previewKey({ ...base, grain: "month" }), previewKey({ ...base, grain: "year" }))
})

test("every input changes the key, and an unchanged input does not", () => {
  assert.equal(previewKey({ ...base }), previewKey(base))
  for (const [name, value] of Object.entries(base)) {
    const changed = { ...base, [name]: typeof value === "number" ? value + 1 : `${value}x` }
    assert.notEqual(previewKey(changed), previewKey(base), name)
  }
})
