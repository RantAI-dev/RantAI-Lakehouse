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
import {
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import type { ChartKind } from "@/lib/dashboard-specs";

type Fields = { dimensions: string[]; measures: string[] };
const KINDS: { value: ChartKind; label: string }[] = [
  { value: "bar", label: "Batang" },
  { value: "hbar", label: "Batang horizontal (peringkat)" },
  { value: "line", label: "Garis (tren)" },
  { value: "area", label: "Area (tren)" },
  { value: "pie", label: "Donat (komposisi)" },
  { value: "stacked", label: "Batang bertumpuk (≥2 measure)" },
];
const AGGS = ["sum", "avg", "max", "min", "count"];

/**
 * Builder chart MANUAL (jalur "Tableau"): pilih mart Gold → kolom → tipe, server
 * menyusun SQL-nya. Menulis ke artefak yang SAMA dengan jalur chat (console.
 * bi_chart), jadi manual & AI selalu sinkron.
 */
export function ChartBuilder({ onCreated }: { onCreated: () => void }) {
  const [open, setOpen] = React.useState(false);
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
  const [breakdown, setBreakdown] = React.useState("");
  const [aggregate, setAggregate] = React.useState("sum");
  const [span, setSpan] = React.useState<1 | 2>(1);
  const canBreakdown = kind !== "pie" && kind !== "stacked";

  // Muat daftar mart saat dialog dibuka.
  React.useEffect(() => {
    if (!open) return;
    void fetch("/api/dashboard/fields")
      .then((r) => r.json())
      .then((j) => setMarts(j.marts ?? []))
      .catch(() => setMarts([]));
  }, [open]);

  // Muat kolom saat mart dipilih.
  React.useEffect(() => {
    if (!mart) return setFields(null);
    setFields(null);
    setDimension("");
    setMeasure("");
    setMeasure2("");
    void fetch(`/api/dashboard/fields?mart=${encodeURIComponent(mart)}`)
      .then((r) => r.json())
      .then((j) => setFields({ dimensions: j.dimensions ?? [], measures: j.measures ?? [] }))
      .catch(() => setError("Gagal memuat kolom."));
  }, [mart]);

  function reset() {
    setTitle(""); setMart(""); setKind("hbar"); setDimension("");
    setMeasure(""); setMeasure2(""); setBreakdown(""); setAggregate("sum"); setSpan(1);
    setFields(null); setError(null);
  }

  async function save() {
    setError(null);
    const measures = kind === "stacked" ? [measure, measure2].filter(Boolean) : [measure].filter(Boolean);
    if (!title || !mart || !dimension || measures.length === 0) {
      setError("Lengkapi judul, mart, dimensi, dan measure.");
      return;
    }
    if (kind === "stacked" && measures.length < 2) {
      setError("Chart bertumpuk butuh 2 measure.");
      return;
    }
    setBusy(true);
    try {
      const res = await fetch("/api/dashboard/specs", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          title, mart, kind, dimension, measures, aggregate, span,
          breakdown: canBreakdown && breakdown ? breakdown : undefined,
        }),
      });
      const json = await res.json();
      if (!res.ok) throw new Error(json?.error ?? "Gagal membuat chart");
      setOpen(false);
      reset();
      onCreated();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={(o) => { setOpen(o); if (!o) reset(); }}>
      <DialogTrigger render={<Button variant="outline" size="sm" />}>
        <Plus className="size-4" /> Chart baru
      </DialogTrigger>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Chart baru</DialogTitle>
          <DialogDescription>
            Pilih mart Gold & kolom — server menyusun query-nya. Tersimpan di lakehouse dan langsung tampil.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3 py-1">
          <div className="grid gap-1.5">
            <Label htmlFor="ch-title">Judul</Label>
            <Input id="ch-title" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="mis. Wisman per Kawasan" />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Mart (Gold)</Label>
              <Select value={mart} onValueChange={(v) => setMart(v ?? "")}>
                <SelectTrigger><SelectValue placeholder="pilih mart" /></SelectTrigger>
                <SelectContent>
                  {marts.map((m) => (
                    <SelectItem key={m.name} value={m.name}>{m.name} · {m.rows.toLocaleString("id-ID")}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Tipe</Label>
              <Select value={kind} onValueChange={(v) => setKind(v as ChartKind)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  {KINDS.map((k) => <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Dimensi (X)</Label>
              <Select value={dimension} onValueChange={(v) => setDimension(v ?? "")} disabled={!fields}>
                <SelectTrigger><SelectValue placeholder={fields ? "pilih kolom" : "pilih mart dulu"} /></SelectTrigger>
                <SelectContent>
                  {fields?.dimensions.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Agregasi</Label>
              <Select value={aggregate} onValueChange={(v) => setAggregate(v ?? "")}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  {AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Measure (Y)</Label>
              <Select value={measure} onValueChange={(v) => setMeasure(v ?? "")} disabled={!fields}>
                <SelectTrigger><SelectValue placeholder={fields ? "pilih kolom" : "pilih mart dulu"} /></SelectTrigger>
                <SelectContent>
                  {fields?.measures.map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            {kind === "stacked" ? (
              <div className="grid gap-1.5">
                <Label>Measure ke-2</Label>
                <Select value={measure2} onValueChange={(v) => setMeasure2(v ?? "")} disabled={!fields}>
                  <SelectTrigger><SelectValue placeholder="pilih kolom" /></SelectTrigger>
                  <SelectContent>
                    {fields?.measures.filter((m) => m !== measure).map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>
            ) : (
              <div className="grid gap-1.5">
                <Label>Lebar</Label>
                <Select value={String(span)} onValueChange={(v) => setSpan(v === "2" ? 2 : 1)}>
                  <SelectTrigger><SelectValue /></SelectTrigger>
                  <SelectContent>
                    <SelectItem value="1">Setengah</SelectItem>
                    <SelectItem value="2">Penuh</SelectItem>
                  </SelectContent>
                </Select>
              </div>
            )}
          </div>

          {canBreakdown ? (
            <div className="grid gap-1.5">
              <Label>Breakdown / seri (opsional)</Label>
              <Select
                value={breakdown || "__none__"}
                onValueChange={(v) => setBreakdown(v === "__none__" ? "" : v ?? "")}
                disabled={!fields}
              >
                <SelectTrigger><SelectValue placeholder="tanpa breakdown" /></SelectTrigger>
                <SelectContent>
                  <SelectItem value="__none__">— tanpa breakdown —</SelectItem>
                  {fields?.dimensions.filter((d) => d !== dimension).map((d) => (
                    <SelectItem key={d} value={d}>{d}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
              <p className="text-[11px] text-muted-foreground">Pecah jadi banyak seri (mis. per kawasan). Pakai 1 measure.</p>
            </div>
          ) : null}

          {error ? <p className="text-sm text-destructive">{error}</p> : null}
        </div>

        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Batal</DialogClose>
          <Button size="sm" onClick={() => void save()} disabled={busy}>
            {busy ? "Menyimpan…" : "Buat chart"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
