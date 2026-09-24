"use client";

import { AlertCircle } from "lucide-react";
import { Button } from "@/components/ui/button";
import { ChartDraftCard } from "./chart-draft-card";
import { asObj, type ToolStep } from "./tool-step";

/**
 * The Confirm/Cancel card for a tool step the server held back with
 * `needs_confirmation`. A chart draft renders as the chart it would create;
 * every other action shows the server's own summary of what will happen.
 */
export function PendingActionCard({
  step,
  confirming,
  onConfirm,
  onCancel,
  onSavedInBuilder,
}: {
  readonly step: ToolStep;
  readonly confirming: boolean;
  readonly onConfirm: () => void;
  readonly onCancel: () => void;
  readonly onSavedInBuilder: (result: Record<string, unknown>) => void;
}) {
  const res = asObj(step.result);
  const args = asObj(res.args ?? step.args);
  const tool = typeof res.tool === "string" ? res.tool : step.tool;

  if (tool === "create_chart") {
    return (
      <ChartDraftCard
        args={args}
        confirming={confirming}
        onConfirm={onConfirm}
        onCancel={onCancel}
        onSavedInBuilder={(saved) =>
          onSavedInBuilder({
            created: true,
            title: saved.title,
            kind: saved.kind,
            mart: saved.mart,
            board: saved.board,
            via: "builder",
          })
        }
      />
    );
  }

  return (
    <div className="space-y-2 rounded-xl border border-amber-500/30 bg-amber-500/10 p-3 text-xs">
      <div className="flex items-center gap-1.5 font-medium text-amber-800 dark:text-amber-300">
        <AlertCircle className="size-4 shrink-0" />
        Action requires confirmation
      </div>
      <p className="leading-relaxed text-foreground">
        {typeof res.summary === "string" && res.summary ? res.summary : "Copilot wants to run this action."}
      </p>
      <div className="flex items-center gap-2 pt-1">
        <Button size="sm" onClick={onConfirm} disabled={confirming}>
          {confirming ? "Running…" : "Confirm"}
        </Button>
        <Button size="sm" variant="outline" onClick={onCancel} disabled={confirming}>
          Cancel
        </Button>
      </div>
    </div>
  );
}
