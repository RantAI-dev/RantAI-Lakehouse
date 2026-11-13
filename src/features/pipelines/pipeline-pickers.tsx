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
 * namespaces the page loaded; the column list is the table's own schema. When
 * that list is not available the field goes back to free text with a line
 * saying so — never an empty picker that reads as "there is nothing to pick".
 * A value the pipeline already holds that is not in the list is offered as its
 * own entry, so opening the picker cannot silently replace it.
 */

/** The line under a column field while the source table is not known. */
export const COLUMNS_APPEAR_LATER = "Columns appear here once the table exists"

/** A stored value the list does not have, marked as such rather than dropped. */
function heldOption(value: string, offered: readonly string[], suffix: string) {
  return value !== "" && !offered.includes(value) ? `${value} ${suffix}` : null
}

/**
 * A zone: a `Select` over the namespaces the page loaded, or a text input
 * when there is no list (`namespaces === null`) — with `note` saying why, when
 * the read failed.
 */
export function ZoneField({
  id,
  label,
  value,
  onChange,
  zones,
  note,
  describedBy,
}: {
  id: string
  label: string
  value: string
  onChange: (next: string) => void
  /** The namespaces the lakehouse listed, or `null` when there is no list to pick from. */
  zones: string[] | null
  /** Shown under the field when there is no list: why, and that typing is allowed. */
  note?: string
  /** The id of a sentence about this field, when the page has one (BLD-1 part 1 S3 review R1-S10). */
  describedBy?: string
}) {
  if (zones === null) {
    return (
      <div className="space-y-1.5">
        <Label htmlFor={id}>{label}</Label>
        <Input
          id={id}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          aria-invalid={describedBy ? true : undefined}
          aria-describedby={describedBy}
        />
        {note ? <p className="text-xs text-muted-foreground">{note}</p> : null}
      </div>
    )
  }
  const held = heldOption(value, zones, "(not a namespace this lakehouse lists)")
  const choices = held === null ? zones : [...zones, value]
  // `items` is what the trigger reads its own label from: a value the list
  // does not hold would otherwise show as the bare string, without the mark
  // that says it is not a namespace this lakehouse lists.
  const labels = Object.fromEntries(
    choices.map((zone) => [zone, zone === value && held !== null ? held : zone])
  )
  return (
    <div className="space-y-1.5">
      <Label htmlFor={id}>{label}</Label>
      <Select items={labels} value={value} onValueChange={(next) => onChange(next ?? "")}>
        <SelectTrigger
          id={id}
          aria-label={label}
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
}: {
  id?: string
  label: string
  value: string
  onChange: (next: string) => void
  /** The source table's own columns, or `null` when the table is not known. */
  columns: string[] | null
  placeholder?: string
  className?: string
}) {
  if (columns === null || columns.length === 0) {
    return (
      <div className={className ?? "space-y-1.5"}>
        <Label htmlFor={id}>{label}</Label>
        <Input
          id={id}
          aria-label={label}
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
      <Label htmlFor={id}>{label}</Label>
      <Select items={labels} value={value} onValueChange={(next) => onChange(next ?? "")}>
        <SelectTrigger id={id} aria-label={label} className="w-full">
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
}: {
  label: string
  value: string
  onChange: (next: string) => void
  columns: string[] | null
  className?: string
}) {
  const chosen = React.useMemo(
    () =>
      value
        .split(",")
        .map((column) => column.trim())
        .filter((column) => column.length > 0),
    [value]
  )
  if (columns === null || columns.length === 0) {
    return (
      <div className={className ?? "space-y-1.5"}>
        <Label>{label}</Label>
        <Input
          aria-label={label}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="id,name,amount"
        />
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
      <Label>{label}</Label>
      <Faceted
        multiple
        value={chosen}
        onValueChange={(next) => onChange((next ?? []).join(","))}
      >
        <FacetedTrigger asChild>
          <Button type="button" variant="outline" size="sm" className="w-full justify-between font-normal" aria-label={label}>
            <FacetedBadgeList options={options} placeholder="Pick columns" />
          </Button>
        </FacetedTrigger>
        <FacetedContent className="w-56">
          <FacetedInput placeholder="Search columns" aria-label={`Search ${label}`} />
          <FacetedList>
            <FacetedEmpty>No column of this table matches.</FacetedEmpty>
            <FacetedGroup>
              {options.map((option) => (
                <FacetedItem key={option.value} value={option.value}>
                  <span>{option.label}</span>
                </FacetedItem>
              ))}
            </FacetedGroup>
          </FacetedList>
        </FacetedContent>
      </Faceted>
    </div>
  )
}
