"use client"

import * as React from "react"
import { SelectField } from "@/components/patterns/select-field"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { formatNumber } from "@/lib/format"
import { COLUMN_TYPES, DATE_FORMATS, columnTypeLabel, columnNameProblem, keepsNanoseconds } from "@/lib/uploads"
import type { UploadBadValues, UploadColumnType, UploadDateFormat } from "@/services/contracts/uploads"
import type { ColumnChoices } from "./use-column-choices"
import { Refusal, UploadNotice } from "./upload-parts"

/** The types a column's select offers: the six, and "Floating point (from the file)" for a column that has it. */
function typeOptions(type: UploadColumnType, guessed: UploadColumnType) {
  const floats = type === "float" || guessed === "float"
  return floats ? [...COLUMN_TYPES, { value: "float" as const, label: columnTypeLabel("float") }] : COLUMN_TYPES
}

/**
 * Under the preview: each column's name (pre-filled with what loading names it),
 * its type (the guess, the file's own, or the table's) and, for a date or a
 * timestamp, how it is read. A date that could be read either way is a question
 * ("Day first or month first?") and Load stays off until it is answered
 * (`DATA-18` decision 4). When the file will be added to a table, the names and
 * types are the table's and cannot be changed (decision 9).
 */
export function UploadColumnsEditor({
  choices,
  sampledRows,
  fileTypes,
}: {
  readonly choices: ColumnChoices
  /** How many data rows the guess read. */
  readonly sampledRows: number
  /**
   * The file's own type label per column, for a Parquet file only (`DATA-53`): what a
   * column starts with when the file declares it, so the Check step can say what the
   * table keeps of it.
   */
  readonly fileTypes?: readonly (string | undefined)[]
}) {
  const { columns, fixed } = choices
  return (
    <section className="space-y-3" aria-label="Columns">
      <div className="space-y-1">
        <h3 className="text-sm font-semibold">Columns</h3>
        <p className="text-xs text-muted-foreground">
          {fixed
            ? "The names and types are the table's: adding to a table keeps them, so they cannot be changed here."
            : `Types guessed from the first ${formatNumber(sampledRows)} ${sampledRows === 1 ? "row" : "rows"}. Change a name or a type before loading.`}
        </p>
      </div>
      <div className="overflow-x-auto rounded-lg border border-border">
        <table className="w-full min-w-[40rem] text-sm">
          <thead>
            <tr className="border-b border-border bg-muted/40 text-left text-xs text-muted-foreground">
              <th className="px-3 py-2 font-medium">In the file</th>
              <th className="px-3 py-2 font-medium">Name</th>
              <th className="px-3 py-2 font-medium">Type</th>
              <th className="px-3 py-2 font-medium">Read as</th>
            </tr>
          </thead>
          <tbody>
            {columns.map((column, index) => {
              const nameProblem = columnNameProblem(column.name)
              const dated = column.type === "date" || column.type === "timestamp"
              const asking = dated && column.dateFormat === undefined
              return (
                <tr key={index} className="border-b border-border last:border-0 align-top">
                  <td className="max-w-48 truncate px-3 py-2 text-muted-foreground" title={column.source}>
                    {column.source || "(empty)"}
                  </td>
                  <td className="px-3 py-2">
                    <Input
                      aria-label={`Name of column ${index + 1}`}
                      value={column.name}
                      disabled={fixed}
                      aria-invalid={nameProblem !== null}
                      onChange={(e) => choices.setName(index, e.target.value)}
                      className="h-8"
                    />
                  </td>
                  <td className="px-3 py-2">
                    {/* WEB-9 D3: the console's one dropdown, same types and
                        order; a disabled row keeps the control disabled. */}
                    <SelectField
                      aria-label={`Type of ${column.name || `column ${index + 1}`}`}
                      className="w-full"
                      value={column.type}
                      disabled={fixed}
                      options={typeOptions(column.type, column.guessedType).map((t) => ({
                        value: t.value,
                        label: t.label,
                      }))}
                      onValueChange={(next) => choices.setType(index, next as UploadColumnType)}
                    />
                    {column.type !== column.guessedType && !fixed ? (
                      <p className="mt-1 text-xs text-muted-foreground">Guessed: {columnTypeLabel(column.guessedType)}</p>
                    ) : null}
                    {/* `DATA-53` (QA `LH-DATA-9-32`): what the table keeps of the type, which
                        a person can only see here. Both notes show with the columns fixed too,
                        because a table's types are what the load will write. */}
                    {column.type === "decimal" ? (
                      <p className="mt-1 text-xs text-muted-foreground">
                        Stored exactly, with up to 38 digits, 9 of them after the point.
                      </p>
                    ) : null}
                    {column.type === "timestamp" && keepsNanoseconds(fileTypes?.[index]) ? (
                      <p className="mt-1 text-xs text-muted-foreground">
                        The file keeps nanoseconds. The table keeps microseconds, so digits after the sixth are cut.
                      </p>
                    ) : null}
                  </td>
                  <td className="space-y-1.5 px-3 py-2">
                    {dated ? (
                      <>
                        <Label className="sr-only" htmlFor={`upload-date-${index}`}>
                          How the dates of {column.name || `column ${index + 1}`} are read
                        </Label>
                        {/* WEB-9 D2/D3: "Day first or month first?" was a
                            disabled prompt option, so it is the placeholder;
                            the three readings stay the options. */}
                        <SelectField
                          id={`upload-date-${index}`}
                          className="w-full"
                          value={column.dateFormat ?? ""}
                          placeholder="Day first or month first?"
                          disabled={fixed && column.dateFormat !== undefined}
                          aria-invalid={asking}
                          options={DATE_FORMATS.map((f) => ({ value: f.value, label: f.label }))}
                          onValueChange={(next) => choices.setDateFormat(index, next as UploadDateFormat)}
                        />
                        {column.type === "timestamp" ? (
                          <SelectField
                            aria-label={`Time zone of ${column.name || `column ${index + 1}`}`}
                            className="w-full"
                            value={column.timezone ? "zone" : "none"}
                            disabled={fixed}
                            options={[
                              { value: "none", label: "No time zone in the values" },
                              { value: "zone", label: "A time zone in every value (stored in UTC)" },
                            ]}
                            onValueChange={(next) => choices.setTimezone(index, next === "zone")}
                          />
                        ) : null}
                      </>
                    ) : null}
                  </td>
                </tr>
              )
            })}
          </tbody>
        </table>
      </div>
      {choices.problems.length > 0 ? (
        <ul role="alert" className="space-y-1 rounded-lg border border-destructive/30 bg-destructive/5 px-3 py-2 text-sm text-destructive">
          {choices.problems.map((problem) => (
            <li key={problem}>{problem}</li>
          ))}
        </ul>
      ) : null}
    </section>
  )
}

/** The values that do not fit, as a list: the count per column and the rows. */
export function BadValuesList({ bad }: { readonly bad: UploadBadValues }) {
  return (
    <div className="space-y-2">
      <ul className="text-sm">
        {bad.byColumn.map((entry) => (
          <li key={entry.column}>
            <span className="font-medium">{entry.column}</span>: {formatNumber(entry.count)}
          </li>
        ))}
      </ul>
      <div className="max-h-64 overflow-auto rounded-lg border border-border">
        <table className="w-full text-sm">
          <thead>
            <tr className="border-b border-border bg-muted/40 text-left text-xs text-muted-foreground">
              <th className="px-3 py-1.5 font-medium">Row</th>
              <th className="px-3 py-1.5 font-medium">Column</th>
              <th className="px-3 py-1.5 font-medium">Value</th>
            </tr>
          </thead>
          <tbody>
            {bad.rows.map((cell, index) => (
              <tr key={index} className="border-b border-border last:border-0">
                <td className="px-3 py-1.5 tabular-nums">{formatNumber(cell.row)}</td>
                <td className="px-3 py-1.5">{cell.column}</td>
                <td className="max-w-80 truncate px-3 py-1.5 font-mono text-[13px]" title={cell.value}>
                  {cell.value}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {bad.count > bad.rows.length ? (
        <p className="text-xs text-muted-foreground">
          The first {bad.rows.length} of {formatNumber(bad.count)} are listed.
        </p>
      ) : null}
    </div>
  )
}

/**
 * The "N values do not fit" heading and the list under it. The Check step and the Review
 * step show the same one (QA LH-DATA-18-08: the mismatch is listed before loading).
 */
export function BadValuesReport({ bad }: { readonly bad: UploadBadValues & { sampledRows?: number } }) {
  return (
    <>
      <h3 className="text-sm font-semibold text-destructive">
        {formatNumber(bad.count)} {bad.count === 1 ? "value does" : "values do"} not fit
        {bad.sampledRows ? ` in the first ${formatNumber(bad.sampledRows)} rows` : ""}
      </h3>
      <BadValuesList bad={bad} />
    </>
  )
}

/**
 * Before Load: the check of the column list against the sample, and the choice of
 * what to do with a value that does not fit (`DATA-18` decision 6). **Stop** is
 * pre-selected. Nothing is shown while every value fits, except that they do.
 */
export function UploadBadValuesPanel({ choices, sampledRows }: { readonly choices: ColumnChoices; readonly sampledRows: number }) {
  const { check } = choices
  // Nothing to ask while every column is text: nothing can fail to fit.
  if (choices.columns.every((column) => column.type === "text")) return null
  const bad = check.status === "done" && check.result.count > 0 ? check.result : null
  return (
    <section
      className={
        bad
          ? "space-y-3 rounded-lg border border-destructive/30 bg-destructive/5 p-4"
          : "space-y-3 rounded-lg border border-border p-4"
      }
      aria-label="Values that do not fit"
    >
      {check.status === "checking" ? (
        <p role="status" className="text-xs text-muted-foreground">
          Checking the values against these types…
        </p>
      ) : null}
      {check.status === "error" ? <Refusal error={check.error} /> : null}
      {check.status === "done" && bad === null ? (
        <p role="status" className="text-sm text-muted-foreground">
          All {formatNumber(check.result.sampledRows || sampledRows)} sampled rows fit these types.
        </p>
      ) : null}
      {bad ? <BadValuesReport bad={bad} /> : null}
      <fieldset className="space-y-1.5">
        <legend className="text-sm font-medium">What should happen to a value that does not fit?</legend>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="radio"
            name="upload-on-bad-value"
            checked={choices.onBadValue === "stop"}
            onChange={() => choices.setOnBadValue("stop")}
          />
          Stop the load. Nothing is written, and the count and the first 100 are shown.
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="radio"
            name="upload-on-bad-value"
            checked={choices.onBadValue === "null"}
            onChange={() => choices.setOnBadValue("null")}
          />
          Load them as empty, and show the same report.
        </label>
      </fieldset>
      <UploadNotice>
        The guess and this check read the first rows. A value further down is caught by the load, which does what is chosen here.
      </UploadNotice>
    </section>
  )
}
