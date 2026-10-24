"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import type { DataChoice, PdfChoice } from "./chart-download";

/** A labelled pair of native radio inputs: small, keyboard-friendly, and testable by role. */
function Choice<T extends string>({
  legend, value, options, onChange,
}: {
  readonly legend: string;
  readonly value: T;
  readonly options: readonly { value: T; label: string }[];
  readonly onChange: (v: T) => void;
}) {
  return (
    <fieldset className="flex flex-col gap-1.5">
      <legend className="mb-1 text-sm font-medium">{legend}</legend>
      {options.map((o) => (
        <Label key={o.value} className="flex items-center gap-2 font-normal">
          <input type="radio" name={legend} checked={value === o.value} onChange={() => onChange(o.value)} />
          {o.label}
        </Label>
      ))}
    </fieldset>
  );
}

/**
 * "Download PDF…" (`BI-1`): the paper and the orientation, then the server makes
 * the file. The dialog stays open while it does and shows the sentence the
 * server gave when it fails, so the person can try again.
 */
export function DownloadPdfDialog({
  open, onOpenChange, onDownload,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** Makes and saves the file; rejects with an `Error` whose message is shown. */
  readonly onDownload: (choice: PdfChoice) => Promise<unknown>;
}) {
  const [paper, setPaper] = React.useState<PdfChoice["paper"]>("a4");
  const [orientation, setOrientation] = React.useState<PdfChoice["orientation"]>("portrait");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  async function go() {
    setBusy(true); setError(null);
    try {
      await onDownload({ paper, orientation });
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "The PDF could not be made.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open={open} onOpenChange={(next) => { if (!busy) onOpenChange(next); }}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader><DialogTitle>Download PDF</DialogTitle></DialogHeader>
        <p className="text-xs text-muted-foreground">Every tab, with the filters you have on, starting each tab on a new page.</p>
        <div className="flex gap-8">
          <Choice legend="Paper" value={paper} onChange={setPaper} options={[{ value: "a4", label: "A4" }, { value: "letter", label: "Letter" }]} />
          <Choice legend="Orientation" value={orientation} onChange={setOrientation} options={[{ value: "portrait", label: "Portrait" }, { value: "landscape", label: "Landscape" }]} />
        </div>
        {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        <DialogFooter>
          <Button variant="ghost" size="sm" disabled={busy} onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button size="sm" disabled={busy} onClick={() => void go()}>{busy ? "Making the PDF…" : "Download"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}

/** "Download data…" on a chart (`BI-1`): Excel or CSV, formatted as on the chart or raw values. */
export function DownloadDataDialog({
  open, onOpenChange, title, onDownload,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly title: string;
  /** Makes and saves the file; the server's sentence for a refusal is the rejection's message. */
  readonly onDownload: (choice: DataChoice) => Promise<unknown>;
}) {
  const [format, setFormat] = React.useState<DataChoice["format"]>("xlsx");
  const [values, setValues] = React.useState<DataChoice["values"]>("formatted");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  async function go() {
    setBusy(true); setError(null);
    try {
      await onDownload({ format, values });
      onOpenChange(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : "The download failed.");
    } finally {
      setBusy(false);
    }
  }
  return (
    <Dialog open={open} onOpenChange={(next) => { if (!busy) onOpenChange(next); }}>
      <DialogContent className="sm:max-w-sm">
        <DialogHeader><DialogTitle>Download data · {title}</DialogTitle></DialogHeader>
        <div className="flex gap-8">
          <Choice legend="File" value={format} onChange={setFormat} options={[{ value: "xlsx", label: "Excel (.xlsx)" }, { value: "csv", label: "CSV" }]} />
          <Choice legend="Values" value={values} onChange={setValues} options={[{ value: "formatted", label: "As on the chart" }, { value: "raw", label: "Raw values" }]} />
        </div>
        <p className="text-xs text-muted-foreground">Up to 1,000,000 rows. A file that reaches the limit says so in its name.</p>
        {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
        <DialogFooter>
          <Button variant="ghost" size="sm" disabled={busy} onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button size="sm" disabled={busy} onClick={() => void go()}>{busy ? "Preparing…" : "Download"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
