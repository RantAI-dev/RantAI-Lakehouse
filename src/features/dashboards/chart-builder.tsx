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
  Select, SelectContent, SelectItem, SelectTrigger, SelectValue,
} from "@/components/ui/select";
import type { ChartKind } from "@/lib/dashboard-specs";

type Fields = { dimensions: string[]; measures: string[] };
export type ChartDef = {
  title?: string; subtitle?: string; mart?: string; kind?: ChartKind;
  dimension?: string; measures?: string[]; breakdown?: string;
  aggregate?: string; span?: 1 | 2; board?: string; text?: string; caption?: string;
  order?: "desc" | "asc" | "none"; limit?: number;
};
type BoardOpt = { id: string; name: string };

const KINDS: { value: ChartKind; label: string }[] = [
  { value: "bar", label: "Batang" },
  { value: "hbar", label: "Batang horizontal (peringkat)" },
  { value: "line", label: "Garis (tren)" },
  { value: "area", label: "Area (tren)" },
  { value: "pie", label: "Donat (komposisi)" },
  { value: "stacked", label: "Batang bertumpuk (≥2 measure)" },
  { value: "kpi", label: "KPI — angka besar" },
  { value: "table", label: "Tabel data" },
  { value: "text", label: "Teks / catatan" },
];
const AGGS = ["sum", "avg", "max", "min", "count"];

/**
 * Builder chart — jalur MANUAL (ala Tableau). Dipakai dua mode:
 *  - BUAT (punya trigger sendiri "Chart baru"),
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
  const [breakdown, setBreakdown] = React.useState("");
  const [aggregate, setAggregate] = React.useState("sum");
  const [span, setSpan] = React.useState<1 | 2>(1);
  const [caption, setCaption] = React.useState("");
  const [text, setText] = React.useState("");
  const [order, setOrder] = React.useState<"desc" | "asc" | "none">("desc");
  const [limit, setLimit] = React.useState(20);
  const [targetBoard, setTargetBoard] = React.useState(board);
  const isText = kind === "text";
  const isKpi = kind === "kpi";
  const canBreakdown = kind === "bar" || kind === "hbar" || kind === "line" || kind === "area";
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
        });
      }
    } else {
      setTargetBoard(board);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  function reset() {
    setTitle(""); setMart(""); setKind("hbar"); setDimension("");
    setMeasure(""); setMeasure2(""); setBreakdown(""); setAggregate("sum"); setSpan(1);
    setCaption(""); setText(""); setOrder("desc"); setLimit(20);
    setTargetBoard(board); setFields(null); setError(null);
  }

  // Ganti mart oleh USER → reset pilihan kolom & muat ulang.
  function onMartChange(m: string) {
    setMart(m); setDimension(""); setMeasure(""); setMeasure2(""); setBreakdown("");
    if (m) void loadFields(m); else setFields(null);
  }

  async function save() {
    setError(null);
    if (!title) { setError("Judul wajib."); return; }
    let payload: Record<string, unknown>;
    if (isText) {
      if (!text.trim()) { setError("Isi teks/catatan."); return; }
      payload = { title, kind, text, span, board: targetBoard };
    } else if (isKpi) {
      if (!mart || !measure) { setError("Pilih mart & measure."); return; }
      payload = { title, kind, mart, measures: [measure], aggregate, caption: caption || undefined, span, board: targetBoard };
    } else {
      const measures = kind === "stacked" ? [measure, measure2].filter(Boolean) : [measure].filter(Boolean);
      if (!mart || !dimension || measures.length === 0) { setError("Lengkapi mart, dimensi, dan measure."); return; }
      if (kind === "stacked" && measures.length < 2) { setError("Bertumpuk butuh 2 measure."); return; }
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
      if (!res.ok) throw new Error(json?.error ?? "Gagal menyimpan chart");
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
          <Plus className="size-4" /> Chart baru
        </DialogTrigger>
      )}
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{isEdit ? "Ubah chart" : "Chart baru"}</DialogTitle>
          <DialogDescription>
            Pilih mart Gold & kolom — server menyusun query-nya. Tersimpan di lakehouse dan langsung tampil.
          </DialogDescription>
        </DialogHeader>

        <div className="grid gap-3 py-1">
          <div className="grid gap-1.5">
            <Label htmlFor="ch-title">Judul</Label>
            <Input id="ch-title" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="mis. Wisman per Kawasan" />
          </div>

          {/* Tipe + Board */}
          <div className="grid grid-cols-2 gap-3">
            <div className="grid gap-1.5">
              <Label>Tipe</Label>
              <Select value={kind} onValueChange={(v) => setKind(v as ChartKind)}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>{KINDS.map((k) => <SelectItem key={k.value} value={k.value}>{k.label}</SelectItem>)}</SelectContent>
              </Select>
            </div>
            <div className="grid gap-1.5">
              <Label>Dashboard</Label>
              <Select value={targetBoard} onValueChange={(v) => setTargetBoard(v ?? "default")}>
                <SelectTrigger><SelectValue /></SelectTrigger>
                <SelectContent>
                  {(boards.length ? boards : [{ id: "default", name: "Utama" }]).map((b) => (
                    <SelectItem key={b.id} value={b.id}>{b.name}</SelectItem>
                  ))}
                </SelectContent>
              </Select>
            </div>
          </div>

          {isText ? (
            <div className="grid gap-1.5">
              <Label>Konten (markdown)</Label>
              <Textarea rows={5} value={text} onChange={(e) => setText(e.target.value)} placeholder="Judul **tebal**, - poin, atau | tabel | GFM |." />
            </div>
          ) : (
            <>
              <div className="grid gap-1.5">
                <Label>Mart (Gold)</Label>
                <Select value={mart} onValueChange={(v) => onMartChange(v ?? "")}>
                  <SelectTrigger><SelectValue placeholder="pilih mart" /></SelectTrigger>
                  <SelectContent>
                    {marts.map((m) => <SelectItem key={m.name} value={m.name}>{m.name} · {m.rows.toLocaleString("id-ID")}</SelectItem>)}
                  </SelectContent>
                </Select>
              </div>

              {!isKpi ? (
                <div className="grid grid-cols-2 gap-3">
                  <div className="grid gap-1.5">
                    <Label>Dimensi (X)</Label>
                    <Select value={dimension} onValueChange={(v) => setDimension(v ?? "")} disabled={!fields}>
                      <SelectTrigger><SelectValue placeholder={fields ? "pilih kolom" : "pilih mart dulu"} /></SelectTrigger>
                      <SelectContent>{fields?.dimensions.map((d) => <SelectItem key={d} value={d}>{d}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label>Agregasi</Label>
                    <Select value={aggregate} onValueChange={(v) => setAggregate(v ?? "")}>
                      <SelectTrigger><SelectValue /></SelectTrigger>
                      <SelectContent>{AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                </div>
              ) : null}

              <div className="grid grid-cols-2 gap-3">
                <div className="grid gap-1.5">
                  <Label>Measure (Y)</Label>
                  <Select value={measure} onValueChange={(v) => setMeasure(v ?? "")} disabled={!fields}>
                    <SelectTrigger><SelectValue placeholder={fields ? "pilih kolom" : "pilih mart dulu"} /></SelectTrigger>
                    <SelectContent>{fields?.measures.map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                  </Select>
                </div>
                {isKpi ? (
                  <div className="grid gap-1.5">
                    <Label>Agregasi</Label>
                    <Select value={aggregate} onValueChange={(v) => setAggregate(v ?? "")}>
                      <SelectTrigger><SelectValue /></SelectTrigger>
                      <SelectContent>{AGGS.map((a) => <SelectItem key={a} value={a}>{a}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                ) : kind === "stacked" ? (
                  <div className="grid gap-1.5">
                    <Label>Measure ke-2</Label>
                    <Select value={measure2} onValueChange={(v) => setMeasure2(v ?? "")} disabled={!fields}>
                      <SelectTrigger><SelectValue placeholder="pilih kolom" /></SelectTrigger>
                      <SelectContent>{fields?.measures.filter((m) => m !== measure).map((m) => <SelectItem key={m} value={m}>{m}</SelectItem>)}</SelectContent>
                    </Select>
                  </div>
                ) : <div />}
              </div>

              {isKpi ? (
                <div className="grid gap-1.5">
                  <Label>Caption (opsional)</Label>
                  <Input value={caption} onChange={(e) => setCaption(e.target.value)} placeholder="mis. kunjungan mancanegara (akumulasi)" />
                </div>
              ) : (
                <div className="grid grid-cols-2 gap-3">
                  <div className="grid gap-1.5">
                    <Label>Urutan</Label>
                    <Select value={order} onValueChange={(v) => setOrder((v as "desc" | "asc" | "none") ?? "desc")}>
                      <SelectTrigger><SelectValue /></SelectTrigger>
                      <SelectContent>
                        <SelectItem value="desc">Tertinggi dulu</SelectItem>
                        <SelectItem value="asc">Terendah dulu</SelectItem>
                        <SelectItem value="none">Alami (per dimensi)</SelectItem>
                      </SelectContent>
                    </Select>
                  </div>
                  <div className="grid gap-1.5">
                    <Label>Batas (Top N)</Label>
                    <Input type="number" min={1} max={100} value={limit} onChange={(e) => setLimit(Math.max(1, Math.min(100, Number(e.target.value) || 20)))} />
                  </div>
                </div>
              )}

              {canBreakdown ? (
                <div className="grid gap-1.5">
                  <Label>Breakdown / seri (opsional)</Label>
                  <Select value={breakdown || "__none__"} onValueChange={(v) => setBreakdown(v === "__none__" ? "" : v ?? "")} disabled={!fields}>
                    <SelectTrigger><SelectValue placeholder="tanpa breakdown" /></SelectTrigger>
                    <SelectContent>
                      <SelectItem value="__none__">— tanpa breakdown —</SelectItem>
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
          <DialogClose render={<Button variant="ghost" size="sm" />}>Batal</DialogClose>
          <Button size="sm" onClick={() => void save()} disabled={busy}>
            {busy ? "Menyimpan…" : isEdit ? "Simpan perubahan" : "Buat chart"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
