/**
 * The pipeline editor's state and its rules (BLD-1 part 1, D2, D7, D16,
 * D17, D17a; BLD-16, J9), kept out of the component so each rule is
 * tested on its own (`CODE-STANDARD.md` 4.6).
 *
 * A pipeline is a list of inputs, an ordered list of steps and one output
 * table (D1). The editor holds the same shape with one addition: every input
 * and step carries a client-side `id` that never changes and is never
 * reused, so the selection, a preview and an error stay attached to the item
 * while its name is being typed, renamed or moved.
 *
 * BLD-16 / J1 adds the `join`, `aggregate` and `filter` form kinds. Their
 * reads follow the item by id, as `transform`'s do (R1-B2); the page checks
 * only what it can know without a round trip (a read not chosen, a join
 * with no pair, an empty row), and every other rule stays the API's (D7) —
 * the console has no SQL renderer of its own (J6): "Show as SQL" asks the
 * API, and "Switch to SQL step" writes the API's answer back as a SQL
 * step.
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
  PipelineAggregateStep,
  PipelineFilterStep,
  PipelineInput,
  PipelineJoinPair,
  PipelineJoinColumn,
  PipelineJoinStep,
  PipelineMeasure,
  PipelinePreview,
  PipelinePreviewInput,
  PipelineSqlStep,
  PipelineStep,
  PipelineStepCondition,
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
 *
 * BLD-16 / J1: the three form kinds. `left`, `right` and `reads` are
 * [`StepRead`]s — the item by its id, so a rename follows with nothing to
 * rewrite (R1-B2) — and are written as names only when the body is built.
 */
export type EditorStep =
  | { id: string; kind: "transform"; name: string; reads: StepRead; transforms: string[] }
  | { id: string; kind: "sql"; name: string; sql: string }
  | {
      id: string
      kind: "join"
      name: string
      left: StepRead
      right: StepRead
      joinType: string
      on: PipelineJoinPair[]
      columns: PipelineJoinColumn[]
    }
  | {
      id: string
      kind: "aggregate"
      name: string
      reads: StepRead
      groupBy: string[]
      measures: PipelineMeasure[]
      havingMatch: string
      having: PipelineStepCondition[]
    }
  | { id: string; kind: "filter"; name: string; reads: StepRead; match: string; conditions: PipelineStepCondition[] }
  | { id: string; kind: "unknown"; name: string; stored: PipelineStep }

/** A stored step of a kind this console knows how to edit, or `null`. */
export type KnownStep = PipelineTransformStep | PipelineSqlStep | PipelineJoinStep | PipelineAggregateStep | PipelineFilterStep

/** A `join` step's `on` pair, checked by field shape. */
function isJoinPair(pair: unknown): pair is PipelineJoinPair {
  const fields = pair as Record<string, unknown>
  return typeof fields.left === "string" && typeof fields.right === "string"
}

/** A join output column, checked by field shape. */
function isJoinColumn(column: unknown): column is PipelineJoinColumn {
  const fields = column as Record<string, unknown>
  return typeof fields.side === "string" && typeof fields.column === "string"
}

/** A filter or `having` condition, checked by field shape. */
function isCondition(condition: unknown): condition is PipelineStepCondition {
  const fields = condition as Record<string, unknown>
  return (
    typeof fields.column === "string" && typeof fields.operator === "string" && typeof fields.value === "string"
  )
}

/** An aggregate measure, checked by field shape. */
function isMeasure(measure: unknown): measure is PipelineMeasure {
  const fields = measure as Record<string, unknown>
  return (
    typeof fields.function === "string" &&
    typeof fields.name === "string" &&
    (fields.column === undefined || typeof fields.column === "string")
  )
}

/**
 * A stored step as one of the kinds this console knows, or `null`.
 * The fields are checked, not only `kind`: the API may add kinds, and a
 * step whose fields are not the ones its kind has is not edited as if
 * they were (R1-S2) — BLD-16's kinds included.
 */
export function knownStep(step: PipelineStep): KnownStep | null {
  const fields = step as Record<string, unknown>
  if (step.kind === "sql" && typeof fields.sql === "string") return step as PipelineSqlStep
  if (
    step.kind === "transform" &&
    typeof fields.reads === "string" &&
    Array.isArray(fields.transforms) &&
    (fields.transforms as unknown[]).every((t) => typeof t === "string")
  )
    return step as PipelineTransformStep
  // BLD-16 / J2: each form kind only when every field has its type; a join
  // whose `on` is not a list stays `unknown` and is sent back as stored.
  if (
    step.kind === "join" &&
    typeof fields.left === "string" &&
    typeof fields.right === "string" &&
    typeof fields.joinType === "string" &&
    Array.isArray(fields.on) &&
    (fields.on as unknown[]).every(isJoinPair) &&
    Array.isArray(fields.columns) &&
    (fields.columns as unknown[]).every(isJoinColumn)
  )
    return step as PipelineJoinStep
  if (
    step.kind === "aggregate" &&
    typeof fields.reads === "string" &&
    Array.isArray(fields.groupBy) &&
    (fields.groupBy as unknown[]).every((c) => typeof c === "string") &&
    Array.isArray(fields.measures) &&
    (fields.measures as unknown[]).every(isMeasure) &&
    (fields.havingMatch === undefined || typeof fields.havingMatch === "string") &&
    (fields.having === undefined || (Array.isArray(fields.having) && (fields.having as unknown[]).every(isCondition)))
  )
    return step as PipelineAggregateStep
  if (
    step.kind === "filter" &&
    typeof fields.reads === "string" &&
    (fields.match === undefined || typeof fields.match === "string") &&
    Array.isArray(fields.conditions) &&
    (fields.conditions as unknown[]).every(isCondition)
  )
    return step as PipelineFilterStep
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
    // BLD-16 / J2: a join reads two items, an aggregate and a filter one;
    // each `reads`/`left`/`right` is resolved to its item's id once, here.
    if (step.kind === "join") {
      const leftId = byName.get(step.left)
      const rightId = byName.get(step.right)
      return {
        id,
        kind: "join",
        name: step.name,
        left: leftId ? { id: leftId } : { name: step.left },
        right: rightId ? { id: rightId } : { name: step.right },
        joinType: step.joinType,
        on: step.on.map((pair) => ({ ...pair })),
        columns: step.columns.map((column) => ({ ...column })),
      }
    }
    if (step.kind === "aggregate") {
      const readId = byName.get(step.reads)
      return {
        id,
        kind: "aggregate",
        name: step.name,
        reads: readId ? { id: readId } : { name: step.reads },
        groupBy: [...step.groupBy],
        measures: step.measures.map((measure) => ({ ...measure })),
        havingMatch: step.havingMatch ?? "",
        having: (step.having ?? []).map((condition) => ({ ...condition })),
      }
    }
    if (step.kind === "filter") {
      const readId = byName.get(step.reads)
      return {
        id,
        kind: "filter",
        name: step.name,
        reads: readId ? { id: readId } : { name: step.reads },
        match: step.match ?? "",
        conditions: step.conditions.map((condition) => ({ ...condition })),
      }
    }
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
 *
 * BLD-16 / J9: a new `join` reads the last item on the LEFT and nothing
 * on the right (nothing is readable until a side is chosen); a new
 * `aggregate` or `filter` reads the last item. The rest starts empty, and
 * the page says what is missing (`editorProblems`).
 */
export function addStep(
  state: EditorState,
  kind: "transform" | "sql" | "join" | "aggregate" | "filter"
): { state: EditorState; id: string | null } {
  if (!canAddStep(state)) return { state, id: null }
  const [id, next] = nextId(state, "step")
  const names = [...state.inputs, ...state.steps].map((item) => item.name)
  const name = firstFree(`step_${state.steps.length + 1}`, names)
  const previous = readableItems(state, state.steps.length)
  const last = previous[previous.length - 1]
  const readsLast: StepRead = last ? { id: last.id } : { name: "" }
  const step: EditorStep =
    kind === "sql"
      ? { id, kind: "sql", name, sql: last ? `SELECT *\nFROM ${last.name}` : "SELECT *\nFROM " }
      : kind === "join"
        ? {
            id,
            kind: "join",
            name,
            left: readsLast,
            right: { name: "" },
            joinType: "inner",
            on: [],
            columns: [],
          }
        : kind === "aggregate"
          ? { id, kind: "aggregate", name, reads: readsLast, groupBy: [], measures: [], havingMatch: "all", having: [] }
          : kind === "filter"
            ? { id, kind: "filter", name, reads: readsLast, match: "all", conditions: [] }
            : { id, kind: "transform", name, reads: readsLast, transforms: [] }
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
 * Sets an input's table. While the input's name is blank, or still the one
 * its previous table suggested and no step names it, the name follows the
 * table: picking `orders` names the input `orders`, and a name the user
 * typed is kept. BLD-1 part 1 S3 review R1-S1: once a step names the input,
 * its name stays, so a SQL step that reads it by name is not left behind.
 */
export function setInputTable(state: EditorState, id: string, table: string): EditorState {
  const input = state.inputs.find((candidate) => candidate.id === id)
  if (!input) return state
  const others = otherNames(state, id)
  const followed =
    input.name === "" || (input.name === suggestInputName(input.table, others) && !inputIsNamed(state, id))
  const withTable = updateInput(state, id, { table })
  return followed ? renameItem(withTable, id, suggestInputName(table, others)) : withTable
}

/**
 * Changes one step's fields, per kind. A step of an unknown kind is not
 * edited here (R1-S2); the API checks what this page cannot (D7).
 */
export type StepPatch =
  | { reads?: StepRead; transforms?: string[] }
  | { sql?: string }
  | { left?: StepRead; right?: StepRead; joinType?: string; on?: PipelineJoinPair[]; columns?: PipelineJoinColumn[] }
  | { groupBy?: string[]; measures?: PipelineMeasure[]; havingMatch?: string; having?: PipelineStepCondition[] }
  | { match?: string; conditions?: PipelineStepCondition[] }

export function updateStep(state: EditorState, id: string, patch: StepPatch): EditorState {
  return {
    ...state,
    steps: state.steps.map((step) => {
      if (step.id !== id) return step
      if (step.kind === "transform") {
        const fields = patch as { reads?: StepRead; transforms?: string[] }
        return { ...step, reads: fields.reads ?? step.reads, transforms: fields.transforms ?? step.transforms }
      }
      if (step.kind === "join") {
        const fields = patch as {
          left?: StepRead
          right?: StepRead
          joinType?: string
          on?: PipelineJoinPair[]
          columns?: PipelineJoinColumn[]
        }
        return {
          ...step,
          left: fields.left ?? step.left,
          right: fields.right ?? step.right,
          joinType: fields.joinType ?? step.joinType,
          on: fields.on ?? step.on,
          columns: fields.columns ?? step.columns,
        }
      }
      if (step.kind === "aggregate") {
        const fields = patch as {
          groupBy?: string[]
          measures?: PipelineMeasure[]
          havingMatch?: string
          having?: PipelineStepCondition[]
        }
        return {
          ...step,
          groupBy: fields.groupBy ?? step.groupBy,
          measures: fields.measures ?? step.measures,
          havingMatch: fields.havingMatch ?? step.havingMatch,
          having: fields.having ?? step.having,
        }
      }
      if (step.kind === "filter") {
        const fields = patch as { match?: string; conditions?: PipelineStepCondition[] }
        return { ...step, match: fields.match ?? step.match, conditions: fields.conditions ?? step.conditions }
      }
      if (step.kind === "unknown") return step
      const fields = patch as { sql?: string }
      return { ...step, sql: fields.sql ?? step.sql }
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

/**
 * The control a problem is about, so the pane ties the sentence to that
 * control (`aria-describedby`, `aria-invalid`; BLD-1 part 1 S3 review
 * R1-S10). BLD-16 adds the form kinds' own fields.
 */
export type ProblemField =
  | "name"
  | "zone"
  | "table"
  | "reads"
  | "operations"
  | "sql"
  | "output-zone"
  | "output-table"
  | "plain-copy"
  | "left"
  | "right"
  | "on"
  | "columns"
  | "groupBy"
  | "measures"
  | "having"
  | "conditions"

export type Problem = { message: string; field: ProblemField }

/** What is wrong, by item id, plus the output's own problem. Empty means it can be saved. */
export type EditorProblems = { items: Record<string, Problem>; output: Problem | null }

/**
 * Every problem the page can know before the API is asked. One sentence per
 * item, the first that applies, so the pane shows one thing to fix at a time.
 */
export function editorProblems(state: EditorState, output: OutputSettings): EditorProblems {
  const items: Record<string, Problem> = {}
  const at = (field: ProblemField) => (message: string | null): Problem | null => (message ? { message, field } : null)
  state.inputs.forEach((input, index) => {
    const problem =
      at("name")(nameProblem(input.name, otherNames(state, input.id))) ??
      at("zone")(input.zone.trim() === "" ? "Choose a zone." : null) ??
      at("table")(input.table.trim() === "" ? "Choose a table." : null) ??
      at("name")(output.hasConnector && index > 0 ? "A pipeline fed by a connector reads one input: remove this one." : null)
    if (problem) items[input.id] = problem
  })
  state.steps.forEach((step, index) => {
    // A step of an unknown kind is not edited here, so nothing about it is
    // the page's to check; the API checks it on save (R1-S2).
    if (step.kind === "unknown") return
    let problem = at("name")(nameProblem(step.name, otherNames(state, step.id)))
    // BLD-16 / J9: only what the page can know without the API — a read
    // not chosen or later, an empty list, an empty row. Everything else
    // (a duplicate output name, a measure's name a column already has, a
    // value's length) is the API's (D7). `what === "step"` keeps the
    // part 1 sentences byte for byte; a join's sides get their own.
    const readProblem = (field: ProblemField, reads: StepRead, what: string): Problem | null => {
      const later = "id" in reads && state.steps.slice(index).some((other) => other.id === reads.id)
      if (later)
        return at(field)(
          what === "step"
            ? "This step reads a step that comes after it; reorder the steps."
            : `The ${what} reads a step that comes after it; reorder the steps.`
        )
      if (!("id" in reads) || !readableItems(state, index).some((item) => item.id === reads.id))
        return at(field)(
          what === "step" ? "Choose an input or an earlier step for this step to read." : `Choose an input or an earlier step for the ${what} to read.`
        )
      return null
    }
    if (!problem && step.kind === "transform") {
      problem =
        readProblem("reads", step.reads, "step") ??
        // BLD-10 D2: an empty form step is a copy that was not chosen.
        at("operations")(step.transforms.length === 0 ? "Add an operation, or remove this step." : null)
    }
    if (!problem && step.kind === "sql") {
      if (step.sql.trim() === "") problem = at("sql")("Write the step's SELECT.")
      else if (step.sql.length > MAX_SQL_CHARS)
        problem = at("sql")(`A SQL step is at most ${MAX_SQL_CHARS.toLocaleString("en-US")} characters.`)
    }
    if (!problem && step.kind === "join") {
      problem =
        readProblem("left", step.left, "left side") ??
        readProblem("right", step.right, "right side") ??
        at("on")(step.on.length === 0 ? "Add at least one pair of columns to match on." : null) ??
        at("on")(
          step.on.some((pair) => pair.left.trim() === "" || pair.right.trim() === "")
            ? "Choose a column on both sides of every pair."
            : null
        ) ??
        at("columns")(step.columns.length === 0 ? "Choose at least one output column." : null) ??
        at("columns")(
          step.columns.some((column) => column.column.trim() === "")
            ? "Choose a column for every output column."
            : null
        )
    }
    if (!problem && step.kind === "aggregate") {
      problem =
        readProblem("reads", step.reads, "step") ??
        at("groupBy")(
          step.groupBy.length === 0 && step.measures.length === 0
            ? "Add a column to group by or a measure."
            : null
        ) ??
        at("groupBy")(
          step.groupBy.some((column) => column.trim() === "") ? "Choose a column for every group-by entry." : null
        ) ??
        at("measures")(
          step.measures.some((measure) => measure.function !== "count" && (measure.column ?? "").trim() === "")
            ? "Choose a column for every measure."
            : null
        )
    }
    if (!problem && step.kind === "filter") {
      problem =
        readProblem("reads", step.reads, "step") ??
        at("conditions")(step.conditions.length === 0 ? "Add at least one condition." : null) ??
        at("conditions")(
          step.conditions.some((condition) => condition.column.trim() === "")
            ? "Choose a column for every condition."
            : null
        ) ??
        at("conditions")(
          step.conditions.some((condition) => condition.operator.trim() === "")
            ? "Choose an operator for every condition."
            : null
        )
    }
    if (problem) items[step.id] = problem
  })
  let outputProblem: Problem | null = null
  if (output.targetZone.trim() === "") outputProblem = { message: "Choose the output's zone.", field: "output-zone" }
  else if (output.targetTable.trim() === "") outputProblem = { message: "Choose the output's table.", field: "output-table" }
  else if (
    state.inputs.some(
      (input) => input.zone.trim() === output.targetZone.trim() && input.table.trim() === output.targetTable.trim()
    )
  )
    // `pipeline_compile::OUTPUT_IS_INPUT`, said before the round trip.
    outputProblem = { message: "The output cannot be one of the inputs.", field: "output-table" }
  else if (state.steps.length === 0 && state.inputs.length > 1)
    outputProblem = { message: "With more than one input, add a step that reads them.", field: "plain-copy" }
  else if (state.steps.length === 0 && !output.plainCopy)
    outputProblem = { message: "Add a step, or choose to copy the input as it is.", field: "plain-copy" }
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

/** One reason the page cannot save yet, and where to go to fix it. */
export type SaveBlocker = { text: string; target: EditorSelection | { kind: "name" } | { kind: "schedule" } }

/**
 * Why Save is disabled, in the order of the page, each with where to go
 * (BLD-1 part 1 S3 review R1-S3). A stored pipeline the API accepts can
 * fail a rule only this page has (a form step with no operation, several
 * inputs and no step), and a missing name or schedule disabled Save with no
 * word; the page now says each one next to the button.
 */
export function saveBlockers(
  state: Pick<EditorState, "inputs" | "steps">,
  problems: EditorProblems,
  page: { name: string; scheduleOk: boolean }
): SaveBlocker[] {
  const out: SaveBlocker[] = []
  if (page.name.trim() === "") out.push({ text: "Give the pipeline a name.", target: { kind: "name" } })
  state.inputs.forEach((input, i) => {
    const problem = problems.items[input.id]
    if (problem) out.push({ text: `Input ${input.name.trim() || i + 1}: ${problem.message}`, target: { kind: "input", id: input.id } })
  })
  state.steps.forEach((step, i) => {
    const problem = problems.items[step.id]
    if (problem) out.push({ text: `Step ${step.name.trim() || i + 1}: ${problem.message}`, target: { kind: "step", id: step.id } })
  })
  if (problems.output) out.push({ text: `Output: ${problems.output.message}`, target: { kind: "output" } })
  if (!page.scheduleOk)
    out.push({ text: "Schedule: choose one the orchestrator can run, or On demand only.", target: { kind: "schedule" } })
  return out
}

// ── what is sent ─────────────────────────────────────────────────────────

/**
 * The `inputs` and `steps` a save sends (D1). Names, zones and tables are
 * trimmed; a SQL step's text goes as written (D4). BLD-16 / J2: the form
 * kinds go as the API's serde shape — a join's unnamed output column and a
 * `count` measure's absent column carry no key, exactly as the API writes
 * them back, so a definition the editor did not change compares equal.
 */
export function definitionBody(inputs: readonly EditorInput[], steps: readonly EditorStep[]): {
  inputs: PipelineInput[]
  steps: PipelineStep[]
} {
  const state = { inputs: [...inputs], steps: [...steps] }
  const condition = (c: PipelineStepCondition): PipelineStepCondition =>
    ({ column: c.column.trim(), operator: c.operator, value: c.value })
  const joinColumn = (column: PipelineJoinColumn): PipelineJoinColumn => {
    const name = column.name?.trim() ?? ""
    // The API treats an absent `name` as "the column's own"; sending an
    // empty one is the same thing, but keeping the key absent matches what
    // the API sends back, so an unchanged definition stays unchanged.
    const out: PipelineJoinColumn = { side: column.side, column: column.column.trim() }
    if (name !== "") out.name = name
    return out
  }
  const measure = (m: PipelineMeasure): PipelineMeasure => {
    const out: PipelineMeasure = { function: m.function, name: m.name.trim() }
    const column = m.column?.trim() ?? ""
    if (column !== "") out.column = column
    return out
  }
  return {
    inputs: inputs.map((input) => ({ name: input.name.trim(), zone: input.zone.trim(), table: input.table.trim() })),
    steps: steps.map((step): PipelineStep => {
      if (step.kind === "unknown")
        // R1-S2: exactly as stored.
        return step.stored
      if (step.kind === "sql") return { name: step.name.trim(), kind: "sql", sql: step.sql }
      if (step.kind === "join")
        return {
          name: step.name.trim(),
          kind: "join",
          left: readsName(state, step.left).trim(),
          right: readsName(state, step.right).trim(),
          joinType: step.joinType,
          on: step.on.map((pair) => ({ left: pair.left.trim(), right: pair.right.trim() })),
          columns: step.columns.map(joinColumn),
        }
      if (step.kind === "aggregate")
        return {
          name: step.name.trim(),
          kind: "aggregate",
          reads: readsName(state, step.reads).trim(),
          groupBy: step.groupBy.map((column) => column.trim()),
          measures: step.measures.map(measure),
          havingMatch: step.havingMatch,
          having: step.having.map(condition),
        }
      if (step.kind === "filter")
        return {
          name: step.name.trim(),
          kind: "filter",
          reads: readsName(state, step.reads).trim(),
          match: step.match,
          conditions: step.conditions.map(condition),
        }
      return {
        name: step.name.trim(),
        kind: "transform",
        reads: readsName(state, step.reads).trim(),
        transforms: [...step.transforms],
      }
    }),
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

// ── BLD-16 (BLD-1 part 2) / J6, J9 ────────────────────────────────────────

/**
 * J6, "Switch to SQL step" (one way): the step becomes a SQL step that
 * carries the API's own text for it, same id, same name, same place; every
 * other item is untouched and nothing is saved until Save.
 */
export function switchToSql(state: EditorState, id: string, sql: string): EditorState {
  return {
    ...state,
    steps: state.steps.map((step) => (step.id === id ? { id: step.id, kind: "sql", name: step.name, sql } : step)),
  }
}

/**
 * J9: the output columns a new join step (or a change of side) is filled
 * with — every column of the left, then every column of the right. A
 * right column whose name is already taken (without regard to case, as the
 * API's duplicate rule reads, D2) is named `<right item name>_<column>`,
 * then `_2`, `_3`, … while still taken. Columns the API would refuse (not
 * letters, digits and underscores) are the pane's to leave out
 * (`formColumns`), not this rule's.
 */
export function joinOutputColumns(
  leftName: string,
  leftColumns: readonly Column[],
  rightName: string,
  rightColumns: readonly Column[]
): PipelineJoinColumn[] {
  const names: string[] = []
  const out: PipelineJoinColumn[] = []
  const claim = (name: string): string => {
    // The API refuses duplicate output names without regard to case, so
    // the collision check reads the same way.
    names.push(name.toLowerCase())
    return name
  }
  for (const column of leftColumns) {
    out.push({ side: "left", column: column.name })
    claim(column.name)
  }
  for (const column of rightColumns) {
    if (!taken(names, column.name)) {
      out.push({ side: "right", column: column.name })
      claim(column.name)
      continue
    }
    const name = firstFree(`${rightName}_${column.name}`, names)
    names.push(name.toLowerCase())
    out.push({ side: "right", column: column.name, name })
  }
  return out
}

/**
 * J9: a new measure's name — `<function>_<column>`, or `count` for a
 * `count` with no column — made unique against `takenNames` (the step's
 * group-by columns and earlier measure names).
 */
export function measureName(fn: string, column: string, takenNames: readonly string[]): string {
  const base = column.trim() === "" && fn === "count" ? "count" : `${fn}_${column.trim()}`
  return firstFree(base, takenNames)
}

/**
 * J9: the columns a form step's pickers offer — those the API would accept
 * (`Ident::new`, the same regular expression as [`IDENT`]) — and whether
 * any was left out, so the pane can say why it is missing.
 */
export function formColumns(columns: readonly Column[]): { columns: Column[]; dropped: boolean } {
  const kept = columns.filter((column) => IDENT.test(column.name))
  return { columns: kept, dropped: kept.length !== columns.length }
}

/**
 * The columns of what one read names: an input's from the lakehouse
 * (`getTableDetail`), a step's from its last preview. `null` when they are
 * not known, and the pane offers a "Load columns" button instead.
 */
export function readColumns(
  read: StepRead,
  inputColumns: Readonly<Record<string, Column[] | null>>,
  stepColumns: Readonly<Record<string, Column[] | null>>
): Column[] | null {
  if (!("id" in read)) return null
  const columns = inputColumns[read.id] ?? stepColumns[read.id]
  return columns && columns.length > 0 ? columns : null
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
 * A preview that can run: the body to send, and which editor item each of
 * its inputs and steps is, so a refusal that names `inputs[i]` or
 * `steps[i]` of that body is put on the right item (R1-S5).
 */
export type PreviewRequest = { body: PipelinePreviewInput; inputIds: string[]; stepIds: string[] }

/**
 * The preview of the selected item (D16), or why there is none yet, said
 * for the item it is about (R1-S7).
 *
 * An input is previewed alone, so an unfinished second input does not stop
 * it. A step is sent with the steps up to it and with every input that has
 * a name and a table: BLD-1 part 1 S3 review R1-S5, an input still being
 * filled in no longer blocks every step's preview. If the step reads it, the
 * API says so, on that step.
 */
export function previewRequest(
  inputs: readonly EditorInput[],
  steps: readonly EditorStep[],
  selection: EditorSelection
): PreviewRequest | { reason: string } {
  const complete = (input: EditorInput) => input.name.trim() !== "" && input.table.trim() !== ""
  if (selection.kind === "input") {
    const input = inputs.find((candidate) => candidate.id === selection.id)
    if (!input) return { reason: "Nothing to preview." }
    if (input.name.trim() === "") return { reason: "Give this input a name first; the preview runs it by its name." }
    if (input.table.trim() === "") return { reason: "Choose this input's table first." }
    return { body: { ...definitionBody([input], []), upTo: input.name.trim() }, inputIds: [input.id], stepIds: [] }
  }
  // The output is what the last step returns, or the one input with no step.
  const index = selection.kind === "step" ? steps.findIndex((step) => step.id === selection.id) : steps.length - 1
  if (selection.kind === "step" && index < 0) return { reason: "Nothing to preview." }
  if (index < 0) {
    return inputs.length === 1
      ? previewRequest(inputs, steps, { kind: "input", id: inputs[0].id })
      : { reason: "Add a step first: with several inputs, the output is what a step returns." }
  }
  const step = steps[index]
  if (step.name.trim() === "") return { reason: "Give this step a name first; the preview runs it by its name." }
  const sentInputs = inputs.filter(complete)
  const sentSteps = steps.slice(0, index + 1)
  // Names are resolved against every item, so a form step that reads an
  // input left out of the body still says which one it reads.
  const full = definitionBody(inputs, steps)
  const body: PipelinePreviewInput = {
    inputs: full.inputs.filter((_, i) => complete(inputs[i])),
    steps: full.steps.slice(0, index + 1),
    upTo: step.name.trim(),
  }
  return { body, inputIds: sentInputs.map((input) => input.id), stepIds: sentSteps.map((sent) => sent.id) }
}

/** The body of a preview of the selected item, or `null` when there is none yet (`previewRequest`). */
export function previewBody(
  inputs: readonly EditorInput[],
  steps: readonly EditorStep[],
  selection: EditorSelection
): PipelinePreviewInput | null {
  const request = previewRequest(inputs, steps, selection)
  return "body" in request ? request.body : null
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
  | {
      kind: "step"
      index: number
      /** The `transforms[j]` the refusal named, or null — kept for the part 1 panes. */
      transform: number | null
      /** BLD-16 / J2: the `<field>` the refusal named (`on`, `columns`, `groupBy`, `measures`, `conditions`, `having`, …), or null. */
      field: string | null
      /** BLD-16 / J2: the `[j]` the refusal named, or null. */
      row: number | null
    }

/**
 * The item an API message names: `inputs[i]`, `steps[i]`,
 * `steps[i].transforms[j]`, or a form kind's `steps[i].<field>[j]` (D7;
 * `DefinitionError::path` in `pipeline_compile.rs`). `null` when it names
 * none, and the page shows the message at the page level instead of
 * guessing an item.
 */
export function errorTarget(message: string): ErrorTarget | null {
  const match = /\b(inputs|steps)\[(\d+)\](?:\.(\w+))?(?:\[(\d+)\])?/.exec(message)
  if (!match) return null
  const index = Number(match[2])
  if (match[1] === "inputs") return { kind: "input", index }
  return {
    kind: "step",
    index,
    transform: match[3] === "transforms" && match[4] !== undefined ? Number(match[4]) : null,
    field: match[3] ?? null,
    row: match[4] === undefined ? null : Number(match[4]),
  }
}

/**
 * The id of the item a refusal names, through the body that was sent: its
 * `i`-th input or step is the `i`-th id of what was sent (R1-S5; a preview
 * sends a part of the definition, so the index is not the editor's).
 * `null` when there is no such item.
 */
export function errorItemId(sent: { inputIds: readonly string[]; stepIds: readonly string[] }, target: ErrorTarget | null): string | null {
  if (!target) return null
  return (target.kind === "input" ? sent.inputIds[target.index] : sent.stepIds[target.index]) ?? null
}

// ── columns and completion (D17) ─────────────────────────────────────────

export type Column = { name: string; type: string }

/**
 * The columns a form step's pickers offer: those of what it reads. An
 * input's come from the lakehouse (`getTableDetail`); a step's come from
 * its last preview, when it has one. Both maps are keyed by item id. `null`
 * when they are not known, and the fields stay free text.
 *
 * BLD-16: every form kind with ONE read (`transform`, `aggregate`,
 * `filter`); a `join` has two, and its panes ask [`readColumns`] per side.
 */
export function columnsRead(
  step: Extract<EditorStep, { kind: "transform" | "aggregate" | "filter" }>,
  inputColumns: Readonly<Record<string, Column[] | null>>,
  stepColumns: Readonly<Record<string, Column[] | null>>
): string[] | null {
  const columns = readColumns(step.reads, inputColumns, stepColumns)
  return columns ? columns.map((column) => column.name) : null
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

// ── what a SQL step names (D17 read view, R1-S1 hint) ───────────────────

/**
 * Words that, right before a `(`, open a subquery or a grouping rather than
 * a function call. After any other word a `(` is a call (`EXTRACT(`,
 * `TRIM(`, `count(`), and a `FROM` inside it is not a table position.
 */
const NOT_A_CALL = new Set([
  "ALL", "AND", "ANY", "AS", "BY", "ELSE", "EXISTS", "FROM", "HAVING", "IN", "JOIN", "LATERAL", "NOT", "ON",
  "OR", "SELECT", "SOME", "THEN", "UNION", "USING", "VALUES", "WHEN", "WHERE", "WITH",
])

/** One name a SQL step writes right after `FROM` or `JOIN`, outside any function call. */
type TableWord = { name: string; qualified: boolean }

/**
 * The names a SQL step writes in a table position: right after `FROM` or
 * `JOIN`, not inside a function call's brackets (`EXTRACT(year FROM paid)`
 * names no table), not in a string or a comment (the console's one
 * scanner), as written, case included. A name followed by `.` is a
 * qualified one (`zone.table`). A second table after a comma (`FROM a, b`)
 * is not placed: without a parser it cannot be told from a column list.
 */
function tableWords(sql: string): TableWord[] {
  const tokens = tokenizeSql(sql).filter((token) => token.kind !== "comment")
  const calls: boolean[] = []
  const out: TableWord[] = []
  tokens.forEach((token, i) => {
    const previous = tokens[i - 1]
    if (token.kind === "punctuation" && token.text === "(") {
      calls.push(previous?.kind === "word" && !NOT_A_CALL.has(previous.text.toUpperCase()))
      return
    }
    if (token.kind === "punctuation" && token.text === ")") {
      calls.pop()
      return
    }
    if (calls.includes(true)) return
    if (!previous || previous.kind !== "word" || !/^(FROM|JOIN)$/i.test(previous.text)) return
    // A backtick- or double-quoted name is the name inside the quotes.
    const name =
      token.kind === "word"
        ? token.text
        : token.kind === "quoted" && /^[`"]/.test(token.text)
          ? token.text.slice(1, -1)
          : null
    if (name === null) return
    const qualified = tokens[i + 1]?.text === "."
    out.push({ name: qualified ? `${name}.${tokens[i + 2]?.text ?? ""}` : name, qualified })
  })
  return out
}

/** The names a step declares in its own `WITH` (`name AS (`), which it may read. */
function ownWithNames(sql: string): string[] {
  const tokens = tokenizeSql(sql).filter((token) => token.kind !== "comment")
  return tokens
    .filter(
      (token, i) =>
        token.kind === "word" &&
        tokens[i + 1]?.kind === "word" &&
        tokens[i + 1]?.text.toUpperCase() === "AS" &&
        tokens[i + 2]?.text === "("
    )
    .map((token) => token.text)
}

/**
 * The names of `names` a SQL step reads in a table position, in the order
 * it first names them (`tableWords`). The pipeline page draws an edge for
 * each; it is a drawing aid, not a check (the API's `StepReads` walk is the
 * check, D6a). What it cannot place is left out rather than guessed, and a
 * name in a function call, a string or a comment is not taken
 * (BLD-1 part 1 S3 review R1-S8).
 */
export function sqlStepReads(sql: string, names: readonly string[]): string[] {
  const out: string[] = []
  for (const word of tableWords(sql)) {
    if (!word.qualified && names.includes(word.name) && !out.includes(word.name)) out.push(word.name)
  }
  return out
}

/**
 * A hint for a SQL step that names, in a table position, something that is
 * not an input, an earlier step or a `WITH` name of its own; `null` when it
 * names none. BLD-1 part 1 S3 review R1-S1: so a rename or a table change
 * that leaves the SQL behind is said on the page, not first by the API's
 * refusal on save. The API stays the authority (D6a); this does not block a
 * save.
 */
export function sqlStepHint(sql: string, readable: readonly string[]): string | null {
  const own = ownWithNames(sql)
  for (const word of tableWords(sql)) {
    if (word.qualified)
      return `This step reads ${word.name}; a step reads an input by its name in this pipeline. Add the table under Inputs.`
    if (!readable.includes(word.name) && !own.includes(word.name))
      return `This step reads ${word.name}, which is not an input or an earlier step.`
  }
  return null
}

/**
 * Whether a step names the input `id`: a form step that reads it — a
 * `transform`, `aggregate` or `filter` step, or a `join` on either side —
 * or a SQL step whose text has its name as a word (outside strings and
 * comments). Broad on purpose: it decides only whether the name may still
 * follow the table (R1-S1), and keeping a name is the safe answer.
 */
function inputIsNamed(state: EditorState, id: string): boolean {
  const name = state.inputs.find((input) => input.id === id)?.name ?? ""
  return state.steps.some((step) => {
    if (step.kind === "transform" || step.kind === "aggregate" || step.kind === "filter")
      return "id" in step.reads && step.reads.id === id
    if (step.kind === "join") return ("id" in step.left && step.left.id === id) || ("id" in step.right && step.right.id === id)
    if (step.kind !== "sql" || name === "") return false
    return tokenizeSql(step.sql).some(
      (token) =>
        (token.kind === "word" && token.text === name) || (token.kind === "quoted" && /^[`"]/.test(token.text) && token.text.slice(1, -1) === name)
    )
  })
}
