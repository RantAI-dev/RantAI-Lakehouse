"use client";

import * as React from "react";
import {
  Dialog, DialogContent, DialogHeader, DialogTitle, DialogFooter,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Textarea } from "@/components/ui/textarea";
import { calcFieldService } from "@/services";
import type {
  CalcField, CalcFieldSource, FormulaCheck, FormulaFunction,
} from "@/services/contracts/calc-fields";
import { applySuggestion, callAtCaret, splitAtProblem, suggest } from "@/lib/formula-assist";
import { cn } from "@/lib/utils";

/** How long the box waits after the last keystroke before asking the server to check. */
const CHECK_DELAY_MS = 450;

/**
 * Create or change a calculated field (BI-8). The server is the only judge:
 * the box asks `validate` once typing pauses and shows its message with the
 * span it names marked in the formula; Save stays off until the answer is
 * `ok`. Suggestions and the help line come from the function catalog and the
 * source's columns, and only help typing.
 */
export function FormulaEditorDialog({
  open, onOpenChange, source, columns, fields, existing, onSaved, onDeleted,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  source: CalcFieldSource;
  /** The source's own column names. */
  columns: string[];
  /** The source's calculated fields (the one being edited is left out of the suggestions). */
  fields: CalcField[];
  /** The field being changed; absent when making a new one. */
  existing?: CalcField;
  onSaved: (field: CalcField) => void;
  onDeleted?: (field: CalcField) => void;
}) {
  const [name, setName] = React.useState("");
  const [formula, setFormula] = React.useState("");
  const [caret, setCaret] = React.useState(0);
  const [check, setCheck] = React.useState<FormulaCheck | "pending" | null>(null);
  const [functions, setFunctions] = React.useState<FormulaFunction[]>([]);
  const [problem, setProblem] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const area = React.useRef<HTMLTextAreaElement>(null);

  React.useEffect(() => {
    if (!open) return;
    setName(existing?.name ?? "");
    setFormula(existing?.formula ?? "");
    setCaret((existing?.formula ?? "").length);
    setCheck(null);
    setProblem(null);
    void calcFieldService.functions().then(setFunctions).catch(() => setFunctions([]));
  }, [open, existing]);

  // Validate on pause. A newer formula cancels the answer of an older one.
  React.useEffect(() => {
    if (!open) return;
    if (!formula.trim()) { setCheck(null); return; }
    setCheck("pending");
    const ctl = new AbortController();
    const timer = setTimeout(() => {
      calcFieldService.validate(source, formula, name || undefined, ctl.signal)
        .then(setCheck)
        .catch((e: unknown) => {
          if (ctl.signal.aborted) return;
          setCheck(null);
          setProblem(e instanceof Error ? e.message : "The formula could not be checked.");
        });
    }, CHECK_DELAY_MS);
    return () => { clearTimeout(timer); ctl.abort(); };
    // `source` is a fresh object each render of the parent; its two ids are the real dependency.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open, formula, name, source.mart, source.source]);

  const names = React.useMemo(() => ({
    columns,
    fields: fields.filter((f) => f.id !== existing?.id).map((f) => ({ name: f.name, formula: f.formula })),
  }), [columns, fields, existing]);
  const hints = suggest(formula, caret, names, functions);
  const call = callAtCaret(formula, caret, functions);
  const nameOk = !!existing || /^[A-Za-z_][A-Za-z0-9_]{0,63}$/.test(name);
  const valid = check !== null && check !== "pending" && check.ok;
  const split = check && check !== "pending" && !check.ok ? splitAtProblem(formula, check.error) : null;

  function pick(i: number) {
    const out = applySuggestion(formula, hints, hints.items[i]);
    setFormula(out.text);
    setCaret(out.caret);
    requestAnimationFrame(() => {
      area.current?.focus();
      area.current?.setSelectionRange(out.caret, out.caret);
    });
  }

  async function save() {
    setBusy(true);
    setProblem(null);
    try {
      const saved = existing
        ? await calcFieldService.update(existing.id, formula)
        : await calcFieldService.create(source, name, formula);
      onSaved(saved);
      onOpenChange(false);
    } catch (e) {
      setProblem(e instanceof Error ? e.message : "The field could not be saved.");
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
      setProblem(e instanceof Error ? e.message : "The field could not be deleted.");
    } finally {
      setBusy(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-xl">
        <DialogHeader>
          <DialogTitle>{existing ? `Edit ${existing.name}` : "New calculated field"}</DialogTitle>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="calc-name">Name</Label>
            <Input
              id="calc-name" value={name} disabled={!!existing} placeholder="profit"
              onChange={(e) => setName(e.target.value.trim())} maxLength={64}
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="calc-formula">Formula</Label>
            <Textarea
              id="calc-formula" ref={area} value={formula} rows={4} spellCheck={false}
              className="font-mono text-sm" placeholder="[revenue] - [cost]"
              aria-invalid={!!split}
              onChange={(e) => { setFormula(e.target.value); setCaret(e.target.selectionStart); }}
              onSelect={(e) => setCaret(e.currentTarget.selectionStart)}
              onKeyUp={(e) => setCaret(e.currentTarget.selectionStart)}
            />
            {call ? (
              <p className="text-xs text-muted-foreground">
                <span className="font-mono">{call.fn.signature}</span> {call.fn.help}
              </p>
            ) : null}
            {hints.items.length ? (
              <ul className="flex flex-wrap gap-1" aria-label="Suggestions">
                {hints.items.map((s, i) => (
                  <li key={`${s.kind}-${s.label}`}>
                    <button
                      type="button" title={s.detail}
                      className={cn(
                        "rounded border px-1.5 py-0.5 font-mono text-xs hover:bg-muted",
                        s.kind === "function" && "text-primary"
                      )}
                      onMouseDown={(e) => e.preventDefault()}
                      onClick={() => pick(i)}
                    >
                      {s.label}
                    </button>
                  </li>
                ))}
              </ul>
            ) : null}
          </div>
          {split && check && check !== "pending" && !check.ok ? (
            <div role="alert" className="grid gap-1 text-sm text-destructive">
              <code className="whitespace-pre-wrap break-all rounded bg-muted px-2 py-1 font-mono text-xs text-foreground">
                {split.before}
                <mark className="rounded bg-destructive/20 px-0.5 text-foreground underline decoration-destructive decoration-wavy">
                  {split.hit || " "}
                </mark>
                {split.after}
              </code>
              <span>{check.error.message}</span>
            </div>
          ) : check && check !== "pending" && check.ok ? (
            <p className="text-xs text-muted-foreground">
              {check.level === "aggregate" ? "Aggregate" : "One value per row"}, {check.type}.
            </p>
          ) : null}
          {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
        </div>
        <DialogFooter>
          {existing ? (
            <Button type="button" variant="outline" className="sm:mr-auto" disabled={busy} onClick={() => void remove()}>
              Delete
            </Button>
          ) : null}
          <Button type="button" variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button type="button" disabled={!valid || !nameOk || busy} onClick={() => void save()}>
            {existing ? "Save" : "Create"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
