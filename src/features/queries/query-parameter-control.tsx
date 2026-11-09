"use client"

import * as React from "react"

import { Button } from "@/components/ui/button"
import {
  Faceted,
  FacetedBadgeList,
  FacetedContent,
  FacetedEmpty,
  FacetedGroup,
  FacetedInput,
  FacetedItem,
  FacetedList,
  FacetedTrigger,
} from "@/components/ui/faceted"
import { Input } from "@/components/ui/input"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type {
  QueryDateRange,
  QueryParamDef,
  QueryParamOption,
  QueryParamValue,
} from "@/services/contracts/queries"

/**
 * The control that holds one parameter's value (`QS-5`): a text input, a
 * number input, a date picker, a start and an end date, a select, or a
 * multi-select, by the definition's type.
 *
 * The control reports a value in the wire shape of the type (a number is a
 * number, a range is `{ start, end }`, a multi-select is a list) and reports
 * `undefined` when the field holds nothing the server could read. It never
 * reports `null` or an empty text for "not given": an unset value is absent.
 * The definition menu uses the same control for a default, so a default has
 * the shape a value has.
 */
export function ParameterControl({
  id,
  def,
  value,
  invalid = false,
  onChange,
}: {
  /** The id of the control the field's label points at. */
  readonly id: string
  readonly def: QueryParamDef
  /** What the field holds; the caller passes the default when the tab has no value. */
  readonly value: QueryParamValue | undefined
  /** The server's last refusal blamed this parameter: the control is marked. */
  readonly invalid?: boolean
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  switch (def.type) {
    case "text":
    case "number":
      return <TypedInput id={id} type={def.type} value={value} invalid={invalid} onChange={onChange} />
    case "date":
      return (
        <Input
          id={id}
          type="date"
          aria-invalid={invalid || undefined}
          className="h-8 w-40"
          value={typeof value === "string" ? value : ""}
          onChange={(event) => onChange(event.target.value === "" ? undefined : event.target.value)}
        />
      )
    case "dateRange":
      return <RangeInputs id={id} value={value} invalid={invalid} onChange={onChange} />
    case "dropdown":
      return (
        <DropdownSelect
          id={id}
          options={def.options ?? []}
          value={value}
          invalid={invalid}
          onChange={onChange}
        />
      )
    case "multiSelect":
      return (
        <ListSelect
          id={id}
          options={def.options ?? []}
          value={value}
          invalid={invalid}
          onChange={onChange}
        />
      )
  }
}

/**
 * A text or number input. While the person edits it keeps what they typed,
 * so a field can be emptied even when a default would fill it again; on
 * blur it shows the stored value.
 */
function TypedInput({
  id,
  type,
  value,
  invalid,
  onChange,
}: {
  readonly id: string
  readonly type: "text" | "number"
  readonly value: QueryParamValue | undefined
  readonly invalid: boolean
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  const [draft, setDraft] = React.useState<string | null>(null)
  const stored = typeof value === "string" || typeof value === "number" ? String(value) : ""
  return (
    <Input
      id={id}
      type={type}
      aria-invalid={invalid || undefined}
      className={type === "number" ? "h-8 w-32" : "h-8 w-44"}
      value={draft ?? stored}
      onChange={(event) => {
        const raw = event.target.value
        setDraft(raw)
        if (raw === "") return onChange(undefined)
        if (type === "text") return onChange(raw)
        const parsed = Number(raw)
        onChange(Number.isFinite(parsed) ? parsed : undefined)
      }}
      onBlur={() => setDraft(null)}
    />
  )
}

function isRange(value: QueryParamValue | undefined): value is QueryDateRange {
  return typeof value === "object" && value !== null && !Array.isArray(value)
}

/** Whether two values are the same date range, or the same plain value. */
function sameValue(a: QueryParamValue | undefined, b: QueryParamValue | undefined) {
  if (isRange(a) && isRange(b)) return a.start === b.start && a.end === b.end
  return a === b
}

/**
 * Two date inputs. The value is `{ start, end }` only when both are set; one
 * end alone is a half-typed range, so the control keeps it and reports no
 * value until the other end comes.
 *
 * The half-typed draft is kept only while the range is incomplete, and only
 * while the person stays in it. It goes when focus leaves both inputs, and
 * when the value changes without the control having reported it (a saved
 * query opened into the same tab empties the values): the inputs then show
 * what the next run will use.
 */
function RangeInputs({
  id,
  value,
  invalid,
  onChange,
}: {
  readonly id: string
  readonly value: QueryParamValue | undefined
  readonly invalid: boolean
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  const [draft, setDraft] = React.useState<{ start: string; end: string } | null>(null)
  // The value the draft was typed against. It follows the value the control
  // itself reported (the render after `set`), and any other change drops the draft.
  const [base, setBase] = React.useState(value)
  const [reported, setReported] = React.useState(false)
  if (reported) {
    setReported(false)
    setBase(value)
  } else if (!sameValue(value, base)) {
    setBase(value)
    setDraft(null)
  }
  const start = draft?.start ?? (isRange(value) ? value.start : "")
  const end = draft?.end ?? (isRange(value) ? value.end : "")
  function set(part: "start" | "end", next: string) {
    const merged = { start, end, [part]: next }
    const complete = merged.start !== "" && merged.end !== ""
    setDraft(complete ? null : merged)
    setReported(true)
    onChange(complete ? { start: merged.start, end: merged.end } : undefined)
  }
  return (
    <div
      role="group"
      aria-labelledby={`${id}-label`}
      className="flex items-center gap-2"
      onBlur={(event) => {
        // Moving from one end to the other stays inside the range.
        if (!event.currentTarget.contains(event.relatedTarget as Node | null)) setDraft(null)
      }}
    >
      <label className="flex items-center gap-1 text-xs text-muted-foreground" htmlFor={`${id}-start`}>
        Start
      </label>
      <Input
        id={`${id}-start`}
        type="date"
        aria-invalid={invalid || undefined}
        className="h-8 w-40"
        value={start}
        onChange={(event) => set("start", event.target.value)}
      />
      <label className="flex items-center gap-1 text-xs text-muted-foreground" htmlFor={`${id}-end`}>
        End
      </label>
      <Input
        id={`${id}-end`}
        type="date"
        aria-invalid={invalid || undefined}
        className="h-8 w-40"
        value={end}
        onChange={(event) => set("end", event.target.value)}
      />
    </div>
  )
}

/** The option whose value reads as `text`, so a number option keeps being a number. */
function optionByText(options: readonly QueryParamOption[], text: string) {
  return options.find((option) => String(option.value) === text)
}

/**
 * A dropdown: a list of the options the definition holds, and a first item
 * that takes the value away again.
 *
 * A person who picked an option could not get back to "no value" before:
 * the list held the options and nothing else. The first item, "No value",
 * reports no value, so the field can be emptied again (#422 S4).
 */
function DropdownSelect({
  id,
  options,
  value,
  invalid,
  onChange,
}: {
  readonly id: string
  readonly options: readonly QueryParamOption[]
  readonly value: QueryParamValue | undefined
  readonly invalid: boolean
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  // The first item holds no value: `null` is how the select says "nothing is
  // chosen", and the label of that item is what the trigger reads then.
  const items = [
    { value: null, label: "No value" },
    ...options.map((option) => ({ value: String(option.value), label: option.label })),
  ]
  const chosen = typeof value === "string" || typeof value === "number" ? String(value) : null
  return (
    <Select
      value={chosen}
      items={items}
      onValueChange={(next) => {
        const option = next === null ? undefined : optionByText(options, next)
        onChange(option?.value)
      }}
    >
      <SelectTrigger id={id} size="sm" className="w-44" aria-invalid={invalid || undefined}>
        <SelectValue placeholder="Choose" />
      </SelectTrigger>
      <SelectContent>
        <SelectItem value={null}>No value</SelectItem>
        {options.map((option) => (
          <SelectItem key={String(option.value)} value={String(option.value)}>
            {option.label}
          </SelectItem>
        ))}
      </SelectContent>
    </Select>
  )
}

function ListSelect({
  id,
  options,
  value,
  invalid,
  onChange,
}: {
  readonly id: string
  readonly options: readonly QueryParamOption[]
  readonly value: QueryParamValue | undefined
  readonly invalid: boolean
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  // The faceted control speaks text; the wire keeps each option's own kind.
  const chosen = Array.isArray(value) ? value.map(String) : []
  return (
    <Faceted
      multiple
      value={chosen}
      onValueChange={(next) => {
        const picked = (next ?? []).flatMap((text) => optionByText(options, text)?.value ?? [])
        onChange(picked.length > 0 ? picked : undefined)
      }}
    >
      <FacetedTrigger asChild>
        <Button
          id={id}
          variant="outline"
          size="sm"
          className="w-52 font-normal"
          aria-invalid={invalid || undefined}
        >
          <FacetedBadgeList
            options={options.map((option) => ({ value: String(option.value), label: option.label }))}
            placeholder="Choose"
          />
        </Button>
      </FacetedTrigger>
      <FacetedContent className="w-56">
        <FacetedInput placeholder="Search" aria-label="Search options" />
        <FacetedList>
          <FacetedEmpty>No options.</FacetedEmpty>
          <FacetedGroup>
            {options.map((option) => (
              <FacetedItem key={String(option.value)} value={String(option.value)}>
                <span>{option.label}</span>
              </FacetedItem>
            ))}
          </FacetedGroup>
        </FacetedList>
      </FacetedContent>
    </Faceted>
  )
}
