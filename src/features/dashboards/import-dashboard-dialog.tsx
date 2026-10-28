"use client";

import * as React from "react";
import Link from "next/link";
import { ArrowRight, Upload } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogDescription, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import type { FolderLike } from "@/lib/folder-tree";
import { readableErrorMessage } from "@/lib/notify-message";
import { FolderOptions } from "./folder-options";
import {
  importDashboard, initialMapping, previewImport, readDashboardFile,
  type ImportPreview, type ImportResult,
} from "./dashboard-file";

const NO_FOLDER = "__none__";
const NO_TABLE = "__none__";

/**
 * "Import…" on the dashboards list (`BI-19` part D, plan D14): choose a file,
 * confirm which table here each of its tables means, choose the folder, and
 * read what was made and what was left out, each with its reason. Nothing is
 * created until Import is pressed; the new dashboard is the person's own, in the
 * folder they chose, whatever the file says.
 */
export function ImportDashboardDialog({
  open, onOpenChange, folders, onImported,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** The folders the person may file into. */
  readonly folders: FolderLike[];
  /** Called after a dashboard was made, so the list can be read again. */
  readonly onImported?: () => void;
}) {
  const [file, setFile] = React.useState<unknown>(null);
  const [preview, setPreview] = React.useState<ImportPreview | null>(null);
  const [mapping, setMapping] = React.useState<Record<string, string>>({});
  const [folderId, setFolderId] = React.useState(NO_FOLDER);
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [result, setResult] = React.useState<ImportResult | null>(null);
  const [fileName, setFileName] = React.useState<string | null>(null);
  const fileInput = React.useRef<HTMLInputElement>(null);

  React.useEffect(() => {
    if (!open) return;
    setFile(null); setPreview(null); setMapping({}); setFolderId(NO_FOLDER); setBusy(false); setError(null); setResult(null); setFileName(null);
  }, [open]);

  async function choose(chosen: File | undefined) {
    if (!chosen) return;
    setFileName(chosen.name);
    setBusy(true); setError(null); setPreview(null);
    try {
      const parsed = await readDashboardFile(chosen);
      const shown = await previewImport(parsed);
      setFile(parsed); setPreview(shown); setMapping(initialMapping(shown));
    } catch (err) {
      setError(readableErrorMessage(err instanceof Error ? err.message : String(err)));
    } finally {
      setBusy(false);
    }
  }

  async function run() {
    if (!preview) return;
    setBusy(true); setError(null);
    try {
      const made = await importDashboard(file, mapping, folderId === NO_FOLDER ? "" : folderId);
      setResult(made);
      onImported?.();
    } catch (err) {
      setError(readableErrorMessage(err instanceof Error ? err.message : String(err)));
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>Import a dashboard</DialogTitle>
          <DialogDescription>
            Choose a dashboard file. You pick the tables it reads from, and the new dashboard is yours, in the folder you choose.
          </DialogDescription>
        </DialogHeader>
        {result ? (
          <div className="flex flex-col gap-3 text-sm" data-testid="import-result">
            <p>
              <strong>{result.board.name}</strong> was made with {result.imported.length} of {result.imported.length + result.leftOut.length} charts.
            </p>
            {result.leftOut.length > 0 ? (
              <div>
                <p className="mb-1 font-medium">Left out</p>
                <ul className="flex flex-col gap-1">
                  {result.leftOut.map((l, i) => (
                    <li key={`${l.title}-${i}`} className="text-muted-foreground">
                      <span className="text-foreground">{l.title}</span>: {l.reason}
                    </li>
                  ))}
                </ul>
              </div>
            ) : null}
          </div>
        ) : (
          <div className="flex flex-col gap-3">
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="import-file">Dashboard file</Label>
              {/* BI-19 owner QA 2026-10-28, QA-10: the browser's bare file input (and its
                  native "No file chosen" tooltip) is hidden; the button below opens it. */}
              <input
                ref={fileInput}
                id="import-file"
                type="file"
                accept=".json,application/json"
                disabled={busy}
                onChange={(e) => void choose(e.target.files?.[0])}
                className="sr-only"
                tabIndex={-1}
              />
              <div className="flex items-center gap-2">
                <Button type="button" variant="outline" size="sm" disabled={busy} onClick={() => fileInput.current?.click()}>
                  <Upload className="size-4" aria-hidden /> Choose a file…
                </Button>
                <span className="min-w-0 truncate text-sm text-muted-foreground">{fileName ?? "No file chosen"}</span>
              </div>
            </div>
            {preview ? (
              <>
                <p className="text-sm">
                  <strong>{preview.name}</strong>: {preview.charts.length} {preview.charts.length === 1 ? "chart" : "charts"}.
                </p>
                {preview.charts.some((c) => c.readsSqlSource) && !preview.canImportSql ? (
                  <p role="status" className="text-xs text-muted-foreground">
                    Charts that read a SQL source need the dashboard SQL permission and will be left out.
                  </p>
                ) : null}
                {preview.tables.length > 0 ? (
                  <div className="grid grid-cols-[1fr_auto_1fr] items-center gap-x-2 gap-y-2 text-sm" role="group" aria-label="Table mapping">
                    <span className="text-xs text-muted-foreground">Table in the file</span>
                    <span aria-hidden />
                    <span className="text-xs text-muted-foreground">Table here</span>
                    {preview.tables.map((t) => (
                      <React.Fragment key={t.name}>
                        <div className="min-w-0 break-words">
                          {t.name}
                          {t.suggested && t.missingColumns.length > 0 ? (
                            <span className="block text-xs text-muted-foreground">
                              Missing here: {t.missingColumns.join(", ")}
                            </span>
                          ) : null}
                        </div>
                        <ArrowRight className="size-4 text-muted-foreground" aria-hidden />
                        <Select
                          value={mapping[t.name] ?? NO_TABLE}
                          onValueChange={(v) => setMapping((m) => {
                            const next = { ...m };
                            if (!v || v === NO_TABLE) delete next[t.name]; else next[t.name] = v;
                            return next;
                          })}
                        >
                          <SelectTrigger className="w-full" aria-label={`Table here for ${t.name}`}>
                            <SelectValue>{mapping[t.name] ?? "None (leave its charts out)"}</SelectValue>
                          </SelectTrigger>
                          <SelectContent>
                            <SelectItem value={NO_TABLE}>None (leave its charts out)</SelectItem>
                            {preview.readableTables.map((r) => <SelectItem key={r} value={r}>{r}</SelectItem>)}
                          </SelectContent>
                        </Select>
                      </React.Fragment>
                    ))}
                  </div>
                ) : null}
                {folders.length > 0 ? (
                  <div className="flex flex-col gap-1.5">
                    <Label>Folder</Label>
                    <Select value={folderId} onValueChange={(v) => setFolderId(v ?? NO_FOLDER)}>
                      <SelectTrigger className="w-full" aria-label="Folder">
                        <SelectValue>{folderId === NO_FOLDER ? "No folder" : folders.find((f) => f.id === folderId)?.name ?? "No folder"}</SelectValue>
                      </SelectTrigger>
                      <SelectContent>
                        <FolderOptions folders={folders} noneValue={NO_FOLDER} noneLabel="No folder" />
                      </SelectContent>
                    </Select>
                  </div>
                ) : null}
              </>
            ) : null}
            {error ? <p role="alert" className="text-sm text-destructive">{error}</p> : null}
          </div>
        )}
        <DialogFooter>
          {result ? (
            <>
              <DialogClose render={<Button variant="outline" size="sm" />}>Close</DialogClose>
              <Button size="sm" render={<Link href={`/dashboards/${result.board.id}`} />}>Open dashboard</Button>
            </>
          ) : (
            <>
              <DialogClose render={<Button variant="outline" size="sm" />}>Cancel</DialogClose>
              <Button size="sm" disabled={!preview || busy} onClick={() => void run()}>
                {busy ? "Working…" : "Import"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
