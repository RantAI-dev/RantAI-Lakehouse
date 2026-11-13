// BLD-1 part 1 S3-T2: the editor's state and rules, without React. The
// limits and the name rule mirror `pipeline_compile.rs`; the error paths
// are its `DefinitionError::path` (D7).
import { describe, expect, it } from "bun:test"
import type { AuthoredDefinition } from "@/services/contracts/pipelines"
import {
  MAX_INPUTS,
  MAX_SQL_CHARS,
  MAX_STEPS,
  READS_AS_YOU_NOTICE,
  addInput,
  addStep,
  canAddInput,
  canAddStep,
  columnsRead,
  definitionBody,
  definitionChanged,
  editorProblems,
  editorStateFrom,
  effectiveInputs,
  errorItemId,
  errorTarget,
  hasProblems,
  isUsableName,
  moveStep,
  nameProblem,
  previewAsRecords,
  previewBody,
  previewKey,
  readableNames,
  readsAsYouAfterSave,
  removeItem,
  renameItem,
  setInputTable,
  sqlStepReads,
  stepCompletion,
  suggestInputName,
  updateStep,
  type EditorState,
  type OutputSettings,
} from "./pipeline-editor"

/** The pipeline of plan D1: two inputs, a form step and a SQL step. */
const STORED: Pick<AuthoredDefinition, "inputs" | "steps"> = {
  inputs: [
    { name: "orders", zone: "bronze", table: "orders" },
    { name: "customers", zone: "bronze", table: "customers" },
  ],
  steps: [
    { name: "paid", kind: "transform", reads: "orders", transforms: ["filter(status = 'paid')"] },
    {
      name: "by_country",
      kind: "sql",
      sql: "SELECT c.country, sum(p.amount) AS revenue FROM paid AS p LEFT JOIN customers AS c ON p.customer_id = c.id GROUP BY c.country",
    },
  ],
}

const OUTPUT: OutputSettings = { targetZone: "silver", targetTable: "sales_by_country", plainCopy: false, hasConnector: false }

function stored(): EditorState {
  return editorStateFrom(STORED)
}

describe("editorStateFrom", () => {
  it("gives a new pipeline one Bronze input with nothing chosen and no steps", () => {
    expect(editorStateFrom(null)).toEqual({
      inputs: [{ id: "input-1", name: "", zone: "bronze", table: "" }],
      steps: [],
      seq: 1,
    })
  })

  it("reads a stored definition as its inputs and steps, each with its own id", () => {
    const state = stored()
    expect(state.inputs.map((input) => [input.id, input.name])).toEqual([
      ["input-1", "orders"],
      ["input-2", "customers"],
    ])
    expect(state.steps.map((step) => [step.id, step.kind, step.name])).toEqual([
      ["step-3", "transform", "paid"],
      ["step-4", "sql", "by_country"],
    ])
  })

  it("opens a pipeline made before this change as one input and one form step (BLD-1-AC9)", () => {
    // What the API returns for such a pipeline after migration 0140 (D3).
    const state = editorStateFrom({
      inputs: [{ name: "orders_raw", zone: "bronze", table: "orders_raw" }],
      steps: [{ name: "transformed", kind: "transform", reads: "orders_raw", transforms: ["dedupe(id)"] }],
    })
    expect(state.inputs).toHaveLength(1)
    expect(state.steps).toEqual([
      { id: "step-2", kind: "transform", name: "transformed", reads: "orders_raw", transforms: ["dedupe(id)"] },
    ])
  })
})

describe("names (D2, D6a)", () => {
  it("accepts what Ident accepts, up to 48 characters, and not a leading __", () => {
    expect(isUsableName("orders_2026")).toBe(true)
    expect(isUsableName("_private")).toBe(true)
    expect(isUsableName("__hidden")).toBe(false)
    expect(isUsableName("2026_orders")).toBe(false)
    expect(isUsableName("order-lines")).toBe(false)
    expect(isUsableName("a".repeat(48))).toBe(true)
    expect(isUsableName("a".repeat(49))).toBe(false)
  })

  it("says why a name is refused, and refuses a name another item has in any case", () => {
    expect(nameProblem("", [])).toBe("Give it a name.")
    expect(nameProblem("__x", [])).toBe("A name cannot start with two underscores.")
    expect(nameProblem("a b", [])).toMatch(/letters, digits and underscores/)
    expect(nameProblem("a".repeat(49), [])).toMatch(/at most 48/)
    expect(nameProblem("Orders", ["orders"])).toBe("Another input or step already has this name.")
    expect(nameProblem("orders", ["customers"])).toBeNull()
  })

  it("names an input after its table, or input_1 when the table's name is not usable (D3)", () => {
    expect(suggestInputName("orders", [])).toBe("orders")
    expect(suggestInputName("orders", ["ORDERS"])).toBe("orders_2")
    expect(suggestInputName("2026-orders", [])).toBe("input_1")
    expect(suggestInputName("2026-orders", ["input_1"])).toBe("input_2")
  })

  it("lets a step read every input and only the steps before it, by their exact names", () => {
    const state = stored()
    expect(readableNames(state, 0)).toEqual(["orders", "customers"])
    expect(readableNames(state, 1)).toEqual(["orders", "customers", "paid"])
  })
})

describe("adding, removing, renaming and reordering", () => {
  it("adds inputs up to five, and does not offer a sixth", () => {
    let state = editorStateFrom(null)
    for (let n = 1; n < MAX_INPUTS; n += 1) state = addInput(state).state
    expect(state.inputs).toHaveLength(MAX_INPUTS)
    expect(canAddInput(state, false)).toBe(false)
    const sixth = addInput(state)
    expect(sixth.id).toBeNull()
    expect(sixth.state).toBe(state)
  })

  it("offers one input only when a connector feeds the pipeline (D12)", () => {
    const state = editorStateFrom(null)
    expect(canAddInput(state, true)).toBe(false)
    expect(addInput(state, true).id).toBeNull()
  })

  it("adds steps up to twenty, and does not offer a twenty-first", () => {
    let state = stored()
    while (canAddStep(state)) state = addStep(state, "sql").state
    expect(state.steps).toHaveLength(MAX_STEPS)
    expect(addStep(state, "transform").id).toBeNull()
  })

  it("starts a new step from what was last before it", () => {
    const sql = addStep(stored(), "sql")
    expect(sql.state.steps[2]).toEqual({ id: String(sql.id), kind: "sql", name: "step_3", sql: "SELECT *\nFROM by_country" })
    const form = addStep(editorStateFrom({ inputs: STORED.inputs, steps: [] }), "transform")
    expect(form.state.steps[0]).toEqual({ id: String(form.id), kind: "transform", name: "step_1", reads: "customers", transforms: [] })
  })

  it("never gives a new item the id of one that was removed", () => {
    const state = stored()
    const removed = removeItem(state, "step-4")
    const added = addStep(removed, "sql")
    expect(added.id).not.toBe("step-4")
    expect(added.id).toBe("step-5")
  })

  it("renames an item, and a form step that read the old name reads the new one", () => {
    const state = renameItem(stored(), "input-1", "all_orders")
    expect(state.inputs[0].name).toBe("all_orders")
    expect(state.steps[0]).toMatchObject({ reads: "all_orders" })
    // The SQL step's text is the user's: it is not rewritten.
    expect(state.steps[1]).toMatchObject({ sql: STORED.steps[1].kind === "sql" ? STORED.steps[1].sql : "" })
  })

  it("moves a step up and down, and does nothing past either end", () => {
    const state = stored()
    expect(moveStep(state, "step-4", -1).steps.map((step) => step.name)).toEqual(["by_country", "paid"])
    expect(moveStep(state, "step-3", 1).steps.map((step) => step.name)).toEqual(["by_country", "paid"])
    expect(moveStep(state, "step-3", -1)).toBe(state)
    expect(moveStep(state, "step-4", 1)).toBe(state)
  })

  it("names an input after its table while the name is still the suggested one, and keeps a typed name", () => {
    let state = editorStateFrom(null)
    state = setInputTable(state, "input-1", "o")
    state = setInputTable(state, "input-1", "orders")
    expect(state.inputs[0]).toMatchObject({ name: "orders", table: "orders" })
    state = renameItem(state, "input-1", "src")
    state = setInputTable(state, "input-1", "orders_v2")
    expect(state.inputs[0]).toMatchObject({ name: "src", table: "orders_v2" })
  })
})

describe("the connector's input (BLD-7 decision 5)", () => {
  it("fixes the first input to Bronze and to one of the connector's tables", () => {
    const inputs = [{ id: "input-1", name: "orders", zone: "silver", table: "orders" }]
    expect(effectiveInputs(inputs, ["orders", "customers"])[0]).toMatchObject({ zone: "bronze", table: "orders" })
    expect(effectiveInputs(inputs, ["customers"])[0]).toMatchObject({ zone: "bronze", table: "" })
    expect(effectiveInputs(inputs, null)[0]).toMatchObject({ zone: "silver", table: "orders" })
  })
})

describe("editorProblems", () => {
  it("finds nothing wrong with the plan's own pipeline", () => {
    const problems = editorProblems(stored(), OUTPUT)
    expect(problems).toEqual({ items: {}, output: null })
    expect(hasProblems(problems)).toBe(false)
  })

  it("puts each problem on the item it is about", () => {
    let state = stored()
    state = renameItem(state, "input-2", "Orders")
    state = updateStep(state, "step-3", { reads: "by_country" })
    state = updateStep(state, "step-4", { sql: "   " })
    const problems = editorProblems(state, OUTPUT)
    expect(problems.items["input-2"]).toBe("Another input or step already has this name.")
    // A step after it is not readable: reads names an input or an EARLIER step.
    expect(problems.items["step-3"]).toBe("Choose an input or an earlier step for this step to read.")
    expect(problems.items["step-4"]).toBe("Write the step's SELECT.")
  })

  it("refuses a SQL step over 20,000 characters (decision 16)", () => {
    const state = updateStep(stored(), "step-4", { sql: `SELECT 1 ${"-".repeat(MAX_SQL_CHARS)}` })
    expect(editorProblems(state, OUTPUT).items["step-4"]).toBe("A SQL step is at most 20,000 characters.")
  })

  it("refuses a form step with no operation, so a copy is never made by accident (BLD-10 D2)", () => {
    const state = updateStep(stored(), "step-3", { transforms: [] })
    expect(editorProblems(state, OUTPUT).items["step-3"]).toBe("Add an operation, or remove this step.")
  })

  it("asks for a step, or the plain copy chosen on purpose, when there is no step", () => {
    const one = editorStateFrom({ inputs: [STORED.inputs[0]], steps: [] })
    expect(editorProblems(one, OUTPUT).output).toBe("Add a step, or choose to copy the input as it is.")
    expect(editorProblems(one, { ...OUTPUT, plainCopy: true }).output).toBeNull()
    const two = editorStateFrom({ inputs: STORED.inputs, steps: [] })
    expect(editorProblems(two, { ...OUTPUT, plainCopy: true }).output).toBe("With more than one input, add a step that reads them.")
  })

  it("refuses an output that is one of the inputs, and a second input beside a connector", () => {
    expect(editorProblems(stored(), { ...OUTPUT, targetZone: "bronze", targetTable: "orders" }).output).toBe(
      "The output cannot be one of the inputs."
    )
    expect(editorProblems(stored(), { ...OUTPUT, hasConnector: true }).items["input-2"]).toMatch(/connector reads one input/)
  })
})

describe("what is sent", () => {
  it("sends inputs and steps in D1's shape, names trimmed and SQL as written", () => {
    let state = renameItem(stored(), "input-2", " customers ")
    state = updateStep(state, "step-4", { sql: "SELECT 1 -- note\n" })
    expect(definitionBody(state.inputs, state.steps)).toEqual({
      inputs: [
        { name: "orders", zone: "bronze", table: "orders" },
        { name: "customers", zone: "bronze", table: "customers" },
      ],
      steps: [
        { name: "paid", kind: "transform", reads: "orders", transforms: ["filter(status = 'paid')"] },
        { name: "by_country", kind: "sql", sql: "SELECT 1 -- note\n" },
      ],
    })
  })

  it("tells a change to the inputs or steps from no change", () => {
    const state = stored()
    expect(definitionChanged(state.inputs, state.steps, STORED)).toBe(false)
    const edited = updateStep(state, "step-4", { sql: "SELECT * FROM paid" })
    expect(definitionChanged(edited.inputs, edited.steps, STORED)).toBe(true)
    const moved = moveStep(state, "step-4", -1)
    expect(definitionChanged(moved.inputs, moved.steps, STORED)).toBe(true)
  })

  it("says the pipeline will read as you only when the definition changed and it reads as someone else (D17a)", () => {
    expect(readsAsYouAfterSave("u-author", "u-me", true)).toBe(true)
    expect(readsAsYouAfterSave("u-author", "u-me", false)).toBe(false)
    expect(readsAsYouAfterSave("u-me", "u-me", true)).toBe(false)
    // No author yet: saving as a user makes it read as them (D10).
    expect(readsAsYouAfterSave(undefined, "u-me", true)).toBe(true)
    expect(READS_AS_YOU_NOTICE).toBe("This pipeline will read as you from now on.")
  })
})

describe("the preview (D16)", () => {
  it("previews an input alone, and a step with every input and the steps up to it", () => {
    const state = stored()
    expect(previewBody(state.inputs, state.steps, { kind: "input", id: "input-2" })).toEqual({
      inputs: [{ name: "customers", zone: "bronze", table: "customers" }],
      steps: [],
      upTo: "customers",
    })
    const paid = previewBody(state.inputs, state.steps, { kind: "step", id: "step-3" })
    expect(paid?.upTo).toBe("paid")
    expect(paid?.inputs).toHaveLength(2)
    expect(paid?.steps.map((step) => step.name)).toEqual(["paid"])
  })

  it("previews the output as the last step, or the one input when there is no step", () => {
    const state = stored()
    expect(previewBody(state.inputs, state.steps, { kind: "output" })?.upTo).toBe("by_country")
    const copy = editorStateFrom({ inputs: [STORED.inputs[0]], steps: [] })
    expect(previewBody(copy.inputs, copy.steps, { kind: "output" })?.upTo).toBe("orders")
  })

  it("has nothing to preview for an item with no name", () => {
    const state = editorStateFrom(null)
    expect(previewBody(state.inputs, state.steps, { kind: "input", id: "input-1" })).toBeNull()
  })

  it("keys a preview by what it was made from, so an edit makes it stale", () => {
    const state = stored()
    const before = previewBody(state.inputs, state.steps, { kind: "step", id: "step-4" })
    const edited = updateStep(state, "step-4", { sql: "SELECT country FROM paid" })
    const after = previewBody(edited.inputs, edited.steps, { kind: "step", id: "step-4" })
    expect(before && after && previewKey(before) !== previewKey(after)).toBe(true)
    // A later step's edit does not touch an earlier step's preview.
    const earlier = previewBody(edited.inputs, edited.steps, { kind: "step", id: "step-3" })
    const earlierBefore = previewBody(state.inputs, state.steps, { kind: "step", id: "step-3" })
    expect(earlier && earlierBefore && previewKey(earlier) === previewKey(earlierBefore)).toBe(true)
  })

  it("turns the API's rows of arrays into records keyed by column", () => {
    expect(
      previewAsRecords({
        columns: [
          { name: "id", type: "Int64" },
          { name: "name", type: "Nullable(String)" },
        ],
        rows: [[1, "a"], [2, null]],
        truncated: false,
      }).rows
    ).toEqual([
      { id: 1, name: "a" },
      { id: 2, name: null },
    ])
  })
})

describe("errors that name an item (D7)", () => {
  it("reads inputs[i], steps[i] and steps[i].transforms[j]", () => {
    expect(errorTarget("inputs[0]: you may not read this table")).toEqual({ kind: "input", index: 0 })
    expect(errorTarget("steps[1]: this step reads a table that is not an input; add it under Inputs")).toEqual({
      kind: "step",
      index: 1,
      transform: null,
    })
    expect(errorTarget("steps[0].transforms[2]: this transform is not in the supported grammar")).toEqual({
      kind: "step",
      index: 0,
      transform: 2,
    })
    expect(errorTarget("database error")).toBeNull()
  })

  it("finds the item a refusal names, and nothing for an index that is not there", () => {
    const state = stored()
    expect(errorItemId(state, errorTarget("steps[1]: refused"))).toBe("step-4")
    expect(errorItemId(state, errorTarget("inputs[1]: refused"))).toBe("input-2")
    expect(errorItemId(state, errorTarget("steps[9]: refused"))).toBeNull()
    expect(errorItemId(state, null)).toBeNull()
  })
})

describe("columns and completion", () => {
  const columns = { "input-1": [{ name: "id", type: "Int64" }, { name: "status", type: "String" }], "input-2": null }

  it("offers a form step the columns of the input it reads, or of the step it reads once previewed", () => {
    const state = stored()
    const paid = state.steps[0]
    if (paid.kind !== "transform") throw new Error("fixture")
    expect(columnsRead(paid, state.inputs, columns, {})).toEqual(["id", "status"])
    expect(columnsRead({ ...paid, reads: "customers" }, state.inputs, columns, {})).toBeNull()
    expect(columnsRead({ ...paid, reads: "x" }, state.inputs, columns, { x: [{ name: "n", type: "UInt8" }] })).toEqual(["n"])
  })

  it("completes a SQL step with the inputs and the earlier steps, and the inputs' columns", () => {
    const { tables, columns: cache } = stepCompletion(stored(), 1, columns)
    expect(tables.map((table) => table.reference)).toEqual(["orders", "customers", "paid"])
    expect(cache.orders).toEqual({
      status: "ready",
      columns: [
        { name: "id", dataType: "Int64" },
        { name: "status", dataType: "String" },
      ],
    })
    // No entry for columns not known: no "loading" note that never ends.
    expect(cache.customers).toBeUndefined()
    // The first step reads no step.
    expect(stepCompletion(stored(), 0, columns).tables.map((table) => table.reference)).toEqual(["orders", "customers"])
  })
})

describe("sqlStepReads (the pipeline page's edges)", () => {
  const names = ["orders", "customers", "paid"]

  it("reads the names after FROM and JOIN, as written, once each", () => {
    expect(
      sqlStepReads("SELECT * FROM paid AS p LEFT JOIN customers c ON p.customer_id = c.id JOIN paid q ON 1=1", names)
    ).toEqual(["paid", "customers"])
    expect(sqlStepReads("SELECT * FROM `orders`", names)).toEqual(["orders"])
  })

  it("does not take a name in a string, a comment, another case, or a qualified name", () => {
    expect(sqlStepReads("SELECT 'FROM orders' AS t -- FROM customers\nFROM paid", names)).toEqual(["paid"])
    expect(sqlStepReads("SELECT * FROM ORDERS", names)).toEqual([])
    expect(sqlStepReads("SELECT * FROM bronze.orders", names)).toEqual([])
  })
})
