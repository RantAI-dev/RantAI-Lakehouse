"use client";

import * as React from "react";
import { Input } from "@/components/ui/input";
import { Popover, PopoverContent, PopoverTrigger } from "@/components/ui/popover";
import { paramHint, type ParamControl, type ParamValue } from "@/lib/params";
import { ChipPanelLabel, ChipTrigger, ChoiceRow, chipShell } from "./filters/chip";

/**
 * One chip per parameter the dashboard shows (`BI-11`), in the filter row
 * (`BI-61` T3: "Top berapa 5 ⌄"). The chip opens a small panel with a
 * labelled, full-width control: a list is a column of choices; a number, a
 * text and a date are inputs that report a change when the viewer leaves the
 * field or presses Enter, so typing does not reload every tile on each key.
 * A chip whose value differs from the saved one is tinted.
 *
 * A value the server refuses is said beside that parameter's chip, which
 * turns red, and the control returns to the value in force (`BI-52`, QA
 * `LH-BI-11-05`, plan D28); a refusal for a parameter the dashboard does not
 * show is listed after the chips.
 */
export function ParamControls({
  controls, onChange, refusals, defaults,
}: {
  readonly controls: readonly ParamControl[];
  readonly onChange: (name: string, value: ParamValue) => void;
  /** The sentence for each parameter whose last requested value did not fit (`BI-52`). */
  readonly refusals?: Readonly<Record<string, string>>;
  /** The values the dashboard saved (or each parameter's default); a chip that differs is tinted. Absent: none is tinted. */
  readonly defaults?: Readonly<Record<string, ParamValue>>;
}) {
  const shown = new Set(controls.map((c) => c.param.name));
  const orphans = Object.entries(refusals ?? {}).filter(([name]) => !shown.has(name));
  return (
    <>
      {controls.map((c) => (
        <ParamField key={c.param.name} control={c} refusal={refusals?.[c.param.name]} onChange={onChange}
          changed={defaults !== undefined && c.param.name in defaults && String(defaults[c.param.name]) !== String(c.value)} />
      ))}
      {orphans.map(([name, reason]) => (
        <p key={name} role="alert" className="shrink-0 text-xs text-destructive">{reason}</p>
      ))}
    </>
  );
}

function ParamField({ control, refusal, onChange, changed }: {
  control: ParamControl;
  refusal?: string;
  onChange: (name: string, value: ParamValue) => void;
  changed: boolean;
}) {
  const { param, value } = control;
  const [open, setOpen] = React.useState(false);
  const [typed, setTyped] = React.useState(String(value));
  // What was last reported, so Enter, the blur and the panel closing report one change once.
  const sent = React.useRef<string | null>(null);
  // The refusal is also a dependency: the value in force comes back with it.
  React.useEffect(() => { setTyped(String(value)); sent.current = null; }, [value, refusal]);
  const hint = paramHint(param);
  const commit = () => {
    if (typed === String(value) || typed.trim() === "") { setTyped(String(value)); return; }
    if (sent.current === typed) return;
    sent.current = typed;
    onChange(param.name, param.type === "number" ? Number(typed) : typed);
  };
  const id = `param-${param.name}`;
  const name = param.label || param.name;
  const shownValue = param.type === "list"
    ? (param.values ?? []).find((v) => String(v.value) === String(value))?.label ?? String(value)
    : String(value);
  const tone = refusal ? "refused" : changed ? "changed" : "default";
  return (
    <span className="inline-flex shrink-0 items-center gap-2" data-param={param.name}>
      {/* Closing the panel keeps what was typed, as leaving the field does. */}
      <Popover open={open} onOpenChange={(next) => { if (!next && param.type !== "list") commit(); setOpen(next); }}>
        <span className={chipShell(tone)}>
          <PopoverTrigger asChild>
            <ChipTrigger label={name} value={shownValue} tone={tone} />
          </PopoverTrigger>
        </span>
        <PopoverContent align="start" collisionPadding={16} className="w-64 gap-2 p-3">
          <ChipPanelLabel htmlFor={param.type === "list" ? undefined : id}>{name}</ChipPanelLabel>
          {param.type === "list" ? (
            <div role="radiogroup" aria-label={name} className="-mx-1 grid gap-0.5">
              {(param.values ?? []).map((v) => (
                <ChoiceRow key={String(v.value)} label={v.label} on={String(v.value) === String(value)}
                  onClick={() => { onChange(param.name, v.value); setOpen(false); }} />
              ))}
            </div>
          ) : (
            <Input
              id={id}
              className="w-full"
              type={param.type === "number" ? "number" : param.type === "date" ? "date" : "text"}
              value={typed}
              min={param.type === "number" || param.type === "date" ? param.min : undefined}
              max={param.type === "number" || param.type === "date" ? param.max : undefined}
              maxLength={param.type === "text" ? param.maxLength : undefined}
              onChange={(e) => setTyped(e.target.value)}
              onBlur={commit}
              onKeyDown={(e) => { if (e.key === "Enter") { commit(); setOpen(false); } }}
            />
          )}
          {hint ? <p className="text-xs text-muted-foreground">{hint}</p> : null}
        </PopoverContent>
      </Popover>
      {refusal ? <span role="alert" className="max-w-72 text-xs text-destructive">{refusal}</span> : null}
    </span>
  );
}
