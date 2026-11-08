import { strict as assert } from "node:assert"
import { test } from "node:test"
import type { QueryParamDef } from "@/services/contracts/queries"
import {
  activeParameters,
  findParamNames,
  reconcileDefinitions,
  replaceDefinition,
  retypeDefinition,
} from "./query-param-names"

const names = (sql: string) => findParamNames(sql).map((found) => found.name)

test("findParamNames lists the names in the order they first appear, once each", () => {
  assert.deepEqual(
    names("SELECT * FROM t WHERE b = {{beta}} AND a = {{alpha}} AND c = {{beta}}"),
    ["beta", "alpha"]
  )
})

test("findParamNames reads spaces inside the braces the way the server does", () => {
  assert.deepEqual(names("SELECT {{ region }}, {{\tcity\n}}, {{ day.start }}"), [
    "region",
    "city",
    "day",
  ])
})

test("findParamNames makes no name of a placeholder in a comment", () => {
  const sql = [
    "-- {{in_line}}",
    "SELECT {{real}} /* {{in_block}} */ # {{in_hash}}",
    "FROM t",
  ].join("\n")
  assert.deepEqual(names(sql), ["real"])
})

test("findParamNames makes no name of a placeholder in a quoted string or identifier", () => {
  assert.deepEqual(names(`SELECT '{{in_string}}', "{{in_ident}}", \`{{in_tick}}\`, {{real}}`), [
    "real",
  ])
})

test("findParamNames ignores a {{ that is not a placeholder", () => {
  assert.deepEqual(names("SELECT {{9lives}}, {{a b}}, {{}}, {{a.middle}}, {{ok"), [])
  assert.deepEqual(names(`SELECT {{${"x".repeat(65)}}}`), [])
  assert.deepEqual(names(`SELECT {{${"x".repeat(64)}}}`), ["x".repeat(64)])
})

test("findParamNames takes a leading underscore and digits after the first letter", () => {
  assert.deepEqual(names("SELECT {{_a1}}, {{B2_c}}"), ["_a1", "B2_c"])
})

test("findParamNames reduces name.start and name.end to one name used as a range", () => {
  const found = findParamNames("WHERE d >= {{day.start}} AND d <= {{day.end}}")
  assert.deepEqual(found, [{ name: "day", plain: false, ranged: true }])
})

test("findParamNames marks a name used both ways", () => {
  const found = findParamNames("WHERE d = {{day}} AND d >= {{day.start}}")
  assert.deepEqual(found, [{ name: "day", plain: true, ranged: true }])
})

test("activeParameters gives a new name a text definition and a range name a dateRange one", () => {
  assert.deepEqual(activeParameters("SELECT {{region}} WHERE d > {{day.start}}", []), [
    { name: "region", type: "text" },
    { name: "day", type: "dateRange" },
  ])
})

test("activeParameters keeps the stored definition of a name and leaves out one the SQL does not use", () => {
  const stored: QueryParamDef[] = [
    { name: "gone", type: "number" },
    { name: "region", type: "dropdown", options: [{ value: "eu", label: "Europe" }] },
  ]
  assert.deepEqual(activeParameters("SELECT {{region}}, {{city}}", stored), [
    stored[1],
    { name: "city", type: "text" },
  ])
})

test("activeParameters reads a name used both plain and as a range as text when nothing is stored", () => {
  assert.deepEqual(activeParameters("{{d}} {{d.start}}", []), [{ name: "d", type: "text" }])
})

test("reconcileDefinitions appends a definition for a name the SQL gained", () => {
  const stored: QueryParamDef[] = [{ name: "region", type: "number" }]
  assert.deepEqual(reconcileDefinitions("{{region}} {{day.end}}", stored, {}), [
    { name: "region", type: "number" },
    { name: "day", type: "dateRange" },
  ])
})

test("reconcileDefinitions keeps an edited definition and its value when the name leaves the SQL", () => {
  const dropdown: QueryParamDef = {
    name: "region",
    type: "dropdown",
    options: [{ value: "eu", label: "Europe" }],
  }
  assert.deepEqual(reconcileDefinitions("SELECT 1", [dropdown], {}), [dropdown])
})

test("reconcileDefinitions keeps an untouched definition that holds a value when the name leaves the SQL", () => {
  const plain: QueryParamDef = { name: "region", type: "text" }
  assert.deepEqual(reconcileDefinitions("SELECT 1", [plain], { region: "EU" }), [plain])
})

test("reconcileDefinitions drops a definition nobody edited and nobody filled when its name leaves the SQL", () => {
  // The editor closes `{{` itself, so typing a name leaves `r`, `re`, `reg`
  // behind as names for a moment. Nothing in them is worth keeping.
  assert.deepEqual(
    reconcileDefinitions("SELECT {{region}}", [{ name: "re", type: "text" }], {}),
    [{ name: "region", type: "text" }]
  )
})

test("reconcileDefinitions returns the same list when nothing changes", () => {
  const stored: QueryParamDef[] = [{ name: "region", type: "text" }]
  assert.equal(reconcileDefinitions("SELECT {{region}}", stored, {}), stored)
})

test("replaceDefinition puts the new definition where the old one was", () => {
  const stored: QueryParamDef[] = [
    { name: "a", type: "text" },
    { name: "b", type: "text" },
  ]
  const next = replaceDefinition(stored, {}, { name: "a", type: "number" })
  assert.deepEqual(next.parameters, [
    { name: "a", type: "number" },
    { name: "b", type: "text" },
  ])
})

test("replaceDefinition appends a definition the tab did not have", () => {
  const next = replaceDefinition([{ name: "a", type: "text" }], {}, { name: "b", type: "date" })
  assert.deepEqual(next.parameters, [
    { name: "a", type: "text" },
    { name: "b", type: "date" },
  ])
})

test("replaceDefinition clears the value when it no longer fits the new type", () => {
  const next = replaceDefinition(
    [{ name: "a", type: "text" }],
    { a: "EU", other: 3 },
    { name: "a", type: "number" }
  )
  assert.deepEqual(next.values, { other: 3 })
})

test("replaceDefinition keeps the value when it still fits the new type", () => {
  const next = replaceDefinition(
    [{ name: "a", type: "text" }],
    { a: "EU" },
    { name: "a", type: "dropdown", options: [{ value: "EU", label: "Europe" }] }
  )
  assert.deepEqual(next.values, { a: "EU" })
})

test("retypeDefinition drops a default that does not fit and the options of a type with none", () => {
  const dropdown: QueryParamDef = {
    name: "a",
    type: "dropdown",
    default: "eu",
    options: [{ value: "eu", label: "Europe" }],
  }
  assert.deepEqual(retypeDefinition(dropdown, "number"), { name: "a", type: "number" })
  assert.deepEqual(retypeDefinition(dropdown, "text"), { name: "a", type: "text", default: "eu" })
  assert.deepEqual(retypeDefinition(dropdown, "multiSelect"), {
    name: "a",
    type: "multiSelect",
    options: dropdown.options,
  })
})
