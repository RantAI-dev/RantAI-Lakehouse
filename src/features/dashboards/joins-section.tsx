"use client";

import * as React from "react";
import { Plus, TriangleAlert, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import {
  JOIN_OPS, JOIN_TYPES, MAX_JOINS, MAX_PAIRS, displayColumn, emptyJoin, leftChoices, multiplyWarning,
  type JoinDraft, type JoinPairDef,
} from "@/lib/chart-joins";

/**
 * The chart builder's "Joins" section (`BI-7`): add up to four tables to the
 * chart's own, choose how each is joined, and match columns, with the pairs
 * the server suggests applied when a table is picked. Plain controls over the
 * builder's state; the builder loads the tables' columns and suggestions and
 * owns the draft and the payload.
 */

/** What the builder knows about a table picked as a join: its columns and its sorting key. */
export type JoinTableInfo = { columns: readonly string[]; rightKey: readonly string[] };

export function JoinsSection({
  baseMart, baseColumns, marts, joins, onChange, onPickTable, infoOf, disabled,
}: {
  /** The chart's own table. */
  readonly baseMart: string;
  /** Its columns (without calculated fields). */
  readonly baseColumns: readonly string[];
  /** The tables a join may name. */
  readonly marts: readonly string[];
  readonly joins: readonly JoinDraft[];
  readonly onChange: (next: JoinDraft[]) => void;
  /** A table was picked for join `index`: the builder loads its columns and suggestions and sets the pairs. */
  readonly onPickTable: (index: number, mart: string) => void;
  readonly infoOf: (mart: string) => JoinTableInfo | undefined;
  readonly disabled: boolean;
}) {
  const columnsOf = (mart: string) => infoOf(mart)?.columns;
  const setJoin = (i: number, patch: Partial<JoinDraft>) => onChange(joins.map((j, k) => (k === i ? { ...j, ...patch } : j)));
  const setPair = (i: number, p: number, patch: Partial<JoinPairDef>) =>
    setJoin(i, { on: joins[i].on.map((pair, k) => (k === p ? { ...pair, ...patch } : pair)) });
  return (
    <div className="grid gap-2" data-testid="joins-section">
      <Label>Joins</Label>
      {joins.map((j, i) => {
        const left = leftChoices(baseColumns, joins, i, columnsOf);
        const right = infoOf(j.mart)?.columns ?? [];
        const warning = multiplyWarning(infoOf(j.mart)?.rightKey ?? [], j);
        return (
          <div key={i} className="grid gap-2 rounded-md border border-border p-2">
            <div className="flex items-center gap-1.5">
              <Select value={j.mart} onValueChange={(v) => v && onPickTable(i, v)} disabled={disabled}>
                <SelectTrigger className="w-full min-w-0 flex-1" aria-label={`Join ${i + 1} table`}><SelectValue placeholder="pick a table" /></SelectTrigger>
                <SelectContent>{marts.filter((m) => m !== "").map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
              </Select>
              <Select value={j.type} onValueChange={(v) => v && setJoin(i, { type: v as JoinDraft["type"] })} disabled={disabled} items={Object.fromEntries(JOIN_TYPES.map((t) => [t.value, t.label.split(" — ")[0]]))}>
                <SelectTrigger className="w-28 shrink-0" aria-label={`Join ${i + 1} type`}><SelectValue /></SelectTrigger>
                <SelectContent>{JOIN_TYPES.map((t) => <SelectItem key={t.value} value={t.value}>{t.label}</SelectItem>)}</SelectContent>
              </Select>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove join ${i + 1}`} disabled={disabled} onClick={() => onChange(joins.filter((_, k) => k !== i))}><X /></Button>
            </div>
            {j.mart ? (
              <div className="grid gap-1.5">
                {j.on.map((pair, p) => (
                  <div key={p} className="flex items-center gap-1.5">
                    <Select value={pair.left} onValueChange={(v) => setPair(i, p, { left: v ?? "" })} disabled={disabled}>
                      <SelectTrigger className="w-full min-w-0 flex-1" aria-label={`Join ${i + 1} pair ${p + 1} left`}><SelectValue placeholder="column" /></SelectTrigger>
                      <SelectContent>{left.map((c) => <SelectItem key={c} value={c}>{displayColumn(c)}</SelectItem>)}</SelectContent>
                    </Select>
                    <Select value={pair.op} onValueChange={(v) => setPair(i, p, { op: v ?? "=" })} disabled={disabled} items={Object.fromEntries(JOIN_OPS.map((o) => [o.value, o.label]))}>
                      <SelectTrigger className="w-32 shrink-0" aria-label={`Join ${i + 1} pair ${p + 1} comparison`}><SelectValue /></SelectTrigger>
                      <SelectContent>{JOIN_OPS.map((o) => <SelectItem key={o.value} value={o.value}>{o.label}</SelectItem>)}</SelectContent>
                    </Select>
                    <Select value={pair.right} onValueChange={(v) => setPair(i, p, { right: v ?? "" })} disabled={disabled}>
                      <SelectTrigger className="w-full min-w-0 flex-1" aria-label={`Join ${i + 1} pair ${p + 1} right`}><SelectValue placeholder="column" /></SelectTrigger>
                      <SelectContent>{right.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
                    </Select>
                    <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove join ${i + 1} pair ${p + 1}`} disabled={disabled || j.on.length <= 1} onClick={() => setJoin(i, { on: j.on.filter((_, k) => k !== p) })}><X /></Button>
                  </div>
                ))}
                {j.on.length < MAX_PAIRS ? (
                  <div>
                    <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={disabled} onClick={() => setJoin(i, { on: [...j.on, { left: "", op: "=", right: "" }] })}>
                      <Plus className="size-3" aria-hidden /> Add a pair
                    </Button>
                  </div>
                ) : null}
                {warning ? (
                  <p className="flex items-start gap-1.5 text-xs text-amber-600 dark:text-amber-400" role="status">
                    <TriangleAlert className="mt-0.5 size-3.5 shrink-0" aria-hidden /> {warning}
                  </p>
                ) : null}
              </div>
            ) : null}
          </div>
        );
      })}
      {joins.length < MAX_JOINS ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={disabled || !baseMart} onClick={() => onChange([...joins, emptyJoin()])}>
            <Plus className="size-3" aria-hidden /> Join a table
          </Button>
        </div>
      ) : null}
    </div>
  );
}
