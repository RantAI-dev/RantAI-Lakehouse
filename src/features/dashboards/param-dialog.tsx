"use client";

import * as React from "react";
import { Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { Textarea } from "@/components/ui/textarea";
import { useService } from "@/hooks/use-service";
import { paramService } from "@/services";
import {
  PARAM_TYPE_LABELS, draftFromDef, emptyParamDraft, inputFromDraft, paramProblems, type ParamDraft,
} from "@/lib/param-form";
import type { ParamDef, ParamType } from "@/lib/params";

/**
 * Parameters (`BI-11`): the list, and one form to define or change one. Defined
 * once for the deployment; a dashboard shows the ones its editor ticks. Its name
 * and type stay once saved. Deleting is refused, with the server's sentence,
 * while a formula, a chart or a dashboard uses it.
 */
export function ParamDialog({
  open, onOpenChange, canWrite, shown, onToggleShown, onChanged,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly canWrite: boolean;
  /** Names the open dashboard shows; absent when the dialog is opened from the builder. */
  readonly shown?: readonly string[];
  readonly onToggleShown?: (name: string, on: boolean) => void;
  /** After a parameter was saved or deleted. */
  readonly onChanged?: () => void;
}) {
  const state = useService((signal) => paramService.list(signal), [open]);
  const defs = state.data ?? [];
  const [editing, setEditing] = React.useState<ParamDef | "new" | null>(null);
  const [draft, setDraft] = React.useState<ParamDraft>(emptyParamDraft());
  const [busy, setBusy] = React.useState(false);
  const [problem, setProblem] = React.useState<string | null>(null);

  React.useEffect(() => { if (!open) { setEditing(null); setProblem(null); } }, [open]);

  const edit = (target: ParamDef | "new") => {
    setEditing(target);
    setDraft(target === "new" ? emptyParamDraft() : draftFromDef(target));
    setProblem(null);
  };
  const missing = paramProblems(draft, editing !== "new");
  const set = (patch: Partial<ParamDraft>) => setDraft((d) => ({ ...d, ...patch }));

  async function save() {
    setBusy(true);
    setProblem(null);
    try {
      const input = inputFromDraft(draft);
      if (editing && editing !== "new") await paramService.update(editing.id, input);
      else await paramService.create(input);
      setEditing(null);
      state.reload();
      onChanged?.();
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The parameter could not be saved.");
    } finally {
      setBusy(false);
    }
  }

  async function remove(def: ParamDef) {
    setBusy(true);
    setProblem(null);
    try {
      await paramService.remove(def.id);
      setEditing(null);
      state.reload();
      onChanged?.();
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The parameter could not be deleted.");
    } finally {
      setBusy(false);
    }
  }

  const type = draft.type;
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>{editing === null ? "Parameters" : editing === "new" ? "New parameter" : `Edit ${editing.name}`}</DialogTitle>
        </DialogHeader>
        {editing === null ? (
          <div className="grid gap-2">
            {defs.length === 0 && state.status !== "loading" ? <p className="text-sm text-muted-foreground">No parameters yet.</p> : null}
            <ul className="grid gap-1">
              {defs.map((d) => (
                <li key={d.id} className="flex items-center gap-2 rounded-md border border-border px-2 py-1.5 text-sm">
                  {shown && onToggleShown ? (
                    <Checkbox
                      checked={shown.includes(d.name)}
                      disabled={!canWrite}
                      aria-label={`Show ${d.name} on this dashboard`}
                      onCheckedChange={(on) => onToggleShown(d.name, on === true)}
                    />
                  ) : null}
                  <span className="min-w-0 flex-1 truncate"><span className="font-medium">{d.label || d.name}</span> <span className="text-xs text-muted-foreground">{`{${d.name}}`} · {PARAM_TYPE_LABELS[d.type as ParamType] ?? d.type}</span></span>
                  {canWrite ? <Button type="button" size="xs" variant="ghost" onClick={() => edit(d)}>Edit</Button> : null}
                </li>
              ))}
            </ul>
            {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
            <DialogFooter>
              <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Close</Button>
              {canWrite ? <Button type="button" onClick={() => edit("new")}><Plus className="size-4" aria-hidden /> New parameter</Button> : null}
            </DialogFooter>
          </div>
        ) : (
          <div className="grid gap-3">
            <div className="grid grid-cols-2 gap-3">
              <div className="grid gap-1.5">
                <Label htmlFor="param-name">Name</Label>
                <Input id="param-name" value={draft.name} disabled={editing !== "new"} placeholder="target" maxLength={64} onChange={(e) => set({ name: e.target.value })} />
              </div>
              <div className="grid gap-1.5">
                <Label htmlFor="param-label">Label</Label>
                <Input id="param-label" value={draft.label} maxLength={100} onChange={(e) => set({ label: e.target.value })} />
              </div>
            </div>
            <div className="grid gap-1.5">
              <Label>Type</Label>
              <Select value={type} disabled={editing !== "new"} items={PARAM_TYPE_LABELS} onValueChange={(v) => v && set({ type: v as ParamType })}>
                <SelectTrigger className="w-full" aria-label="Type"><SelectValue /></SelectTrigger>
                <SelectContent>
                  {(Object.keys(PARAM_TYPE_LABELS) as ParamType[]).map((t) => <SelectItem key={t} value={t}>{PARAM_TYPE_LABELS[t]}</SelectItem>)}
                </SelectContent>
              </Select>
            </div>
            {type === "list" ? (
              <div className="grid gap-1.5">
                <Label htmlFor="param-values">Choices, one per line</Label>
                <Textarea id="param-values" rows={4} value={draft.values} placeholder={"visitors\nrooms | Rooms"} onChange={(e) => set({ values: e.target.value })} />
              </div>
            ) : null}
            <div className="grid grid-cols-3 gap-3">
              <div className="grid gap-1.5">
                <Label htmlFor="param-default">Default</Label>
                <Input id="param-default" type={type === "date" ? "date" : type === "number" ? "number" : "text"} value={draft.default} onChange={(e) => set({ default: e.target.value })} />
              </div>
              {type === "number" || type === "date" ? (
                <>
                  <div className="grid gap-1.5">
                    <Label htmlFor="param-min">Minimum</Label>
                    <Input id="param-min" type={type === "date" ? "date" : "number"} value={draft.min} onChange={(e) => set({ min: e.target.value })} />
                  </div>
                  <div className="grid gap-1.5">
                    <Label htmlFor="param-max">Maximum</Label>
                    <Input id="param-max" type={type === "date" ? "date" : "number"} value={draft.max} onChange={(e) => set({ max: e.target.value })} />
                  </div>
                </>
              ) : null}
              {type === "text" ? (
                <div className="grid gap-1.5">
                  <Label htmlFor="param-maxlen">Longest text</Label>
                  <Input id="param-maxlen" type="number" min={1} max={200} value={draft.maxLength} placeholder="200" onChange={(e) => set({ maxLength: e.target.value })} />
                </div>
              ) : null}
            </div>
            {missing.length && !problem ? <p className="text-xs text-muted-foreground">Still needed: {missing.join(", ")}.</p> : null}
            {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
            <DialogFooter>
              {editing !== "new" ? (
                <Button type="button" variant="outline" className="sm:mr-auto" disabled={busy} onClick={() => void remove(editing)}>Delete</Button>
              ) : null}
              <Button type="button" variant="outline" onClick={() => { setEditing(null); setProblem(null); }}>Back</Button>
              <Button type="button" disabled={missing.length > 0 || busy} onClick={() => void save()}>{editing === "new" ? "Create" : "Save"}</Button>
            </DialogFooter>
          </div>
        )}
      </DialogContent>
    </Dialog>
  );
}
