"use client"

import * as React from "react"

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
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"

/**
 * The three controls that offer a value the server already knows, instead of
 * a free-text field (`WEB-8` D8, spec `web-8.md`): the source and target
 * zones, and the source table's columns.
 *
 * Their rule is the same in each case and is the reason they exist: a picker
 * never invents an option and never hides a stored one. The zone list is the
 * pipeline's medallion zones (`pipeline-zones.ts`), shown with the console's
 * layer labels; the column list is the table's own schema. When a list is not
 * available the field goes back to free text with a line saying so — never an
 * empty picker that reads as "there is nothing to pick". A value the pipeline
 * already holds that is not in the list is offered as its own entry, marked as
 * legacy, so opening the picker cannot silently replace it.
 */

/** The line under a column field while the source table is not known. */
export const COLUMNS_APPEAR_LATER = "Columns appear here once the table exists"

/** A stored value the list does not have, marked as such rather than dropped. */
function heldOption(value: string, offered: readonly string[], suffix: string) {
  return value !== "" && !offered.includes(value) ? `${value} ${suffix}` : null
}

/**
 * A zone: a `Select` over the medallion zones it is given, or a text input
 * when there is no list (`zones === null`) — with `note` saying why. `value`
 * is always the exact stored string (`bronze`/`silver`/`serving`, or a legacy
 * one); `zoneLabels` only changes what an option reads.
 */
export function ZoneField({
  id,
  label,
  value,
  onChange,
  zones,
  zoneLabels,
  note,
  describedBy,
  required,
}: {
  id: string
  label: string
  value: string
  onChange: (next: string) => void
  /** The zone values to offer, or `null` when there is no list to pick from. */
  zones: readonly string[] | null
  /** Human label for each value (e.g. `serving` -> "Gold"); a value without one reads as itself. */
  zoneLabels?: Readonly<Record<string, string>>
  /** Shown under the field when there is no list: why, and that typing is allowed. */
  note?: string
  /** The id of a sentence about this field, when the page has one (BLD-1 part 1 S3 review R1-S10). */
  describedBy?: string
  /** UX-1-R8: a save refuses a blank zone, so this input is starred and required. */
  required?: boolean
}) {
  if (zones === null) {
    return (
      <div className="space-y-1.5">
        <Label htmlFor={id} required={required}>{label}</Label>
        <Input
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          aria-required={required}
          aria-invalid={describedBy ? true : undefined}
          aria-describedby={describedBy}
        />
        {note ? <p className="text-xs text-muted-foreground">{note}</p> : null}
      </div>
    )
  }
  // A stored value outside the medallion zones (an older `gold`, or a custom
  // zone) stays selectable and marked, so loading an old pipeline cannot
  // silently blank it. The label is the only thing that is friendly: the
  // stored `value` sent on save is always the exact one from the option.
  const held = heldOption(value, zones, "(legacy zone, not offered here)")
  const choices = held === null ? zones : [...zones, value]
  // `items` is what the trigger reads its own label from: a value the list
  // does not hold would otherwise show as the bare string, without the mark
  // that says it is not one of this pipeline's zones.
  const labels = Object.fromEntries(
    choices.map((zone) => [zone, zone === value && held !== null ? held : (zoneLabels?.[zone] ?? zone)])
  )
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id} required={required}>{label}</Label>
      <Select items={labels} value={value} onValueChange={(next) => onChange(next ?? "")}>
        <SelectTrigger
          id={id}
          aria-label={label}
          aria-required={required}
          aria-invalid={describedBy ? true : undefined}
          aria-describedby={describedBy}
          className="w-full"
        >
          <SelectValue placeholder="Pick a namespace" />
        </SelectTrigger>
        <SelectContent>
          {choices.map((zone) => (
            <SelectItem key={zone} value={zone}>
              {labels[zone]}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

/**
 * One column: a `Select` over the source table's schema when the table is
 * known, and the same text input as before when it is not (`columns ===
 * null`). `label` is also the control's accessible name: a call site may
 * pass no `id`, and then the `Label` names no control (BLD-1 part 1 S3
 * review R1-S8).
 */
export function ColumnField({
  id,
  label,
  value,
  onChange,
  columns,
  placeholder,
  className,
  required,
}: {
  id?: string
  label: string
  value: string
  onChange: (next: string) => void
  /** The source table's own columns, or `null` when the table is not known. */
  columns: string[] | null
  placeholder?: string
  className?: string
  /** UX-1-R8: an operation's column argument that `draftIsComplete` refuses blank. */
  required?: boolean
}) {
  if (columns === null || columns.length === 0) {
    return (
      <div className={className ?? "space-y-1.5"}>
        <Label htmlFor={id} required={required}>{label}</Label>
        <Input
          id={id}
          aria-label={label}
          aria-required={required}
          value={value}
          placeholder={placeholder}
          onChange={(e) => onChange(e.target.value)}
        />
      </div>
    )
  }
  const held = heldOption(value, columns, "(not in this table)")
  const choices = held === null ? columns : [...columns, value]
  const labels = Object.fromEntries(
    choices.map((column) => [column, column === value && held !== null ? held : column])
  )
  return (
    <div className={className ?? "space-y-1.5"}>
      <Label htmlFor={id} required={required}>{label}</Label>
      <Select items={labels} value={value} onValueChange={(next) => onChange(next ?? "")}>
        <SelectTrigger id={id} aria-label={label} aria-required={required} className="w-full">
          <SelectValue placeholder={placeholder ?? "Pick a column"} />
        </SelectTrigger>
        <SelectContent>
          {choices.map((column) => (
            <SelectItem key={column} value={column}>
              {labels[column]}
            </SelectItem>
          ))}
        </SelectContent>
      </Select>
    </div>
  )
}

/**
 * Several columns (`select(a,b,c)`): the `Faceted` multiple-select the rest of
 * the console uses, over the same schema. The draft keeps its comma-separated
 * string, so the transform grammar and its tests are unchanged. A column the
 * draft already names that the table does not have is listed last, so it is
 * visible and can be taken out.
 */
export function ColumnsField({
  label,
  value,
  onChange,
  columns,
  className,
  required,
}: {
  label: string
  value: string
  onChange: (next: string) => void
  columns: string[] | null
  className?: string
  /** UX-1-R8: `select(...)` refuses a blank column list; the Faceted trigger is
   *  a button, which has no supported `aria-required`, so the star is paired
   *  with a linked sentence (the group pattern, not a false button attribute). */
  required?: boolean
}) {
  const needId = React.useId()
  const chosen = React.useMemo(
    () =>
      value
        .split(",")
        .map((column) => column.trim())
        .filter((column) => column.length > 0),
    [value]
  )
  const needed = required ? (
    <p id={needId} className="text-xs text-muted-foreground">Select at least one column.</p>
  ) : null
  if (columns === null || columns.length === 0) {
    return (
      <div className={className ?? "space-y-1.5"}>
        <Label required={required}>{label}</Label>
        <Input
          aria-label={label}
          aria-required={required}
          aria-describedby={required ? needId : undefined}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="id,name,amount"
        />
        {needed}
      </div>
    )
  }
  const missing = chosen.filter((column) => !columns.includes(column))
  const options = [...columns, ...missing].map((column) => ({
    value: column,
    label: missing.includes(column) ? `${column} (not in this table)` : column,
  }))
  return (
    <div className={className ?? "space-y-1.5"}>
      <Label required={required}>{label}</Label>
      <Faceted
        multiple
        value={chosen}
        onValueChange={(next) => onChange((next ?? []).join(","))}
      >
        {/* WEB-13: bound the selected chips and the trigger. The comma list is
            also the trigger's title, and the option row wraps the full name.
            WEB-13 review S1: two chips may wrap, so the trigger must grow with
            its content rather than letting a second row escape its border. */}
        <FacetedTrigger asChild>
          <Button type="button" variant="outline" size="sm" className="h-auto min-h-7 w-full min-w-0 justify-between py-1 font-normal" aria-label={label} aria-describedby={required ? needId : undefined} title={value === "" ? undefined : value}>
            <FacetedBadgeList
              options={options}
              placeholder="Pick columns"
              className="min-w-0 flex-1"
              badgeClassName="max-w-full [&>span]:min-w-0"
            />
          </Button>
        </FacetedTrigger>
        <FacetedContent className="w-56">
          <FacetedInput placeholder="Search columns" aria-label={`Search ${label}`} />
          <FacetedList>
            <FacetedEmpty>No column of this table matches.</FacetedEmpty>
            <FacetedGroup>
              {options.map((option) => (
                <FacetedItem key={option.value} value={option.value}>
                  {/* WEB-13: a long column name wraps inside the option row. */}
                  <span className="min-w-0 break-all leading-snug">{option.label}</span>
                </FacetedItem>
              ))}
            </FacetedGroup>
          </FacetedList>
        </FacetedContent>
      </Faceted>
      {needed}
    </div>
  )
}
