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
  onChange,
}: {
  /** The id of the control the field's label points at. */
  readonly id: string
  readonly def: QueryParamDef
  /** What the field holds; the caller passes the default when the tab has no value. */
  readonly value: QueryParamValue | undefined
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  switch (def.type) {
    case "text":
    case "number":
      return <TypedInput id={id} type={def.type} value={value} onChange={onChange} />
    case "date":
      return (
        <Input
          id={id}
          type="date"
          className="h-8 w-40"
          value={typeof value === "string" ? value : ""}
          onChange={(event) => onChange(event.target.value === "" ? undefined : event.target.value)}
        />
      )
    case "dateRange":
      return <RangeInputs id={id} value={value} onChange={onChange} />
    case "dropdown":
      return <DropdownSelect id={id} options={def.options ?? []} value={value} onChange={onChange} />
    case "multiSelect":
      return <ListSelect id={id} options={def.options ?? []} value={value} onChange={onChange} />
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
  onChange,
}: {
  readonly id: string
  readonly type: "text" | "number"
  readonly value: QueryParamValue | undefined
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  const [draft, setDraft] = React.useState<string | null>(null)
  const stored = typeof value === "string" || typeof value === "number" ? String(value) : ""
  return (
    <Input
      id={id}
      type={type}
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

/**
 * Two date inputs. The value is `{ start, end }` only when both are set; one
 * end alone is a half-typed range, so the control keeps it and reports no
 * value until the other end comes.
 */
function RangeInputs({
  id,
  value,
  onChange,
}: {
  readonly id: string
  readonly value: QueryParamValue | undefined
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  const [draft, setDraft] = React.useState<{ start?: string; end?: string }>({})
  const start = draft.start ?? (isRange(value) ? value.start : "")
  const end = draft.end ?? (isRange(value) ? value.end : "")
  function set(part: "start" | "end", next: string) {
    const merged = { start, end, [part]: next }
    setDraft(merged)
    onChange(merged.start && merged.end ? { start: merged.start, end: merged.end } : undefined)
  }
  return (
    <div className="flex items-center gap-2">
      <label className="flex items-center gap-1 text-xs text-muted-foreground" htmlFor={`${id}-start`}>
        Start
      </label>
      <Input
        id={`${id}-start`}
        type="date"
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

function DropdownSelect({
  id,
  options,
  value,
  onChange,
}: {
  readonly id: string
  readonly options: readonly QueryParamOption[]
  readonly value: QueryParamValue | undefined
  readonly onChange: (value: QueryParamValue | undefined) => void
}) {
  const items = options.map((option) => ({ value: String(option.value), label: option.label }))
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
      <SelectTrigger id={id} size="sm" className="w-44">
        <SelectValue placeholder="Choose" />
      </SelectTrigger>
      <SelectContent>
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
  onChange,
}: {
  readonly id: string
  readonly options: readonly QueryParamOption[]
  readonly value: QueryParamValue | undefined
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
        <Button id={id} variant="outline" size="sm" className="w-52 font-normal">
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
