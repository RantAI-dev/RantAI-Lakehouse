"use client"

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

const selectClassName = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

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
        <select
          id={id("kind")}
          className={selectClassName}
          value={form.kind}
          onChange={(e) => onChange({ kind: e.target.value as Kind })}
        >
          {(Object.keys(KINDS) as Kind[]).map((k) => (
            <option key={k} value={k}>
              {KINDS[k].label}
            </option>
          ))}
        </select>
      </div>
      {KINDS[form.kind].column ? (
        <div className="grid gap-1.5">
          <Label htmlFor={id("column")}>Column</Label>
          {columns ? (
            <select
              id={id("column")}
              className={selectClassName}
              value={form.column}
              onChange={(e) => onChange({ column: e.target.value })}
            >
              {columns.map((c) => (
                <option key={c} value={c}>
                  {c}
                </option>
              ))}
            </select>
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
            <select
              id={id("null-unit")}
              className={selectClassName}
              value={form.nullUnit}
              onChange={(e) => onChange({ nullUnit: e.target.value as NullUnit })}
            >
              <option value="percent">% of rows</option>
              <option value="count">rows</option>
            </select>
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
            <select
              id={id("duration-unit")}
              className={selectClassName}
              value={form.durationUnit}
              onChange={(e) => onChange({ durationUnit: e.target.value as DurationUnit })}
            >
              <option value="m">minutes</option>
              <option value="h">hours</option>
              <option value="d">days</option>
            </select>
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
            <select
              id={id("stat")}
              className={selectClassName}
              value={form.stat}
              onChange={(e) => onChange({ stat: e.target.value as Stat })}
            >
              <option value="min">Minimum</option>
              <option value="max">Maximum</option>
              <option value="avg">Average</option>
            </select>
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
