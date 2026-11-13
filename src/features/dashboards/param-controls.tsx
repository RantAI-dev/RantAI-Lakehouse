"use client";

import * as React from "react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from "@/components/ui/select";
import { paramHint, type ParamControl, type ParamValue } from "@/lib/params";

/**
 * One control per parameter the dashboard shows (`BI-11`), in the filter row.
 * A list is a select; a number, a text and a date are inputs that report a
 * change when the viewer leaves the field or presses Enter, so typing does not
 * reload every tile on each key. A value the server refuses is listed under
 * that parameter's control and the control returns to the value in force
 * (`BI-52`, plan D28); a refusal for a parameter the dashboard does not show is
 * listed after the controls.
 */
export function ParamControls({
  controls, onChange, refusals,
}: {
  readonly controls: readonly ParamControl[];
  readonly onChange: (name: string, value: ParamValue) => void;
  /** The sentence for each parameter whose last requested value did not fit (`BI-52`). */
  readonly refusals?: Readonly<Record<string, string>>;
}) {
  const shown = new Set(controls.map((c) => c.param.name));
  const orphans = Object.entries(refusals ?? {}).filter(([name]) => !shown.has(name));
  return (
    <>
      {controls.map((c) => (
        <ParamField key={c.param.name} control={c} refusal={refusals?.[c.param.name]} onChange={onChange} />
      ))}
      {orphans.map(([name, reason]) => (
        <p key={name} role="alert" className="text-xs text-destructive">{reason}</p>
      ))}
    </>
  );
}

function ParamField({ control, refusal, onChange }: {
  control: ParamControl;
  refusal?: string;
  onChange: (name: string, value: ParamValue) => void;
}) {
  const { param, value } = control;
  const [typed, setTyped] = React.useState(String(value));
  // The refusal is also a dependency: the value in force comes back with it.
  React.useEffect(() => { setTyped(String(value)); }, [value, refusal]);
  const hint = paramHint(param);
  const commit = () => {
    if (typed === String(value) || typed.trim() === "") { setTyped(String(value)); return; }
    onChange(param.name, param.type === "number" ? Number(typed) : typed);
  };
  const id = `param-${param.name}`;
  return (
    <div className="space-y-1" data-param={param.name}>
      <div className="flex items-center gap-2">
        <Label htmlFor={id} className="text-xs text-muted-foreground">{param.label || param.name}</Label>
        {param.type === "list" ? (
          <Select
            value={String(value)}
            items={Object.fromEntries((param.values ?? []).map((v) => [String(v.value), v.label]))}
            onValueChange={(v) => { if (v) onChange(param.name, (param.values ?? []).find((e) => String(e.value) === v)?.value ?? v); }}
          >
            <SelectTrigger id={id} size="sm" className="w-40" aria-label={param.label || param.name}><SelectValue /></SelectTrigger>
            <SelectContent>
              {(param.values ?? []).map((v) => <SelectItem key={String(v.value)} value={String(v.value)}>{v.label}</SelectItem>)}
            </SelectContent>
          </Select>
        ) : (
          <Input
            id={id}
            className="h-8 w-36"
            type={param.type === "number" ? "number" : param.type === "date" ? "date" : "text"}
            value={typed}
            min={param.type === "number" || param.type === "date" ? param.min : undefined}
            max={param.type === "number" || param.type === "date" ? param.max : undefined}
            maxLength={param.type === "text" ? param.maxLength : undefined}
            title={hint || undefined}
            onChange={(e) => setTyped(e.target.value)}
            onBlur={commit}
            onKeyDown={(e) => { if (e.key === "Enter") commit(); }}
          />
        )}
      </div>
      {refusal ? <p role="alert" className="text-xs text-destructive">{refusal}</p> : null}
    </div>
  );
}
