"use client";

import * as React from "react";
import { ArrowDown, ArrowUp, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { hierarchyService } from "@/services";
import { MAX_LEVELS, MIN_LEVELS, type Hierarchy, type HierarchySource } from "@/services/contracts/hierarchies";
import { hierarchyProblems } from "@/lib/hierarchies";
import { moveItem } from "@/lib/table-draft";

/**
 * Define or change a hierarchy (`BI-10`): a name and 2 to 6 columns of the
 * source, outermost first. Nothing is stored on a chart, so deleting one
 * never breaks a chart.
 */
export function HierarchyDialog({
  open, onOpenChange, source, columns, existing, onSaved, onDeleted,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly source: HierarchySource;
  readonly columns: readonly string[];
  readonly existing?: Hierarchy;
  readonly onSaved: (hierarchy: Hierarchy) => void;
  readonly onDeleted?: (hierarchy: Hierarchy) => void;
}) {
  const [name, setName] = React.useState("");
  const [levels, setLevels] = React.useState<string[]>(["", ""]);
  const [busy, setBusy] = React.useState(false);
  const [problem, setProblem] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    setName(existing?.name ?? "");
    setLevels(existing?.levels.length ? [...existing.levels] : ["", ""]);
    setProblem(null);
  }, [open, existing]);

  const missing = hierarchyProblems(name, levels);
  const set = (i: number, column: string) => setLevels((ls) => ls.map((l, j) => (j === i ? column : l)));

  async function save() {
    setBusy(true);
    setProblem(null);
    try {
      const saved = existing
        ? await hierarchyService.update(existing.id, name.trim(), levels)
        : await hierarchyService.create(source, name.trim(), levels);
      onSaved(saved);
      onOpenChange(false);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The hierarchy could not be saved.");
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!existing) return;
    setBusy(true);
    setProblem(null);
    try {
      await hierarchyService.remove(existing.id);
      onDeleted?.(existing);
      onOpenChange(false);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The hierarchy could not be deleted.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{existing ? `Edit ${existing.name}` : "New hierarchy"}</DialogTitle>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="hier-name">Name</Label>
            <Input id="hier-name" value={name} placeholder="Geography" maxLength={64} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="grid gap-1.5">
            <Label>Levels, outermost first</Label>
            <ol className="grid gap-1.5">
              {levels.map((level, i) => (
                <li key={i} className="flex items-center gap-1.5">
                  <span className="w-5 shrink-0 text-xs text-muted-foreground">{i + 1}</span>
                  <Select value={level} onValueChange={(v) => set(i, v ?? "")}>
                    <SelectTrigger className="w-full min-w-0 flex-1" aria-label={`Level ${i + 1}`}><SelectValue placeholder="pick a column" /></SelectTrigger>
                    <SelectContent>
                      {columns.filter((c) => c === level || !levels.includes(c)).map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}
                    </SelectContent>
                  </Select>
                  <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move level ${i + 1} up`} disabled={i === 0} onClick={() => setLevels(moveItem(levels, i, -1))}><ArrowUp /></Button>
                  <Button type="button" size="icon-xs" variant="ghost" aria-label={`Move level ${i + 1} down`} disabled={i === levels.length - 1} onClick={() => setLevels(moveItem(levels, i, 1))}><ArrowDown /></Button>
                  <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove level ${i + 1}`} disabled={levels.length <= MIN_LEVELS} onClick={() => setLevels(levels.filter((_, j) => j !== i))}><X /></Button>
                </li>
              ))}
            </ol>
            {levels.length < MAX_LEVELS ? (
              <div>
                <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" onClick={() => setLevels([...levels, ""])}>
                  <Plus className="size-3" aria-hidden /> Add a level
                </Button>
              </div>
            ) : null}
          </div>
          {missing.length && !problem ? <p className="text-xs text-muted-foreground">Still needed: {missing.join(", ")}.</p> : null}
          {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
        </div>
        <DialogFooter>
          {existing ? (
            <Button type="button" variant="outline" className="sm:mr-auto" disabled={busy} onClick={() => void remove()}>Delete</Button>
          ) : null}
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button type="button" disabled={missing.length > 0 || busy} onClick={() => void save()}>{existing ? "Save" : "Create"}</Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
