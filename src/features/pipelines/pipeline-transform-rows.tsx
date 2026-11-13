"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { SelectField } from "@/components/patterns/select-field"
import {
  CAST_TYPES,
  FILTER_OPERATORS,
  TRANSFORM_VERBS,
  draftIsComplete,
  emptyDraftFor,
  renderTransformDraft,
  type TransformDraft,
} from "@/lib/transform-draft"
import { cn } from "@/lib/utils"
import { COLUMNS_APPEAR_LATER, ColumnField, ColumnsField } from "./pipeline-pickers"

/**
 * The five operations of a `transform` step: the list of what the step does,
 * and the row that adds one more. BLD-1 part 1 D17 moves these rows out of
 * the wizard's Transform step as they were; part 2 redesigns them, this
 * slice does not. What changed is only where the transforms live (a step's
 * own list, not the pipeline's) and where the columns come from (what the
 * step reads, `pipeline-editor.ts` `columnsRead`).
 *
 * The list is of the grammar strings the API stores. A string the draft
 * parser cannot read still shows, raw, and is kept: dropping it would
 * change the pipeline on a save that did not touch it.
 */
export function TransformRows({
  transforms,
  onChange,
  columns,
  columnsNote,
  failedIndex,
  failedMessage,
  failedMessageId,
  describedBy,
}: {
  transforms: string[]
  onChange: (next: string[]) => void
  /** The columns of what the step reads, or `null` when they are not known. */
  columns: string[] | null
  /** Why the columns are not known, shown when `columns` is `null`. */
  columnsNote?: string
  /** The operation an API refusal named (`steps[i].transforms[j]`, D7). */
  failedIndex: number | null
  failedMessage: string | null
  failedMessageId?: string
  /** The sentence of a problem about the operations as a whole (R1-S10). */
  describedBy?: string
}) {
  const [pendingVerb, setPendingVerb] = React.useState<TransformDraft["verb"]>("dedupe")
  const [pendingDraft, setPendingDraft] = React.useState<TransformDraft>(emptyDraftFor("dedupe"))

  function addTransformDraft() {
    if (!draftIsComplete(pendingDraft)) return
    onChange([...transforms, renderTransformDraft(pendingDraft)])
    setPendingDraft(emptyDraftFor(pendingVerb))
  }

  return (
    <div>
      <p className="mb-2 text-sm font-medium">Operations</p>
      <p className="mb-2 text-xs text-muted-foreground">
        Only the verbs and operators the server&apos;s transform grammar accepts are offered here — the request is
        still re-validated server-side on save.
      </p>
      {columns === null ? (
        <p className="mb-2 text-xs text-muted-foreground">{columnsNote ?? COLUMNS_APPEAR_LATER}</p>
      ) : null}
      {transforms.length > 0 ? (
        <ul className="mb-3 space-y-1" aria-label="Operations of this step" aria-describedby={describedBy}>
          {transforms.map((transform, index) => (
            <li
              key={`${index}:${transform}`}
              className={cn(
                "flex items-center justify-between gap-2 rounded-lg border px-3 py-1.5 text-xs",
                failedIndex === index ? "border-destructive bg-destructive/10" : "border-border"
              )}
              // R1-S10: `aria-invalid` is for a control, not a list item; the
              // row is described by the refusal instead.
              aria-describedby={failedIndex === index ? failedMessageId : undefined}
            >
              <span className="font-mono">{transform}</span>
              <div className="flex items-center gap-2">
                {failedIndex === index && failedMessage ? (
                  <span className="text-destructive">{failedMessage}</span>
                ) : null}
                <button
                  type="button"
                  onClick={() => onChange(transforms.filter((_, i) => i !== index))}
                  className="text-muted-foreground hover:text-foreground"
                  aria-label={`Remove ${transform}`}
                >
                  Remove
                </button>
              </div>
            </li>
          ))}
        </ul>
      ) : (
        <p className="mb-3 text-xs text-muted-foreground" aria-describedby={describedBy}>
          No operations added yet.
        </p>
      )}
      <div className="flex flex-wrap items-end gap-2 rounded-lg border border-dashed border-border p-3">
        <div className="space-y-1.5">
          <Label>Verb</Label>
          <SelectField
            className="w-full"
            aria-label="Verb"
            value={pendingVerb}
            options={TRANSFORM_VERBS.map((verb) => ({ value: verb, label: verb }))}
            onValueChange={(next) => {
              const verb = next as TransformDraft["verb"]
              setPendingVerb(verb)
              setPendingDraft(emptyDraftFor(verb))
            }}
          />
        </div>
        {pendingDraft.verb === "dedupe" ? (
          <ColumnField
            label="Key column"
            value={pendingDraft.key}
            onChange={(key) => setPendingDraft({ verb: "dedupe", key })}
            columns={columns}
            placeholder="order_id"
          />
        ) : null}
        {pendingDraft.verb === "filter" ? (
          <>
            <ColumnField
              label="Column"
              value={pendingDraft.column}
              onChange={(column) => setPendingDraft({ ...pendingDraft, column })}
              columns={columns}
              placeholder="status"
            />
            <div className="space-y-1.5">
              <Label>Operator</Label>
              <SelectField
                className="w-full"
                aria-label="Operator"
                value={pendingDraft.operator}
                options={FILTER_OPERATORS.map((op) => ({ value: op, label: op }))}
                onValueChange={(next) =>
                  setPendingDraft({ ...pendingDraft, operator: next as (typeof FILTER_OPERATORS)[number] })
                }
              />
            </div>
            {/* The value is the literal the column is compared to
                (`filter(column op 'value')`, `renderTransformDraft`), so it
                stays typed: it is not a column of the table. */}
            <div className="space-y-1.5">
              <Label htmlFor="pipeline-filter-value">Value</Label>
              <Input
                id="pipeline-filter-value"
                value={pendingDraft.value}
                onChange={(e) => setPendingDraft({ ...pendingDraft, value: e.target.value })}
                placeholder="active"
              />
            </div>
          </>
        ) : null}
        {pendingDraft.verb === "rename" ? (
          <>
            <ColumnField
              label="From column"
              value={pendingDraft.from}
              onChange={(from) => setPendingDraft({ ...pendingDraft, from })}
              columns={columns}
              placeholder="old_col"
            />
            <ColumnField
              label="To column"
              value={pendingDraft.to}
              onChange={(to) => setPendingDraft({ ...pendingDraft, to })}
              columns={columns}
              placeholder="new_col"
            />
          </>
        ) : null}
        {pendingDraft.verb === "cast" ? (
          <>
            <ColumnField
              label="Column"
              value={pendingDraft.column}
              onChange={(column) => setPendingDraft({ ...pendingDraft, column })}
              columns={columns}
              placeholder="amount"
            />
            <div className="space-y-1.5">
              <Label>Type</Label>
              <SelectField
                className="w-full"
                aria-label="Type"
                value={pendingDraft.type}
                options={CAST_TYPES.map((t) => ({ value: t, label: t }))}
                onValueChange={(next) =>
                  setPendingDraft({ ...pendingDraft, type: next as (typeof CAST_TYPES)[number] })
                }
              />
            </div>
          </>
        ) : null}
        {pendingDraft.verb === "select" ? (
          <ColumnsField
            label="Columns"
            value={pendingDraft.columns}
            onChange={(selected) => setPendingDraft({ verb: "select", columns: selected })}
            columns={columns}
          />
        ) : null}
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={!draftIsComplete(pendingDraft)}
          onClick={addTransformDraft}
        >
          Add transform
        </Button>
      </div>
    </div>
  )
}
