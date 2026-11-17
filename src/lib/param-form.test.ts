import assert from "node:assert/strict"
import test from "node:test"

import { draftFromDef, emptyParamDraft, entriesFromLines, inputFromDraft, paramProblems } from "./param-form"

test("the lines of a list become entries, numbers when every value is one", () => {
  assert.deepEqual(entriesFromLines("visitors | Visitors\nrooms\n\n"), [
    { value: "visitors", label: "Visitors" }, { value: "rooms", label: "rooms" },
  ])
  assert.deepEqual(entriesFromLines("5\n10 | ten"), [{ value: 5, label: "5" }, { value: 10, label: "ten" }])
  assert.deepEqual(entriesFromLines("5\nx"), [{ value: "5", label: "5" }, { value: "x", label: "x" }])
})

test("each type says what is still needed", () => {
  const d = { ...emptyParamDraft(), name: "target" }
  assert.deepEqual(paramProblems(d, false), ["a default number"])
  assert.deepEqual(paramProblems({ ...d, default: "5", min: "x" }, false), ["a minimum that is a number"])
  assert.deepEqual(paramProblems({ ...d, name: "1x", default: "5" }, false), ["a name of letters, digits and _"])
  assert.deepEqual(paramProblems({ ...d, name: "", default: "5" }, true), [], "a name is not retyped when editing")
  assert.deepEqual(paramProblems({ ...d, type: "date", default: "2026-1-1" }, false), ["a default date (YYYY-MM-DD)"])
  const list = { ...d, type: "list" as const, values: "a\nb", default: "c" }
  assert.deepEqual(paramProblems(list, false), ["a default that is one of the choices"])
  assert.deepEqual(paramProblems({ ...list, default: "b" }, false), [])
  assert.deepEqual(paramProblems({ ...list, values: "" }, false), ["at least one choice"])
})

test("the draft becomes the definition the server takes, leaving empty bounds out", () => {
  assert.deepEqual(inputFromDraft({ ...emptyParamDraft(), name: " target ", default: "1000", max: "5000" }), {
    name: "target", type: "number", default: 1000, max: 5000,
  })
  assert.deepEqual(inputFromDraft({ ...emptyParamDraft(), name: "pick", type: "list", values: "visitors\nrooms | Rooms", default: "rooms", label: "Measure" }), {
    name: "pick", type: "list", default: "rooms", label: "Measure",
    values: [{ value: "visitors", label: "visitors" }, { value: "rooms", label: "Rooms" }],
  })
  assert.deepEqual(inputFromDraft({ ...emptyParamDraft(), name: "who", type: "text", default: "x", maxLength: "20" }), {
    name: "who", type: "text", default: "x", maxLength: 20,
  })
})

test("a saved definition fills the form back in", () => {
  const draft = draftFromDef({
    id: "p_1", name: "pick", label: "pick", type: "list", default: "rooms",
    values: [{ value: "visitors", label: "visitors" }, { value: "rooms", label: "Rooms" }],
  })
  assert.equal(draft.label, "")
  assert.equal(draft.values, "visitors\nrooms | Rooms")
  assert.equal(draft.default, "rooms")
})
