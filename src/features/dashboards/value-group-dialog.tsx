"use client";

import * as React from "react";
import { Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { calcFieldService } from "@/services";
import { MAX_GROUPS, groupDefOf, type CalcField, type CalcFieldSource, type ValueGroup } from "@/services/contracts/calc-fields";
import { addGroup, groupOf, groupPayload, groupProblems, toggleValue } from "@/lib/value-groups";
import { useFilterValues } from "./filters/use-filter-values";

const REST_ITEMS = { keep: "Keep their own names", other: "Call them Other" } as const;

/**
 * Create or change a value group (`BI-10`): pick the values of a text column
 * from its value list and gather them under names of your own. The rest
 * keeps its names or becomes "Other". Saved with the source's calculated
 * fields; the server checks every rule again.
 */
export function ValueGroupDialog({
  open, onOpenChange, source, textColumns, existing, onSaved, onDeleted,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly source: CalcFieldSource;
  /** The source's text columns. */
  readonly textColumns: readonly string[];
  /** The group being changed; absent when making a new one. */
  readonly existing?: CalcField;
  readonly onSaved: (field: CalcField) => void;
  readonly onDeleted?: (field: CalcField) => void;
}) {
  const [name, setName] = React.useState("");
  const [column, setColumn] = React.useState("");
  const [groups, setGroups] = React.useState<ValueGroup[]>([]);
  const [active, setActive] = React.useState(0);
  const [rest, setRest] = React.useState<"keep" | "other">("keep");
  const [search, setSearch] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [problem, setProblem] = React.useState<string | null>(null);

  React.useEffect(() => {
    if (!open) return;
    const def = existing ? groupDefOf(existing) : null;
    setName(existing?.name ?? "");
    setColumn(def?.column ?? "");
    setGroups(def?.groups ?? [{ name: "Group 1", values: [] }]);
    setActive(0);
    setRest(def?.rest ?? "keep");
    setSearch("");
    setProblem(null);
  }, [open, existing]);

  const list = useFilterValues({ board: "", column, others: [], search });
  const missing = groupProblems(name, column, groups, rest);
  const nameOk = existing ? true : /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name);

  async function save() {
    setBusy(true);
    setProblem(null);
    try {
      const def = groupPayload(column, groups, rest);
      const saved = existing
        ? await calcFieldService.updateGroup(existing.id, def)
        : await calcFieldService.createGroup(source, name, def);
      onSaved(saved);
      onOpenChange(false);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The group could not be saved.");
    } finally {
      setBusy(false);
    }
  }

  async function remove() {
    if (!existing) return;
    setBusy(true);
    setProblem(null);
    try {
      await calcFieldService.remove(existing.id);
      onDeleted?.(existing);
      onOpenChange(false);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The group could not be deleted.");
    } finally {
      setBusy(false);
    }
  }

  const rename = (i: number, next: string) => setGroups((gs) => gs.map((g, j) => (j === i ? { ...g, name: next } : g)));
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[90vh] overflow-y-auto sm:max-w-2xl">
        <DialogHeader>
          <DialogTitle>{existing ? `Edit ${existing.name}` : "New value group"}</DialogTitle>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="grid gap-1.5">
              <Label htmlFor="vg-name">Name</Label>
              <Input id="vg-name" value={name} disabled={!!existing} placeholder="region_group" maxLength={64} onChange={(e) => setName(e.target.value.trim())} />
            </div>
            <div className="grid gap-1.5">
              <Label>Column</Label>
              <Select value={column} onValueChange={(v) => { setColumn(v ?? ""); setGroups([{ name: "Group 1", values: [] }]); setActive(0); }} disabled={!!existing}>
                <SelectTrigger className="w-full" aria-label="Column"><SelectValue placeholder="pick a text column" /></SelectTrigger>
                <SelectContent>{textColumns.map((c) => <SelectItem key={c} value={c}>{c}</SelectItem>)}</SelectContent>
              </Select>
            </div>
          </div>

          <div className="grid grid-cols-1 gap-3 sm:grid-cols-2">
            <div className="grid content-start gap-1.5">
              <Label>Groups</Label>
              <ul className="grid gap-1.5">
                {groups.map((g, i) => (
                  <li key={i} className={`grid gap-1 rounded-md border p-1.5 ${i === active ? "border-primary" : "border-border"}`}>
                    <div className="flex items-center gap-1">
                      <Input
                        value={g.name} onChange={(e) => rename(i, e.target.value)} onFocus={() => setActive(i)}
                        aria-label={`Name of group ${i + 1}`} className="h-7" maxLength={64}
                      />
                      <Button type="button" size="icon-xs" variant="ghost" aria-label={`Remove group ${i + 1}`}
                        onClick={() => { setGroups((gs) => gs.filter((_, j) => j !== i)); setActive(0); }}><X /></Button>
                    </div>
                    <button type="button" className="text-left text-xs text-muted-foreground" onClick={() => setActive(i)}>
                      {g.values.length ? `${g.values.length} value${g.values.length === 1 ? "" : "s"}: ${g.values.slice(0, 3).join(", ")}${g.values.length > 3 ? ", …" : ""}` : "no values yet"}
                    </button>
                  </li>
                ))}
              </ul>
              <div>
                <Button type="button" size="sm" variant="ghost" className="h-7 px-2 text-xs" disabled={groups.length >= MAX_GROUPS}
                  onClick={() => { setGroups((gs) => addGroup(gs)); setActive(groups.length); }}>
                  <Plus className="size-3" aria-hidden /> Add a group
                </Button>
              </div>
            </div>

            <div className="grid content-start gap-1.5">
              <Label>Values of {column || "the column"}</Label>
              <Input value={search} onChange={(e) => setSearch(e.target.value)} placeholder="Search" aria-label="Search values" disabled={!column} />
              <ul className="max-h-56 overflow-auto rounded-md border border-border" aria-label="Values">
                {list.loading ? <li className="px-2 py-1.5 text-xs text-muted-foreground">Loading…</li> : null}
                {list.error ? <li role="alert" className="px-2 py-1.5 text-xs text-destructive">{list.error}</li> : null}
                {!list.loading && !list.error && column && !list.values.length ? <li className="px-2 py-1.5 text-xs text-muted-foreground">No values.</li> : null}
                {list.values.map((v) => {
                  const holder = groupOf(groups, v);
                  const taken = holder !== -1 && holder !== active;
                  return (
                    <li key={v}>
                      <label className={`flex items-center gap-2 px-2 py-1 text-sm ${taken ? "opacity-50" : "cursor-pointer hover:bg-muted"}`}>
                        <Checkbox checked={holder === active} disabled={taken || !groups.length} onCheckedChange={() => setGroups((gs) => toggleValue(gs, active, v))} aria-label={v} />
                        <span className="min-w-0 flex-1 truncate">{v}</span>
                        {taken ? <span className="shrink-0 text-xs text-muted-foreground">{groups[holder]?.name}</span> : null}
                      </label>
                    </li>
                  );
                })}
              </ul>
              {list.truncated ? <p className="text-xs text-muted-foreground">Search to find more values.</p> : null}
            </div>
          </div>

          <div className="grid gap-1.5 sm:max-w-xs">
            <Label>The other values</Label>
            <Select value={rest} items={REST_ITEMS} onValueChange={(v) => setRest(v === "other" ? "other" : "keep")}>
              <SelectTrigger className="w-full" aria-label="The other values"><SelectValue /></SelectTrigger>
              <SelectContent>
                <SelectItem value="keep">{REST_ITEMS.keep}</SelectItem>
                <SelectItem value="other">{REST_ITEMS.other}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          {missing.length && !problem ? <p className="text-xs text-muted-foreground">Still needed: {missing.join(", ")}.</p> : null}
          {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
        </div>
        <DialogFooter>
          {existing ? (
            <Button type="button" variant="outline" className="sm:mr-auto" disabled={busy} onClick={() => void remove()}>Delete</Button>
          ) : null}
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button type="button" disabled={missing.length > 0 || !nameOk || busy} onClick={() => void save()}>
            {existing ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
