"use client";

import * as React from "react";
import { Plus } from "lucide-react";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogDescription,
  DialogFooter, DialogTrigger, DialogClose,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import {
  Select, SelectContent, SelectGroup, SelectLabel, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import type { ChartKind } from "@/lib/dashboard-specs";

type Fields = { dimensions: string[]; measures: string[] };
export type ChartDef = {
  title?: string; subtitle?: string; mart?: string; kind?: ChartKind;
  dimension?: string; measures?: string[]; breakdown?: string;
  aggregate?: string; span?: 1 | 2; board?: string; text?: string; caption?: string;
  order?: "desc" | "asc" | "none"; limit?: number; target?: number;
};
type BoardOpt = { id: string; name: string };

/** Tipe chart dikelompokkan (ala pemilih visualisasi Metabase/Tableau). */
const KIND_GROUPS: { group: string; items: { value: ChartKind; label: string }[] }[] = [
  { group: "Comparison", items: [
    { value: "bar", label: "Bar" },
    { value: "hbar", label: "Horizontal bar (ranking)" },
    { value: "stacked", label: "Stacked bar (≥2 measures)" },
    { value: "combo", label: "Combo (bar + line)" },
  ] },
  { group: "Trend", items: [
    { value: "line", label: "Line" },
    { value: "area", label: "Area" },
    { value: "waterfall", label: "Waterfall (cumulative)" },
  ] },
  { group: "Composition", items: [
    { value: "pie", label: "Donut / pie" },
    { value: "rose", label: "Rose (nightingale)" },
    { value: "funnel", label: "Funnel" },
    { value: "treemap", label: "Treemap" },
  ] },
  { group: "Relationship / distribution", items: [
    { value: "scatter", label: "Scatter (X vs Y)" },
    { value: "bubble", label: "Bubble (X, Y, size)" },
    { value: "heatmap", label: "Heatmap (2 dimensions)" },
    { value: "radar", label: "Radar" },
  ] },
  { group: "Geographic", items: [
    { value: "geomap", label: "Map — Jakarta regions (choropleth)" },
  ] },
  { group: "Single value", items: [
    { value: "kpi", label: "KPI — big number" },
    { value: "gauge", label: "Gauge" },
  ] },
  { group: "Other", items: [
    { value: "table", label: "Data table" },
    { value: "text", label: "Text / note" },
  ] },
];
const AGGS = ["sum", "avg", "max", "min", "count"];
/** Label measure yang berubah menurut kind (X/Y/size, bar/line, dst). */
const MEASURE_LABELS: Partial<Record<ChartKind, string[]>> = {
  scatter: ["X metric", "Y metric"],
  bubble: ["X metric", "Y metric", "Size metric"],
  combo: ["Bar metric", "Line metric"],
  stacked: ["Measure", "2nd measure"],
};

/**
 * Builder chart — jalur MANUAL (ala Tableau). Dipakai dua mode:
 *  - BUAT (punya trigger sendiri "New chart"),
 *  - EDIT (dikendalikan induk: `open`, `initial` berisi id+def).
 * Menulis ke artefak yang SAMA dengan jalur chat (console.bi_chart).
 */
export function ChartBuilder({
  onSaved, board = "default", boards = [], initial, editId,
  open: openProp, onOpenChange, hideTrigger,
}: {
  onSaved: () => void;
  board?: string;
  boards?: BoardOpt[];
  initial?: ChartDef;
  editId?: string;
  open?: boolean;
  onOpenChange?: (o: boolean) => void;
  hideTrigger?: boolean;
}) {
  const controlled = openProp !== undefined;
  const [openState, setOpenState] = React.useState(false);
  const open = controlled ? openProp! : openState;
  const setOpen = (o: boolean) => { onOpenChange?.(o); if (!controlled) setOpenState(o); };

  const [marts, setMarts] = React.useState<{ name: string; rows: number }[]>([]);
  const [fields, setFields] = React.useState<Fields | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);

  const [title, setTitle] = React.useState("");
  const [mart, setMart] = React.useState("");
  const [kind, setKind] = React.useState<ChartKind>("hbar");
  const [dimension, setDimension] = React.useState("");
  const [measure, setMeasure] = React.useState("");
  const [measure2, setMeasure2] = React.useState("");
  const [measure3, setMeasure3] = React.useState("");
  const [breakdown, setBreakdown] = React.useState("");
  const [aggregate, setAggregate] = React.useState("sum");
  const [span, setSpan] = React.useState<1 | 2>(1);
  const [caption, setCaption] = React.useState("");
  const [target, setTarget] = React.useState("");
  const [text, setText] = React.useState("");
  const [order, setOrder] = React.useState<"desc" | "asc" | "none">("desc");
  const [limit, setLimit] = React.useState(20);
  const [targetBoard, setTargetBoard] = React.useState(board);
  const isText = kind === "text";
  const isKpi = kind === "kpi";
  const isGauge = kind === "gauge";
  const isSingle = isKpi || isGauge;         // tanpa dimensi (angka tunggal)
  const isHeatmap = kind === "heatmap";
  const needsM2 = kind === "stacked" || kind === "scatter" || kind === "combo" || kind === "bubble";
  const needsM3 = kind === "bubble";
  const canBreakdown = kind === "bar" || kind === "hbar" || kind === "line" || kind === "area" || isHeatmap;
  const mLabels = MEASURE_LABELS[kind] ?? ["Measure (Y)"];
  const isEdit = !!editId;

  async function loadFields(m: string): Promise<Fields> {
    const j = await fetch(`/api/dashboard/fields?mart=${encodeURIComponent(m)}`).then((r) => r.json());
    const f = { dimensions: j.dimensions ?? [], measures: j.measures ?? [] };
    setFields(f);
    return f;
  }

  // Saat dibuka: muat mart, dan bila EDIT prefill dari initial.
  React.useEffect(() => {
    if (!open) return;
    void fetch("/api/dashboard/fields").then((r) => r.json()).then((j) => setMarts(j.marts ?? [])).catch(() => setMarts([]));
    if (initial) {
      setTitle(initial.title ?? "");
      setKind((initial.kind as ChartKind) ?? "hbar");
      setAggregate(initial.aggregate ?? "sum");
      setSpan(initial.span === 2 ? 2 : 1);
      setBreakdown(initial.breakdown ?? "");
      setCaption(initial.caption ?? "");
      setTarget(initial.target != null ? String(initial.target) : "");
      setText(initial.text ?? "");
      setOrder((initial.order as "desc" | "asc" | "none") ?? "desc");
      setLimit(initial.limit ?? 20);
      setTargetBoard(initial.board ?? board);
      const m = initial.mart ?? "";
      setMart(m);
      if (m) {
        void loadFields(m).then(() => {
          setDimension(initial.dimension ?? "");
          setMeasure(initial.measures?.[0] ?? "");
          setMeasure2(initial.measures?.[1] ?? "");
          setMeasure3(initial.measures?.[2] ?? "");
        });
      }
    } else {
      setTargetBoard(board);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function reset() {
    setTitle(""); setMart(""); setKind("hbar"); setDimension("");
    setMeasure(""); setMeasure2(""); setMeasure3(""); setBreakdown(""); setAggregate("sum"); setSpan(1);
    setCaption(""); setTarget(""); setText(""); setOrder("desc"); setLimit(20);
    setTargetBoard(board); setFields(null); setError(null);
  }

  // Ganti mart oleh USER → reset pilihan kolom & muat ulang.
  function onMartChange(m: string) {
    setMart(m); setDimension(""); setMeasure(""); setMeasure2(""); setMeasure3(""); setBreakdown("");
    if (m) void loadFields(m); else setFields(null);
  }

  async function save() {
    setError(null);
    if (!title) { setError("Title is required."); return; }
    let payload: Record<string, unknown>;
    if (isText) {
      if (!text.trim()) { setError("Enter text/note."); return; }
      payload = { title, kind, text, span, board: targetBoard };
    } else if (isSingle) {
      if (!mart || !measure) { setError("Pick a mart & measure."); return; }
      payload = {
        title, kind, mart, measures: [measure], aggregate, span, board: targetBoard,
        caption: isKpi && caption ? caption : undefined,
        target: isGauge && Number(target) > 0 ? Number(target) : undefined,
      };
    } else {
      const measures = (needsM3 ? [measure, measure2, measure3] : needsM2 ? [measure, measure2] : [measure]).filter(Boolean);
      if (!mart || !dimension || measures.length === 0) { setError("Fill in mart, dimension, and measure."); return; }
      if (needsM3 && measures.length < 3) { setError(`${mLabels.join(", ")} — need all 3.`); return; }
      if (needsM2 && measures.length < 2) { setError(`${mLabels.join(" & ")} — need both.`); return; }
      if (isHeatmap && !breakdown) { setError("Heatmap needs a breakdown (2nd dimension)."); return; }
      payload = {
        title, mart, kind, dimension, measures, aggregate, span, board: targetBoard,
        breakdown: canBreakdown && breakdown ? breakdown : undefined,
        order, limit,
      };
    }
    if (isEdit) payload.id = editId;
    setBusy(true);
    try {
      const res = await fetch("/api/dashboard/specs", {
        method: isEdit ? "PUT" : "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(payload),
      });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.error ?? "Failed to save chart");
      setOpen(false);
      if (!isEdit) reset();
      onSaved();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => { setOpen(o); if (!o && !isEdit) reset(); }}>
      {hideTrigger ? null : (
        <DialogTrigger render={<Button variant="outline" size="sm" />}>
          <Plus className="size-4" /> New chart
        </DialogTrigger>
      )}
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{isEdit ? "Edit chart" : "New chart"}</DialogTitle>
          <DialogDescription>
            Pick a Gold mart & columns — the server builds the query. Saved to the lakehouse and shown instantly.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3 py-1">
          <div className="grid gap-1.5">
            <Label htmlFor="ch-title">Title</Label>
            <Input id="ch-title" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g. Visitors by Region" />
          </div>

          {/* Tipe + Board */}
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Tipe</Label>
              <Select value={kind} onValueChange={(v) => setKind(v as ChartKind)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  {KIND_GROUPS.map((g) => (
                    <SelectGroup key={g.group}>
                      <SelectLabel>{g.group}</SelectLabel>
                      {g.items.map((k) => <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>)}
                    </SelectGroup>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Dashboard</Label>
              <Select value={targetBoard} onValueChange={(v) => setTargetBoard(v ?? "default")}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  {(boards.length ? boards : [{ id: "default", name: "Main" }]).map((b) => (
                    <SelectItem key={b.id} value={b.id}>{b.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          {isText ? (
            <div className="grid gap-1.5">
              <Label>Content (markdown)</Label>
              <Textarea rows={5} value={text} onChange={(e) => setText(e.target.value)} placeholder="Title **bold**, - bullets, or | table | GFM |." />
            </div>
          ) : (
            <>
              <div className="grid gap-1.5">
                <Label>Mart (Gold)</Label>
                <Select value={mart} onValueChange={(v) => onMartChange(v ?? "")}>
                  <SelectTrigger><SelectValue placeholder="pick a mart" /></SelectTrigger>
                  <SelectContent>
                    {marts.map((m) => <SelectItem key={m.name} value={m.name}>{m.name} · {m.rows.toLocaleString("id-ID")}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>

              {!isSingle ? (
                <div className="grid grid-cols-2 gap-3">
                  <div className="grid gap-1.5">
                    <Label>Dimension (X)</Label>
                    <Select value={dimension} onValueChange={(v) => setDimension(v ?? "")} disabled={!fields}>
                      <SelectTrigger><SelectValue placeholder={fields ? "pick a column" : "pick a mart first"} /></SelectTrigger>
                      <SelectContent>{fields?.dimensions.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label>Aggregation</Label>
                    <Select value={aggregate} onValueChange={(v) => setAggregate(v ?? "")}>
                      <SelectTrigger><SelectValue /></SelectTrigger>
                      <SelectContent>{AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                </div>
              ) : null}

              <div className="grid grid-cols-2 gap-3">
                <div className="grid gap-1.5">
                  <Label>{mLabels[0]}</Label>
                  <Select value={measure} onValueChange={(v) => setMeasure(v ?? "")} disabled={!fields}>
                    <SelectTrigger><SelectValue placeholder={fields ? "pick a column" : "pick a mart first"} /></SelectTrigger>
                    <SelectContent>{fields?.measures.map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                  </Select>
                </div>
                {isSingle ? (
                  <div className="grid gap-1.5">
                    <Label>Aggregation</Label>
                    <Select value={aggregate} onValueChange={(v) => setAggregate(v ?? "")}>
                      <SelectTrigger><SelectValue /></SelectTrigger>
                      <SelectContent>{AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                ) : needsM2 ? (
                  <div className="grid gap-1.5">
                    <Label>{mLabels[1] ?? "2nd measure"}</Label>
                    <Select value={measure2} onValueChange={(v) => setMeasure2(v ?? "")} disabled={!fields}>
                      <SelectTrigger><SelectValue placeholder="pick a column" /></SelectTrigger>
                      <SelectContent>{fields?.measures.filter((m) => m !== measure).map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                ) : <div />}
              </div>

              {needsM3 ? (
                <div className="grid gap-1.5">
                  <Label>{mLabels[2] ?? "3rd measure"}</Label>
                  <Select value={measure3} onValueChange={(v) => setMeasure3(v ?? "")} disabled={!fields}>
                    <SelectTrigger><SelectValue placeholder="pick a column" /></SelectTrigger>
                    <SelectContent>{fields?.measures.filter((m) => m !== measure && m !== measure2).map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                  </Select>
                </div>
              ) : null}

              {isKpi ? (
                <div className="grid gap-1.5">
                  <Label>Caption (optional)</Label>
                  <Input value={caption} onChange={(e) => setCaption(e.target.value)} placeholder="e.g. foreign visits (cumulative)" />
                </div>
              ) : isGauge ? (
                <div className="grid gap-1.5">
                  <Label>Target / max (optional)</Label>
                  <Input type="number" min={0} value={target} onChange={(e) => setTarget(e.target.value)} placeholder="auto from value if empty" />
                </div>
              ) : (
                <div className="grid grid-cols-2 gap-3">
                  <div className="grid gap-1.5">
                    <Label>Sort</Label>
                    <Select value={order} onValueChange={(v) => setOrder((v as "desc" | "asc" | "none") ?? "desc")}>
                      <SelectTrigger><SelectValue /></SelectTrigger>
                      <SelectContent>
                        <SelectItem value="desc">Highest first</SelectItem>
                        <SelectItem value="asc">Lowest first</SelectItem>
                        <SelectItem value="none">Natural (by dimension)</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label>Limit (Top N)</Label>
                    <Input type="number" min={1} max={100} value={limit} onChange={(e) => setLimit(Math.max(1, Math.min(100, Number(e.target.value) || 20)))} />
                  </div>
                </div>
              )}

              {canBreakdown ? (
                <div className="grid gap-1.5">
                  <Label>{isHeatmap ? "2nd dimension (Y) — required" : "Breakdown / series (optional)"}</Label>
                  <Select value={breakdown || "__none__"} onValueChange={(v) => setBreakdown(v === "__none__" ? "" : v ?? "")} disabled={!fields}>
                    <SelectTrigger><SelectValue placeholder={isHeatmap ? "pick a 2nd column" : "no breakdown"} /></SelectTrigger>
                    <SelectContent>
                      {isHeatmap ? null : <SelectItem value="__none__">— no breakdown —</SelectItem>}
                      {fields?.dimensions.filter((d) => d !== dimension).map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                    </SelectContent>
                  </Select>
                </div>
              ) : null}
            </>
          )}

          {error ? <p className="text-sm text-destructive">{error}</p> : null}
        </div>

        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" onClick={() => void save()} disabled={busy}>
            {busy ? "Saving…" : isEdit ? "Save changes" : "Create chart"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
