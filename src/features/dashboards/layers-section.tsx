"use client";

import * as React from "react";
import { ArrowDown, ArrowUp, Plus, TriangleAlert, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectGroup, SelectItem, SelectLabel, SelectTrigger, SelectValue } from "@/components/ui/select";
import { MAX_LAYERS, emptyLayer, matchingDimensions, noMatchNote, type LayerAxis, type LayerDraft, type LayerDraw } from "@/lib/chart-layers";
import { decodeSourceChoice, encodeSourceChoice, fieldsQuery } from "@/lib/chart-source";
import { displayColumn } from "@/lib/chart-joins";
import { moveItem } from "@/lib/table-draft";
import { columnKindOfType, type ColumnKind } from "@/lib/time-grain";
import { apiFetch } from "@/services/http";
import { ChartFilters, MeasureRows, type ColumnChoice } from "./chart-shape-section";

/**
 * The chart builder's "More questions" section (`BI-13`): up to four more
 * questions on a bar, line, area or combo chart, each from a table or SQL
 * source of its own, lined up on the chart's dimension. Plain controls over
 * the builder's state; the builder owns the drafts and the payload, and this
 * loads the columns of each question's source.
 */

const DRAW_ITEMS: Record<string, string> = { bar: "Bars", line: "Line" };
const AXIS_ITEMS: Record<string, string> = { left: "Left axis", right: "Right axis" };
const OWN_DEFAULT = "__default__";
const AGGREGATES = ["sum", "avg", "max", "min", "count"];

/** The columns of one question's source, as the builder's fields route describes them. */
type SourceFields = { columns: ColumnChoice[]; measures: string[] };

type Props = {
  readonly layers: readonly LayerDraft[];
  readonly onChange: (next: LayerDraft[]) => void;
  /** The chart's own dimension kind, and whether it is cut by a time grain. */
  readonly baseKind: ColumnKind | undefined;
  readonly grained: boolean;
  readonly marts: readonly string[];
  readonly sqlSources: readonly { id: string; title: string }[];
  /** How the chart's own series is drawn and on which axis ("" = the kind's default). */
  readonly ownDraw: string;
  readonly ownAxis: string;
  readonly onOwnChange: (patch: { draw?: string; axis?: string }) => void;
  /** Why questions cannot be added now (a breakdown, "Other", steps or bins on the chart), in the builder's words. */
  readonly blocked?: string;
  readonly disabled: boolean;
};

export function LayersSection({ layers, onChange, baseKind, grained, marts, sqlSources, ownDraw, ownAxis, onOwnChange, blocked, disabled }: Props) {
  const [loaded, setLoaded] = React.useState<Record<string, SourceFields>>({});
  const keyOf = (l: LayerDraft) => `${l.source}|${JSON.stringify(l.joins)}`;
  const wanted = layers.map(keyOf).join("\n");
  React.useEffect(() => {
    for (const layer of layers) {
      const key = keyOf(layer);
      const choice = decodeSourceChoice(layer.source);
      if (!choice || loaded[key]) continue;
      const joins = choice.kind === "mart" && layer.joins.length ? `&joins=${encodeURIComponent(JSON.stringify(layer.joins))}` : "";
      void apiFetch(`/api/dashboard/fields?${fieldsQuery(choice)}${joins}`)
        .then((r) => r.json())
        .then((j: { columns?: { name: string; type: string }[]; measures?: string[]; calculated?: { name: string; type: string; level?: string }[] }) => {
          const columns: ColumnChoice[] = (j.columns ?? []).map((c) => ({ name: c.name, kind: columnKindOfType(c.type) }));
          const calculated = j.calculated ?? [];
          for (const f of calculated) columns.push({ name: f.name, kind: f.type === "number" ? "number" : f.type === "date" ? "date" : f.type === "datetime" ? "datetime" : "text" });
          const measures = [...(j.measures ?? []), ...calculated.filter((f) => f.type === "number" || f.level === "aggregate" || f.level === "table").map((f) => f.name)];
          setLoaded((prev) => ({ ...prev, [key]: { columns, measures: [...new Set(measures)] } }));
        })
        .catch(() => setLoaded((prev) => ({ ...prev, [key]: { columns: [], measures: [] } })));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `wanted` names the sources whose columns are needed
  }, [wanted]);

  // A dimension that no longer lines up (the chart's changed, or the question's source did) is dropped
  // rather than sent: the server would refuse it. Waits for the columns, and for a source that answered.
  const fitKey = layers.map((l) => `${keyOf(l)}>${l.dimension}`).join("\n") + `|${baseKind}|${grained}|${Object.keys(loaded).length}`;
  React.useEffect(() => {
    let changed = false;
    const next = layers.map((l) => {
      const fields = loaded[keyOf(l)];
      if (!l.dimension || !fields?.columns.length) return l;
      if (matchingDimensions(baseKind, grained, fields.columns).some((c) => c.name === l.dimension)) return l;
      changed = true;
      return { ...l, dimension: "" };
    });
    if (changed) onChange(next);
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `fitKey` names everything this reads
  }, [fitKey]);

  const set = (i: number, patch: Partial<LayerDraft>) => onChange(layers.map((l, k) => (k === i ? { ...l, ...patch } : l)));
  const sourceItems: Record<string, string> = {
    ...Object.fromEntries(marts.map((m) => [encodeSourceChoice({ kind: "mart", name: m }), m])),
    ...Object.fromEntries(sqlSources.map((s) => [encodeSourceChoice({ kind: "sql", id: s.id }), `SQL · ${s.title}`])),
  };
  return (
    <div className="grid gap-2" data-testid="layers-section">
      <Label>More questions</Label>
      {blocked ? (
        <p className="flex items-start gap-1.5 text-xs text-amber-600 dark:text-amber-400" role="status">
          <TriangleAlert className="mt-0.5 size-3.5 shrink-0" aria-hidden /> {blocked}
        </p>
      ) : null}
      {layers.length ? (
        <div className="grid grid-cols-2 gap-2">
          <Select value={ownDraw || OWN_DEFAULT} items={{ [OWN_DEFAULT]: "Default drawing", ...DRAW_ITEMS }} onValueChange={(v) => onOwnChange({ draw: !v || v === OWN_DEFAULT ? "" : v })} disabled={disabled}>
            <SelectTrigger className="w-full" aria-label="This chart's own series drawn as"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value={OWN_DEFAULT}>Default drawing</SelectItem>
              {Object.entries(DRAW_ITEMS).map(([v, label]) => <SelectItem key={v} value={v}>{label}</SelectItem>)}
            </SelectContent>
          </Select>
          <Select value={ownAxis || OWN_DEFAULT} items={{ [OWN_DEFAULT]: "Default axis", ...AXIS_ITEMS }} onValueChange={(v) => onOwnChange({ axis: !v || v === OWN_DEFAULT ? "" : v })} disabled={disabled}>
            <SelectTrigger className="w-full" aria-label="This chart's own series axis"><SelectValue /></SelectTrigger>
            <SelectContent>
              <SelectItem value={OWN_DEFAULT}>Default axis</SelectItem>
              {Object.entries(AXIS_ITEMS).map(([v, label]) => <SelectItem key={v} value={v}>{label}</SelectItem>)}
            </SelectContent>
          </Select>
        </div>
      ) : null}
      {layers.map((layer, i) => {
        const n = i + 1;
        const fields = loaded[keyOf(layer)];
        const dimensions = fields ? matchingDimensions(baseKind, grained, fields.columns) : [];
        const choices = new Set(dimensions.map((d) => d.name));
        return (
          <div key={i} className="grid gap-2 rounded-md border border-border p-2" data-testid={`layer-${n}`}>
            <div className="flex items-center gap-1.5">
              <span className="text-xs font-medium text-muted-foreground">Question {n}</span>
              <span className="flex-1" />
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move question ${n} up`} disabled={disabled || i === 0} onClick={() => onChange(moveItem(layers, i, -1))}><ArrowUp /></Button>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move question ${n} down`} disabled={disabled || i === layers.length - 1} onClick={() => onChange(moveItem(layers, i, 1))}><ArrowDown /></Button>
              <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove question ${n}`} disabled={disabled} onClick={() => onChange(layers.filter((_, k) => k !== i))}><X /></Button>
            </div>
            <Select
              value={layer.source} items={sourceItems} disabled={disabled}
              onValueChange={(v) => v && set(i, { source: v, dimension: "", measures: [{ column: "", aggregate: "" }], filters: [], joins: [] })}
            >
              <SelectTrigger className="w-full" aria-label={`Question ${n} source`}><SelectValue placeholder="pick a source" /></SelectTrigger>
              <SelectContent>
                <SelectGroup>
                  <SelectLabel>Gold marts</SelectLabel>
                  {marts.filter((m) => m !== "").map((m) => <SelectItem key={m} value={encodeSourceChoice({ kind: "mart", name: m })}>{m}</SelectItem>)}
                </SelectGroup>
                {sqlSources.length ? (
                  <SelectGroup>
                    <SelectLabel>SQL sources</SelectLabel>
                    {sqlSources.map((s) => <SelectItem key={s.id} value={encodeSourceChoice({ kind: "sql", id: s.id })}>{s.title}</SelectItem>)}
                  </SelectGroup>
                ) : null}
              </SelectContent>
            </Select>
            {layer.source ? (
              <>
                <div className="grid gap-1.5">
                  <Label>Lined up on</Label>
                  <Select value={choices.has(layer.dimension) ? layer.dimension : ""} onValueChange={(v) => set(i, { dimension: v ?? "" })} disabled={disabled || !fields}>
                    <SelectTrigger className="w-full" aria-label={`Question ${n} dimension`}><SelectValue placeholder={fields ? "pick a column" : "loading"} /></SelectTrigger>
                    <SelectContent>{dimensions.map((d) => <SelectItem key={d.name} value={d.name}>{displayColumn(d.name)}</SelectItem>)}</SelectContent>
                  </Select>
                  {fields && !dimensions.length ? <p className="text-xs text-muted-foreground">{noMatchNote(baseKind)}</p> : null}
                </div>
                <MeasureRows
                  rows={layer.measures} onChange={(measures) => set(i, { measures })} labels={(k) => (k === 0 ? "Measure" : `Measure ${k + 1}`)}
                  min={1} max={6} own columns={fields?.measures ?? []} defaultAggregate={layer.aggregate}
                  disabled={disabled || !fields} placeholder={fields ? "pick a column" : "loading"} wholeMeasure={new Set<string>()}
                  fieldLabel={(name) => displayColumn(name)}
                />
                <div className="grid gap-1.5">
                  <Label>Default aggregate</Label>
                  <Select value={layer.aggregate} onValueChange={(v) => v && set(i, { aggregate: v })} disabled={disabled}>
                    <SelectTrigger className="w-full" aria-label={`Question ${n} aggregate`}><SelectValue /></SelectTrigger>
                    <SelectContent>{AGGREGATES.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                  </Select>
                </div>
                <ChartFilters value={layer.filters} onChange={(filters) => set(i, { filters })} columns={fields?.columns ?? []} disabled={disabled || !fields} />
              </>
            ) : null}
            <div className="grid gap-1.5">
              <Label>Legend name <span className="font-normal text-muted-foreground">(optional)</span></Label>
              <Input value={layer.label} onChange={(e) => set(i, { label: e.target.value })} placeholder={layer.source ? (sourceItems[layer.source] ?? "") : ""} aria-label={`Question ${n} label`} maxLength={60} disabled={disabled} />
            </div>
            <div className="grid grid-cols-2 gap-2">
              <Select value={layer.draw} items={DRAW_ITEMS} onValueChange={(v) => v && set(i, { draw: v as LayerDraw })} disabled={disabled}>
                <SelectTrigger className="w-full" aria-label={`Question ${n} drawn as`}><SelectValue /></SelectTrigger>
                <SelectContent>{Object.entries(DRAW_ITEMS).map(([v, label]) => <SelectItem key={v} value={v}>{label}</SelectItem>)}</SelectContent>
              </Select>
              <Select value={layer.axis} items={AXIS_ITEMS} onValueChange={(v) => v && set(i, { axis: v as LayerAxis })} disabled={disabled}>
                <SelectTrigger className="w-full" aria-label={`Question ${n} axis`}><SelectValue /></SelectTrigger>
                <SelectContent>{Object.entries(AXIS_ITEMS).map(([v, label]) => <SelectItem key={v} value={v}>{label}</SelectItem>)}</SelectContent>
              </Select>
            </div>
          </div>
        );
      })}
      {layers.length < MAX_LAYERS ? (
        <div>
          <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={disabled || !!blocked} onClick={() => onChange([...layers, emptyLayer()])}>
            <Plus className="size-3" aria-hidden /> Add a question
          </Button>
        </div>
      ) : null}
    </div>
  );
}
