/**
 * The pipeline editor's state and its rules (BLD-1 part 1, D2, D7, D16,
 * D17, D17a), kept out of the component so each rule is tested on its own
 * (`CODE-STANDARD.md` 4.6).
 *
 * A pipeline is a list of inputs, an ordered list of steps and one output
 * table (D1). The editor holds the same shape with one addition: every input
 * and step carries a client-side `id` that never changes and is never
 * reused, so the selection, a preview and an error stay attached to the item
 * while its name is being typed, renamed or moved.
 *
 * The API is the authority on every rule here (`pipeline_compile.rs`): it
 * checks the names, the limits and what a step reads on save and on preview,
 * and its refusal names the item (`steps[1]: ...`). The editor checks the
 * same things first so the page can say what is wrong before a round trip,
 * and so it never offers what the API would refuse (a sixth input, a
 * twenty-first step). The limits are the API's own constants; a change there
 * is a change here.
 */

import type { ColumnState, CompletionTable } from "./sql-completion"
import { tokenizeSql } from "./sql-text"
import type { DataLayer } from "./status"
import type {
  AuthoredDefinition,
  PipelineInput,
  PipelinePreview,
  PipelinePreviewInput,
  PipelineSqlStep,
  PipelineStep,
  PipelineTransformStep,
} from "@/services/contracts/pipelines"

/** Decision 4; `pipeline_compile::MAX_INPUTS`. */
export const MAX_INPUTS = 5
/** Decision 15; `pipeline_compile::MAX_STEPS`. */
export const MAX_STEPS = 20
/** D2; `pipeline_compile::MAX_NAME_CHARS`. */
export const MAX_NAME_CHARS = 48
/** Decision 16; `sql_guard::SQL_SOURCE_MAX_CHARS`, the limit a dashboard's SQL source has. */
export const MAX_SQL_CHARS = 20_000

export type EditorInput = { id: string; name: string; zone: string; table: string }

/**
 * What a form step reads: the item, by its id. BLD-1 part 1 S3 review
 * R1-B2: following the item rather than the text of its name means a
 * rename (even through an empty name), a reorder or a removal needs no text
 * matching; the name is written only when the body is built.
 * `{ name }` holds a stored `reads` that names no input or step, written
 * back as it was so a save does not invent a different one.
 */
export type StepRead = { id: string } | { name: string }

/**
 * A step being edited. `unknown` is a stored step of a kind this console
 * does not know (R1-S2): it keeps the stored object as it was, is shown and
 * can be moved or removed, and is sent back unchanged.
 */
export type EditorStep =
  | { id: string; kind: "transform"; name: string; reads: StepRead; transforms: string[] }
  | { id: string; kind: "sql"; name: string; sql: string }
  | { id: string; kind: "unknown"; name: string; stored: PipelineStep }

/**
 * A stored step as one of the two kinds this console knows, or `null`.
 * The fields are checked, not only `kind`: the API may add kinds (part 2),
 * and a step whose fields are not the ones its kind has is not edited as
 * if they were (R1-S2).
 */
export function knownStep(step: PipelineStep): PipelineTransformStep | PipelineSqlStep | null {
  const fields = step as Record<string, unknown>
  if (step.kind === "sql" && typeof fields.sql === "string") return step as PipelineSqlStep
  if (
    step.kind === "transform" &&
    typeof fields.reads === "string" &&
    Array.isArray(fields.transforms) &&
    (fields.transforms as unknown[]).every((t) => typeof t === "string")
  )
    return step as PipelineTransformStep
  return null
}

/**
 * The definition being edited. `seq` hands out ids: it only grows, so an
 * item added after another was removed never takes the removed one's id
 * (and with it its preview or its error).
 */
export type EditorState = { inputs: EditorInput[]; steps: EditorStep[]; seq: number }

/** What the rail has selected. */
export type EditorSelection =
  | { kind: "input"; id: string }
  | { kind: "step"; id: string }
  | { kind: "output" }

function nextId(state: EditorState, prefix: "input" | "step"): [string, EditorState] {
  const seq = state.seq + 1
  return [`${prefix}-${seq}`, { ...state, seq }]
}

/**
 * The editor's state for a stored definition, or for a new pipeline: one
 * Bronze input with nothing chosen yet and no steps.
 *
 * A pipeline made before BLD-1 part 1 needs no case of its own: the API
 * returns it as one input and, when it had transforms, one `transform` step
 * (D3, migration `0140`), so it opens as exactly that (`BLD-1-AC9`).
 */
export function editorStateFrom(def: Pick<AuthoredDefinition, "inputs" | "steps"> | null): EditorState {
  if (!def) return { inputs: [{ id: "input-1", name: "", zone: "bronze", table: "" }], steps: [], seq: 1 }
  let seq = 0
  const inputs = def.inputs.map((input) => ({ id: `input-${++seq}`, name: input.name, zone: input.zone, table: input.table }))
  const stepIds = def.steps.map(() => `step-${++seq}`)
  // A stored `reads` names an item exactly (D6a); it is resolved to that
  // item's id once, here.
  const byName = new Map<string, string>()
  def.inputs.forEach((input, i) => byName.set(input.name, inputs[i].id))
  def.steps.forEach((step, i) => {
    if (!byName.has(step.name)) byName.set(step.name, stepIds[i])
  })
  const steps: EditorStep[] = def.steps.map((stored, i) => {
    const id = stepIds[i]
    const step = knownStep(stored)
    if (step === null) return { id, kind: "unknown", name: stored.name, stored }
    if (step.kind === "sql") return { id, kind: "sql", name: step.name, sql: step.sql }
    const readId = byName.get(step.reads)
    return { id, kind: "transform", name: step.name, reads: readId ? { id: readId } : { name: step.reads }, transforms: [...step.transforms] }
  })
  return { inputs, steps, seq }
}

// ── names (D2, D6a) ─────────────────────────────────────────────────────

/** `Ident::new`: letters, digits and underscores, not starting with a digit. */
const IDENT = /^[A-Za-z_][A-Za-z0-9_]*$/

/** Whether `name` is one D2 allows for an input or a step (`pipeline_compile::valid_name`). */
export function isUsableName(name: string): boolean {
  return name.length <= MAX_NAME_CHARS && !name.startsWith("__") && IDENT.test(name)
}

/**
 * What is wrong with an input's or a step's name, or `null`. `others` are
 * the names of every other input and step: D2 makes names unique without
 * regard to case, although a step reads a name exactly (D6a).
 */
export function nameProblem(name: string, others: readonly string[]): string | null {
  if (name.trim() === "") return "Give it a name."
  if (name.length > MAX_NAME_CHARS) return `A name is at most ${MAX_NAME_CHARS} characters.`
  if (name.startsWith("__")) return "A name cannot start with two underscores."
  if (!IDENT.test(name)) return "A name is letters, digits and underscores, and does not start with a digit."
  const key = name.toLowerCase()
  if (others.some((other) => other.toLowerCase() === key)) return "Another input or step already has this name."
  return null
}

/** Every input's and step's name except the item `id`'s. */
function otherNames(state: EditorState, id: string): string[] {
  return [...state.inputs, ...state.steps].filter((item) => item.id !== id).map((item) => item.name)
}

function taken(names: readonly string[], candidate: string): boolean {
  const key = candidate.toLowerCase()
  return names.some((name) => name.toLowerCase() === key)
}

/** `base`, or `base_2`, `base_3`, … : the first that no other item has. */
function firstFree(base: string, names: readonly string[]): string {
  if (!taken(names, base)) return base
  for (let n = 2; ; n += 1) {
    const candidate = `${base}_${n}`
    if (!taken(names, candidate)) return candidate
  }
}

/**
 * The name an input gets from its table: the table's own name when it is a
 * usable one, otherwise `input` (D3 names such an input `input_1`), made
 * unique against the other names.
 */
export function suggestInputName(table: string, others: readonly string[]): string {
  const trimmed = table.trim()
  if (isUsableName(trimmed)) return firstFree(trimmed, others)
  // D3's own fallback is `input_1`; the next free number after that.
  for (let n = 1; ; n += 1) {
    if (!taken(others, `input_${n}`)) return `input_${n}`
  }
}

/**
 * The items a step may read: every input, then every step before it, in
 * order (D6a). Items with a blank name are left out: nothing can read them.
 */
export function readableItems(
  state: Pick<EditorState, "inputs" | "steps">,
  stepIndex: number
): { id: string; name: string }[] {
  return [...state.inputs, ...state.steps.slice(0, Math.max(0, stepIndex))]
    .filter((item) => item.name.trim() !== "")
    .map((item) => ({ id: item.id, name: item.name }))
}

/** The names a step may read, exactly as written, case included (D6a). */
export function readableNames(state: Pick<EditorState, "inputs" | "steps">, stepIndex: number): string[] {
  return readableItems(state, stepIndex).map((item) => item.name)
}

/**
 * The name a form step's `reads` is written as: the read item's current
 * name, or the stored text when it named no item; `""` when the item it read
 * was removed.
 */
export function readsName(state: Pick<EditorState, "inputs" | "steps">, reads: StepRead): string {
  if ("name" in reads) return reads.name
  return [...state.inputs, ...state.steps].find((item) => item.id === reads.id)?.name ?? ""
}

// ── changes ──────────────────────────────────────────────────────────────

/** Whether another input may be added: below decision 4's 5, and only one when a connector feeds the pipeline (D12). */
export function canAddInput(state: EditorState, hasConnector: boolean): boolean {
  return state.inputs.length < (hasConnector ? 1 : MAX_INPUTS)
}

/** Whether another step may be added: below decision 15's 20. */
export function canAddStep(state: EditorState): boolean {
  return state.steps.length < MAX_STEPS
}

/** Adds an empty Bronze input. Nothing happens past the limit: the rail does not offer it then. */
export function addInput(state: EditorState, hasConnector = false): { state: EditorState; id: string | null } {
  if (!canAddInput(state, hasConnector)) return { state, id: null }
  const [id, next] = nextId(state, "input")
  return { state: { ...next, inputs: [...next.inputs, { id, name: "", zone: "bronze", table: "" }] }, id }
}

/**
 * Adds a step at the end. It reads what was last before it (the last step,
 * else the first input), so a new step continues the pipeline; a SQL step
 * starts as `SELECT *` from that name.
 */
export function addStep(state: EditorState, kind: "transform" | "sql"): { state: EditorState; id: string | null } {
  if (!canAddStep(state)) return { state, id: null }
  const [id, next] = nextId(state, "step")
  const names = [...state.inputs, ...state.steps].map((item) => item.name)
  const name = firstFree(`step_${state.steps.length + 1}`, names)
  const previous = readableItems(state, state.steps.length)
  const last = previous[previous.length - 1]
  const step: EditorStep =
    kind === "sql"
      ? { id, kind: "sql", name, sql: last ? `SELECT *\nFROM ${last.name}` : "SELECT *\nFROM " }
      : { id, kind: "transform", name, reads: last ? { id: last.id } : { name: "" }, transforms: [] }
  return { state: { ...next, steps: [...next.steps, step] }, id }
}

export function removeItem(state: EditorState, id: string): EditorState {
  return {
    ...state,
    inputs: state.inputs.filter((input) => input.id !== id),
    steps: state.steps.filter((step) => step.id !== id),
  }
}

/**
 * Renames an input or a step. A form step reads its item by id, so it
 * follows the rename with nothing to rewrite (R1-B2). The text of a SQL
 * step is the user's and is not rewritten: if it names the old name, the
 * page says so on that step. A step of an unknown kind keeps its stored
 * name (R1-S2).
 */
export function renameItem(state: EditorState, id: string, name: string): EditorState {
  return {
    ...state,
    inputs: state.inputs.map((input) => (input.id === id ? { ...input, name } : input)),
    steps: state.steps.map((step) => (step.id === id && step.kind !== "unknown" ? { ...step, name } : step)),
  }
}

/** Moves a step one place up (`-1`) or down (`+1`); nothing at either end. */
export function moveStep(state: EditorState, id: string, delta: -1 | 1): EditorState {
  const from = state.steps.findIndex((step) => step.id === id)
  const to = from + delta
  if (from < 0 || to < 0 || to >= state.steps.length) return state
  const steps = [...state.steps]
  const [step] = steps.splice(from, 1)
  steps.splice(to, 0, step)
  return { ...state, steps }
}

export function updateInput(state: EditorState, id: string, patch: Partial<Omit<EditorInput, "id">>): EditorState {
  return { ...state, inputs: state.inputs.map((input) => (input.id === id ? { ...input, ...patch } : input)) }
}

/**
 * Sets an input's table. While the input's name is blank or still the one
 * its previous table suggested, the name follows the table: picking
 * `orders` names the input `orders`, and a name the user typed is kept.
 */
export function setInputTable(state: EditorState, id: string, table: string): EditorState {
  const input = state.inputs.find((candidate) => candidate.id === id)
  if (!input) return state
  const others = otherNames(state, id)
  const followed = input.name === "" || input.name === suggestInputName(input.table, others)
  const withTable = updateInput(state, id, { table })
  return followed ? renameItem(withTable, id, suggestInputName(table, others)) : withTable
}

export function updateStep(
  state: EditorState,
  id: string,
  patch: { reads?: StepRead; transforms?: string[]; sql?: string }
): EditorState {
  return {
    ...state,
    steps: state.steps.map((step) => {
      if (step.id !== id) return step
      if (step.kind === "transform") {
        return { ...step, reads: patch.reads ?? step.reads, transforms: patch.transforms ?? step.transforms }
      }
      if (step.kind === "unknown") return step
      return { ...step, sql: patch.sql ?? step.sql }
    }),
  }
}

// ── the connector (BLD-7 decision 5, D12) ────────────────────────────────

/**
 * The inputs as they will be saved. With a connector the first input is the
 * connector's: its zone is Bronze and fixed, and its table is one of the
 * connector's targets, so a stored table that is not one reads as "nothing
 * chosen" instead of being sent for a 400 the user cannot see the cause of.
 * `targets` is `null` without a connector.
 */
export function effectiveInputs(inputs: readonly EditorInput[], targets: readonly string[] | null): EditorInput[] {
  if (targets === null) return [...inputs]
  return inputs.map((input, index) =>
    index === 0 ? { ...input, zone: "bronze", table: targets.includes(input.table) ? input.table : "" } : input
  )
}

// ── the check before saving (D2, D6a, BLD-10 D2) ─────────────────────────

export type OutputSettings = {
  targetZone: string
  targetTable: string
  /** BLD-10 D2: a pipeline with no step copies its input; that is chosen, not left over. */
  plainCopy: boolean
  hasConnector: boolean
}

/** What is wrong, by item id, plus the output's own problem. Empty means it can be saved. */
export type EditorProblems = { items: Record<string, string>; output: string | null }

/**
 * Every problem the page can know before the API is asked. One sentence per
 * item, the first that applies, so the pane shows one thing to fix at a time.
 */
export function editorProblems(state: EditorState, output: OutputSettings): EditorProblems {
  const items: Record<string, string> = {}
  state.inputs.forEach((input, index) => {
    const problem =
      nameProblem(input.name, otherNames(state, input.id)) ??
      (input.zone.trim() === "" ? "Choose a zone." : null) ??
      (input.table.trim() === "" ? "Choose a table." : null) ??
      (output.hasConnector && index > 0 ? "A pipeline fed by a connector reads one input: remove this one." : null)
    if (problem) items[input.id] = problem
  })
  state.steps.forEach((step, index) => {
    // A step of an unknown kind is not edited here, so nothing about it is
    // the page's to check; the API checks it on save (R1-S2).
    if (step.kind === "unknown") return
    let problem = nameProblem(step.name, otherNames(state, step.id))
    if (!problem && step.kind === "transform") {
      const reads = step.reads
      const later = "id" in reads && state.steps.slice(index).some((other) => other.id === reads.id)
      if (later) problem = "This step reads a step that comes after it; reorder the steps."
      else if (!("id" in reads) || !readableItems(state, index).some((item) => item.id === reads.id))
        problem = "Choose an input or an earlier step for this step to read."
      // BLD-10 D2: an empty form step is a copy that was not chosen.
      else if (step.transforms.length === 0) problem = "Add an operation, or remove this step."
    }
    if (!problem && step.kind === "sql") {
      if (step.sql.trim() === "") problem = "Write the step's SELECT."
      else if (step.sql.length > MAX_SQL_CHARS)
        problem = `A SQL step is at most ${MAX_SQL_CHARS.toLocaleString("en-US")} characters.`
    }
    if (problem) items[step.id] = problem
  })
  let outputProblem: string | null = null
  if (output.targetZone.trim() === "" || output.targetTable.trim() === "") outputProblem = "Choose the output's zone and table."
  else if (
    state.inputs.some(
      (input) => input.zone.trim() === output.targetZone.trim() && input.table.trim() === output.targetTable.trim()
    )
  )
    // `pipeline_compile::OUTPUT_IS_INPUT`, said before the round trip.
    outputProblem = "The output cannot be one of the inputs."
  else if (state.steps.length === 0 && state.inputs.length > 1)
    outputProblem = "With more than one input, add a step that reads them."
  else if (state.steps.length === 0 && !output.plainCopy)
    outputProblem = "Add a step, or choose to copy the input as it is."
  return { items, output: outputProblem }
}

/**
 * The rail's line under Steps when there is none, true for the state it is
 * said in. BLD-1 part 1 S3 review R1-B3, R1-S7: it said "the output is a
 * copy of the input" for any pipeline with no step, chosen or not, and with
 * several inputs.
 */
export function noStepLine(inputCount: number, plainCopy: boolean): string {
  if (inputCount > 1) return "No steps yet: with several inputs, add one that reads them."
  return plainCopy ? "No steps: the output is a copy of the input." : "No steps yet."
}

/** The Output pane's line saying what the output is written from (R1-S7). */
export function outputSourceLine(state: Pick<EditorState, "inputs" | "steps">, plainCopy: boolean): string {
  const last = state.steps[state.steps.length - 1]
  if (last) return last.name.trim() ? `Written from the last step, ${last.name.trim()}.` : "Written from the last step."
  if (state.inputs.length > 1) return "No step yet: with several inputs, the output is what a step returns."
  const input = state.inputs[0]?.name.trim() || "the input"
  return plainCopy ? `A copy of ${input}, unchanged.` : "No step yet: add one, or choose to copy the input as it is."
}

export function hasProblems(problems: EditorProblems): boolean {
  return problems.output !== null || Object.keys(problems.items).length > 0
}

// ── what is sent ─────────────────────────────────────────────────────────

/**
 * The `inputs` and `steps` a save sends (D1). Names, zones and tables are
 * trimmed; a SQL step's text goes as written (D4).
 */
export function definitionBody(inputs: readonly EditorInput[], steps: readonly EditorStep[]): {
  inputs: PipelineInput[]
  steps: PipelineStep[]
} {
  const state = { inputs: [...inputs], steps: [...steps] }
  return {
    inputs: inputs.map((input) => ({ name: input.name.trim(), zone: input.zone.trim(), table: input.table.trim() })),
    steps: steps.map((step): PipelineStep =>
      step.kind === "unknown"
        ? // R1-S2: exactly as stored.
          step.stored
        : step.kind === "sql"
          ? { name: step.name.trim(), kind: "sql", sql: step.sql }
          : { name: step.name.trim(), kind: "transform", reads: readsName(state, step.reads).trim(), transforms: [...step.transforms] }
    ),
  }
}

/**
 * Whether the inputs or steps differ from the stored ones: the change D10
 * moves the reader on (decision 14). The schedule, the description and the
 * output are not part of it.
 */
export function definitionChanged(
  inputs: readonly EditorInput[],
  steps: readonly EditorStep[],
  stored: Pick<AuthoredDefinition, "inputs" | "steps">
): boolean {
  const now = definitionBody(inputs, steps)
  const storedState = editorStateFrom(stored)
  const before = definitionBody(storedState.inputs, storedState.steps)
  return JSON.stringify(now) !== JSON.stringify(before)
}

/**
 * D17a: whether saving makes the pipeline read as the person saving. True
 * when the inputs or steps changed and the pipeline does not already read
 * as them, including when it reads as nobody (D10 then sets the reader to
 * the caller too). `userId` is `null` while the session is unknown, and the
 * line is said then rather than left out.
 */
export function readsAsYouAfterSave(storedRunAs: string | undefined, userId: string | null, changed: boolean): boolean {
  return changed && (userId === null || storedRunAs !== userId)
}

/** The sentence of D17a, said before the save. */
export const READS_AS_YOU_NOTICE = "This pipeline will read as you from now on."

/**
 * The body of a preview of the selected item (D16), or `null` when the item
 * has no name yet. An input is previewed alone, so an unfinished second
 * input does not stop it; a step is sent with every input and the steps up
 * to it, so an unfinished later step does not stop it either.
 */
export function previewBody(
  inputs: readonly EditorInput[],
  steps: readonly EditorStep[],
  selection: EditorSelection
): PipelinePreviewInput | null {
  if (selection.kind === "input") {
    const input = inputs.find((candidate) => candidate.id === selection.id)
    if (!input || input.name.trim() === "") return null
    const body = definitionBody([input], [])
    return { ...body, upTo: input.name.trim() }
  }
  // The output is what the last step returns, or the one input with no step.
  const index =
    selection.kind === "step" ? steps.findIndex((step) => step.id === selection.id) : steps.length - 1
  if (selection.kind === "step" && index < 0) return null
  if (index < 0) {
    return inputs.length === 1 ? previewBody(inputs, steps, { kind: "input", id: inputs[0].id }) : null
  }
  const step = steps[index]
  if (step.name.trim() === "") return null
  return { ...definitionBody(inputs, steps.slice(0, index + 1)), upTo: step.name.trim() }
}

/**
 * The key a preview was made for. A preview whose key differs from the one
 * the definition gives now is stale: the rows are of an earlier definition
 * (D17).
 */
export function previewKey(body: PipelinePreviewInput): string {
  return JSON.stringify(body)
}

/**
 * The preview's rows as records keyed by column, the shape the shared rows
 * table reads (`SqlRowsTable`). The API sends each row as an array in the
 * order of `columns` (plan section 3).
 */
export function previewAsRecords(preview: PipelinePreview): {
  columns: PipelinePreview["columns"]
  rows: Record<string, unknown>[]
} {
  return {
    columns: preview.columns,
    rows: preview.rows.map((row) => Object.fromEntries(preview.columns.map((column, i) => [column.name, row[i] ?? null]))),
  }
}

// ── errors from the API (D7) ─────────────────────────────────────────────

/** The item a refusal names. */
export type ErrorTarget =
  | { kind: "input"; index: number }
  | { kind: "step"; index: number; transform: number | null }

/**
 * The item an API message names: `inputs[i]`, `steps[i]` or
 * `steps[i].transforms[j]` (D7; `DefinitionError::path` in
 * `pipeline_compile.rs`). `null` when it names none, and the page shows the
 * message at the page level instead of guessing an item.
 */
export function errorTarget(message: string): ErrorTarget | null {
  const match = /\b(inputs|steps)\[(\d+)\](?:\.transforms\[(\d+)\])?/.exec(message)
  if (!match) return null
  const index = Number(match[2])
  if (match[1] === "inputs") return { kind: "input", index }
  return { kind: "step", index, transform: match[3] === undefined ? null : Number(match[3]) }
}

/** The id of the item a refusal names, in the state it was sent from; `null` when there is no such item. */
export function errorItemId(state: Pick<EditorState, "inputs" | "steps">, target: ErrorTarget | null): string | null {
  if (!target) return null
  const item = target.kind === "input" ? state.inputs[target.index] : state.steps[target.index]
  return item?.id ?? null
}

// ── columns and completion (D17) ─────────────────────────────────────────

export type Column = { name: string; type: string }

/**
 * The columns a `transform` step's pickers offer: those of what it reads.
 * An input's come from the lakehouse (`getTableDetail`); a step's come from
 * its last preview, when it has one. Both maps are keyed by item id. `null`
 * when they are not known, and the fields stay free text.
 */
export function columnsRead(
  step: Extract<EditorStep, { kind: "transform" }>,
  inputColumns: Readonly<Record<string, Column[] | null>>,
  stepColumns: Readonly<Record<string, Column[] | null>>
): string[] | null {
  if (!("id" in step.reads)) return null
  const columns = inputColumns[step.reads.id] ?? stepColumns[step.reads.id]
  return columns && columns.length > 0 ? columns.map((column) => column.name) : null
}

const LAYERS: readonly string[] = ["raw", "bronze", "silver", "gold", "semantic"]

/**
 * What the SQL editor of step `stepIndex` completes: the names it may read
 * (the inputs and the earlier steps), and the columns of each input. A step
 * name is offered without columns: they are not known until it runs.
 *
 * `layer` is part of `CompletionTable` for the catalog's schema browser,
 * which this editor does not show; the completion ignores it. An input
 * carries its zone when that is a layer; a step carries `silver`, which
 * nothing reads.
 */
export function stepCompletion(
  state: Pick<EditorState, "inputs" | "steps">,
  stepIndex: number,
  inputColumns: Readonly<Record<string, Column[] | null>>
): { tables: CompletionTable[]; columns: Record<string, ColumnState> } {
  const tables: CompletionTable[] = []
  const columns: Record<string, ColumnState> = {}
  for (const input of state.inputs) {
    const name = input.name.trim()
    if (name === "") continue
    tables.push({
      id: name,
      label: `${name} (${input.zone}.${input.table})`,
      reference: name,
      layer: (LAYERS.includes(input.zone) ? input.zone : "silver") as DataLayer,
      names: [name],
    })
    const known = inputColumns[input.id]
    if (known) columns[name] = { status: "ready", columns: known.map((column) => ({ name: column.name, dataType: column.type })) }
  }
  for (const step of state.steps.slice(0, Math.max(0, stepIndex))) {
    const name = step.name.trim()
    if (name === "") continue
    tables.push({ id: name, label: `${name} (step)`, reference: name, layer: "silver", names: [name] })
  }
  return { tables, columns }
}

// ── what a SQL step reads, for drawing (D17 read view) ───────────────────

/**
 * The names of `names` a SQL step reads right after a `FROM` or a `JOIN`,
 * in the order it first names them. The pipeline page draws an edge for
 * each; it is a drawing aid, not a check (the API's `StepReads` walk is
 * the check, D6a). It reads the text through the console's one scanner, so
 * a name inside a string or a comment is not taken; it compares as written,
 * case included, as the API does; and it leaves out what it cannot place
 * without a parser (a second table after a comma in `FROM a, b`), so an
 * edge it draws is one the step has and none is invented.
 */
export function sqlStepReads(sql: string, names: readonly string[]): string[] {
  const tokens = tokenizeSql(sql).filter((token) => token.kind !== "comment")
  const out: string[] = []
  tokens.forEach((token, i) => {
    const previous = tokens[i - 1]
    if (!previous || previous.kind !== "word" || !/^(FROM|JOIN)$/i.test(previous.text)) return
    // A backtick- or double-quoted name is the name inside the quotes.
    const name =
      token.kind === "word"
        ? token.text
        : token.kind === "quoted" && /^[`"]/.test(token.text)
          ? token.text.slice(1, -1)
          : null
    // `zone.table` is not a step's name (D6a refuses it); skip a qualified one.
    if (name === null || tokens[i + 1]?.text === ".") return
    if (names.includes(name) && !out.includes(name)) out.push(name)
  })
  return out
}
