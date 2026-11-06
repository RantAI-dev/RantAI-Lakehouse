"use client";

import { CheckCircle, XCircle } from "lucide-react";
import { savedTitle } from "./use-copilot";
import { asObj, toolLabel, type ToolStep } from "./tool-step";

/**
 * What the console knows about a draft the person confirmed, or null when
 * it has nothing to add. The model writes its reply before the person
 * confirms, so the reply still says "confirm it to save it" next to a
 * saved object; this line is the console's own word on the outcome and
 * the model's text is left as written.
 *
 * A saved step (`confirmed` and `ok`) says `Saved: <title>`, with the
 * result's title or name and the tool's label when the result has none.
 * A confirm that failed is stored with `confirmed: false`, and says
 * `Not saved: <error>`. A cancelled step has no `confirmed` at all and a
 * draft still waiting has none either, so both say nothing.
 */
export function confirmedStatus(step: ToolStep): { saved: boolean; text: string } | null {
  const res = asObj(step.result);
  if (res.needs_confirmation) return null;
  if (step.confirmed === true && step.ok) {
    return { saved: true, text: `Saved: ${savedTitle(step.result) ?? toolLabel(step.tool)}` };
  }
  if (step.confirmed === false) {
    const error = typeof res.error === "string" && res.error.trim() ? res.error.trim() : null;
    return { saved: false, text: error ? `Not saved: ${error}` : "Not saved" };
  }
  return null;
}

/**
 * One status line per confirmed step of an answer, in step order. It is
 * drawn from the stored steps at render and is not part of the message
 * text, so the history the console sends to the model never carries it.
 */
export function SavedStatusLines({ tools }: { readonly tools: readonly ToolStep[] }) {
  const lines = tools.flatMap((step, i) => {
    const status = confirmedStatus(step);
    return status ? [{ key: i, ...status }] : [];
  });
  if (lines.length === 0) return null;
  return (
    <div className="mt-2 space-y-0.5">
      {lines.map((line) => (
        <p
          key={line.key}
          role="status"
          className={
            line.saved
              ? "flex items-center gap-1.5 text-xs text-muted-foreground"
              : "flex items-center gap-1.5 text-xs text-destructive"
          }
        >
          {line.saved ? (
            <CheckCircle className="size-3 shrink-0 text-emerald-500" aria-hidden />
          ) : (
            <XCircle className="size-3 shrink-0 text-destructive" aria-hidden />
          )}
          {line.text}
        </p>
      ))}
    </div>
  );
}
