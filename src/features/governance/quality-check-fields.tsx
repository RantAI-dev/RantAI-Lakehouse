"use client"

import { SelectField } from "@/components/patterns/select-field"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { Textarea } from "@/components/ui/textarea"
import {
  KINDS,
  MAX_ACCEPTED_VALUES,
  THRESHOLD_EXAMPLES,
  type DurationUnit,
  type Kind,
  type NullUnit,
  type RuleForm,
  type Stat,
} from "./quality-checks"

const KIND_OPTIONS = (Object.keys(KINDS) as Kind[]).map((k) => ({
  value: k,
  label: KINDS[k].label,
}))

const NULL_UNIT_OPTIONS = [
  { value: "percent", label: "% of rows" },
  { value: "count", label: "rows" },
]

const DURATION_UNIT_OPTIONS = [
  { value: "m", label: "minutes" },
  { value: "h", label: "hours" },
  { value: "d", label: "days" },
]

const STAT_OPTIONS = [
  { value: "min", label: "Minimum" },
  { value: "max", label: "Maximum" },
  { value: "avg", label: "Average" },
]

/**
 * The picker for one quality check: which check, the column it reads, and
 * the numbers it takes. Shared by the asset's Quality tab and the Data
 * Quality page (`DATA-14`, F7), so both offer every check the API runs.
 *
 * `columns` is the table's column names where the table is known; `null`
 * where it is not (the Data Quality page names the table by hand), and the
 * column is typed instead.
 */
export function CheckFields({
  form,
  onChange,
  columns,
  idPrefix = "rule",
  note,
}: {
  form: RuleForm
  onChange: (patch: Partial<RuleForm>) => void
  columns: string[] | null
  idPrefix?: string
  /** Shown under the fields of the check being built, e.g. a Bronze caveat. */
  note?: (kind: Kind) => React.ReactNode
}) {
  const id = (name: string) => `${idPrefix}-${name}`
  const bounds = (
    <div className="grid gap-3 sm:grid-cols-2">
      <div className="grid gap-1.5">
        <Label htmlFor={id("min")}>Minimum</Label>
        <Input
          id={id("min")}
          inputMode="decimal"
          value={form.min}
          onChange={(e) => onChange({ min: e.target.value })}
          placeholder="no lower bound"
        />
      </div>
      <div className="grid gap-1.5">
        <Label htmlFor={id("max")}>Maximum</Label>
        <Input
          id={id("max")}
          inputMode="decimal"
          value={form.max}
          onChange={(e) => onChange({ max: e.target.value })}
          placeholder="no upper bound"
        />
      </div>
    </div>
  )

  return (
    <>
      <div className="grid gap-1.5">
        <Label htmlFor={id("kind")}>Check</Label>
        <SelectField
          id={id("kind")}
          className="w-full"
          value={form.kind}
          options={KIND_OPTIONS}
          onValueChange={(next) => onChange({ kind: next as Kind })}
        />
      </div>
      {KINDS[form.kind].column ? (
        <div className="grid gap-1.5">
          <Label htmlFor={id("column")}>Column</Label>
          {columns ? (
            <SelectField
              id={id("column")}
              className="w-full"
              /* WEB-9 S1 review R1-S1 (D2a): a new rule's column starts as
                 `""`, so without this the trigger is an empty box that says
                 nothing about what to pick. */
              placeholder="Pick a column"
              value={form.column}
              options={[
                /* WEB-8 D8: a column the rule already names that this table
                   does not have is still shown, so opening the picker cannot
                   replace it with whatever sorts first. */
                ...(form.column !== "" && !columns.includes(form.column)
                  ? [{ value: form.column, label: `${form.column} (not in this table)` }]
                  : []),
                ...columns.map((c) => ({ value: c, label: c })),
              ]}
              onValueChange={(next) => onChange({ column: next })}
            />
          ) : (
            <Input
              id={id("column")}
              className="font-mono"
              value={form.column}
              onChange={(e) => onChange({ column: e.target.value })}
              placeholder="email"
            />
          )}
        </div>
      ) : null}
      {note?.(form.kind)}
      {form.kind === "rows" ? (
        <div className="grid gap-1.5">
          <Label htmlFor={id("rows")}>Minimum rows</Label>
          <Input
            id={id("rows")}
            inputMode="numeric"
            value={form.rows}
            onChange={(e) => onChange({ rows: e.target.value })}
          />
        </div>
      ) : null}
      {form.kind === "not-null" ? (
        <div className="grid gap-1.5">
          <Label htmlFor={id("percent")}>Rows that must have a value (%)</Label>
          <Input
            id={id("percent")}
            inputMode="decimal"
            value={form.percent}
            onChange={(e) => onChange({ percent: e.target.value })}
          />
        </div>
      ) : null}
      {form.kind === "nulls" ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="grid gap-1.5">
            <Label htmlFor={id("amount")}>At most</Label>
            <Input
              id={id("amount")}
              inputMode="decimal"
              value={form.amount}
              onChange={(e) => onChange({ amount: e.target.value })}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor={id("null-unit")}>Counted as</Label>
            <SelectField
              id={id("null-unit")}
              className="w-full"
              value={form.nullUnit}
              options={NULL_UNIT_OPTIONS}
              onValueChange={(next) => onChange({ nullUnit: next as NullUnit })}
            />
          </div>
        </div>
      ) : null}
      {form.kind === "duplicates" || form.kind === "distinct" ? (
        <div className="grid gap-1.5">
          <Label htmlFor={id("amount")}>
            {form.kind === "duplicates" ? "Most repeated values allowed" : "Fewest different values"}
          </Label>
          <Input
            id={id("amount")}
            inputMode="numeric"
            value={form.amount}
            onChange={(e) => onChange({ amount: e.target.value })}
          />
        </div>
      ) : null}
      {form.kind === "fresh" ? (
        <div className="grid gap-3 sm:grid-cols-2">
          <div className="grid gap-1.5">
            <Label htmlFor={id("amount")}>No older than</Label>
            <Input
              id={id("amount")}
              inputMode="numeric"
              value={form.amount}
              onChange={(e) => onChange({ amount: e.target.value })}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor={id("duration-unit")}>Unit</Label>
            <SelectField
              id={id("duration-unit")}
              className="w-full"
              value={form.durationUnit}
              options={DURATION_UNIT_OPTIONS}
              onValueChange={(next) => onChange({ durationUnit: next as DurationUnit })}
            />
          </div>
        </div>
      ) : null}
      {form.kind === "accepted" ? (
        <div className="grid gap-1.5">
          <Label htmlFor={id("values")}>Accepted values, one per line</Label>
          <Textarea
            id={id("values")}
            rows={4}
            value={form.values}
            onChange={(e) => onChange({ values: e.target.value })}
          />
          <p className="text-xs text-muted-foreground">
            Up to {MAX_ACCEPTED_VALUES} values. Rows where the column is empty are not counted as
            outside the list.
          </p>
        </div>
      ) : null}
      {form.kind === "range" ? bounds : null}
      {form.kind === "stat" ? (
        <>
          <div className="grid gap-1.5">
            <Label htmlFor={id("stat")}>Statistic</Label>
            <SelectField
              id={id("stat")}
              className="w-full"
              value={form.stat}
              options={STAT_OPTIONS}
              onValueChange={(next) => onChange({ stat: next as Stat })}
            />
          </div>
          {bounds}
        </>
      ) : null}
    </>
  )
}

/** The forms a typed threshold can take, for the help under its field. */
export function ThresholdForms() {
  return (
    <>
      {THRESHOLD_EXAMPLES.map((example, i) => (
        <span key={example}>
          <span className="font-mono">{example}</span>
          {i < THRESHOLD_EXAMPLES.length - 1 ? ", " : ""}
        </span>
      ))}
    </>
  )
}
