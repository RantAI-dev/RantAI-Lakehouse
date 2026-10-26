"use client";

import * as React from "react";
import { Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useServiceAction } from "@/hooks/use-service";
import { readableErrorMessage } from "@/lib/notify-message";
import { notifySuccess } from "@/lib/notify";
import { alertRuleService } from "@/services";
import type { AlertCondition } from "@/services/contracts/alerts";
import { alertNumberShape } from "./chart-alert";

const selectClass = "h-8 rounded-md border border-input bg-background px-2 text-sm";

/** What the dialog says about who an alert is read as (feature decision C4). */
export const READ_AS_YOU_SENTENCE =
  "The alert reads the chart as you, with your masking and row filters. If you lose access to it, the alert stops and says why.";

/** Without the alert permission the target is fixed (feature decision C3). */
export const OWN_ADDRESS_SENTENCE =
  "Without the alert permission, an alert can only be sent to your own email address.";

export type AlertChart = {
  readonly id: string;
  readonly title: string;
  readonly kind: string;
  /** The value columns of the chart, the choices for "series". */
  readonly measures: readonly string[];
};

/**
 * A chart's ⋯ > "Alert me…": a condition on the chart's number, the line, the
 * series (for a line or bar), the dashboard filters now on screen, and where to
 * send it. Sends once when the value crosses the line and once when it is back.
 */
export function ChartAlertDialog({
  open, onOpenChange, chart, filters, userEmail, canWriteAlerts,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  readonly chart: AlertChart;
  /** The filters the page is showing now; the alert is read with the same. */
  readonly filters: readonly unknown[];
  readonly userEmail: string | null;
  /** `alert:write`: may send to other addresses and to webhooks. */
  readonly canWriteAlerts: boolean;
}) {
  const shape = alertNumberShape(chart.kind);
  const conditions: AlertCondition[] = shape === "none" ? ["any_rows"] : ["above", "below", "any_rows"];
  const [condition, setCondition] = React.useState<AlertCondition>(conditions[0]);
  const [threshold, setThreshold] = React.useState("");
  const [series, setSeries] = React.useState(chart.measures[0] ?? "");
  const [target, setTarget] = React.useState(userEmail ?? "");
  const [name, setName] = React.useState(`Alert: ${chart.title}`);
  const [problem, setProblem] = React.useState<string | null>(null);
  const save = useServiceAction((signal, input: Parameters<typeof alertRuleService.createRule>[0]) =>
    alertRuleService.createRule(input, signal));

  const needsLine = condition !== "any_rows";
  const isWebhook = /^https?:\/\//i.test(target.trim());

  async function submit() {
    setProblem(null);
    if (needsLine && (threshold.trim() === "" || !Number.isFinite(Number(threshold)))) {
      setProblem("Enter the line as a number.");
      return;
    }
    if (!target.trim()) {
      setProblem("Enter where to send it.");
      return;
    }
    const done = await save.run({
      name: name.trim() || `Alert: ${chart.title}`,
      type: "alert",
      chart: chart.id,
      condition,
      ...(needsLine ? { threshold: Number(threshold) } : {}),
      ...(shape === "series" && needsLine ? { series } : {}),
      ...(filters.length ? { filters: [...filters] } : {}),
      channel: isWebhook ? "webhook" : "email",
      target: target.trim(),
    });
    if (done) {
      notifySuccess("Alert saved", "It is sent once when the value crosses the line, and once when it is back.");
      onOpenChange(false);
    }
  }

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader><DialogTitle>Alert me · {chart.title}</DialogTitle></DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="chart-alert-name">Name</Label>
            <Input id="chart-alert-name" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="chart-alert-condition">When</Label>
            <select id="chart-alert-condition" className={selectClass} value={condition}
              onChange={(e) => setCondition(e.target.value as AlertCondition)}>
              {conditions.map((c) => (
                <option key={c} value={c}>
                  {c === "above" ? "the value goes above" : c === "below" ? "the value goes below" : "the chart returns any rows"}
                </option>
              ))}
            </select>
            {shape === "none" ? (
              <p className="text-xs text-muted-foreground">This chart has no single number, so an alert on it can only ask whether it returns any rows.</p>
            ) : null}
          </div>
          {needsLine ? (
            <div className="grid gap-1.5">
              <Label htmlFor="chart-alert-line">Line</Label>
              <Input id="chart-alert-line" inputMode="decimal" value={threshold} onChange={(e) => setThreshold(e.target.value)} />
            </div>
          ) : null}
          {shape === "series" && needsLine ? (
            <div className="grid gap-1.5">
              <Label htmlFor="chart-alert-series">Series (its latest point is read)</Label>
              <select id="chart-alert-series" className={selectClass} value={series} onChange={(e) => setSeries(e.target.value)}>
                {chart.measures.map((m) => <option key={m} value={m}>{m}</option>)}
              </select>
            </div>
          ) : null}
          <p className="text-xs text-muted-foreground">
            {filters.length
              ? `Read with the ${filters.length} dashboard filter${filters.length === 1 ? "" : "s"} on screen now.`
              : "Read with no dashboard filters."}
          </p>
          <div className="grid gap-1.5">
            <Label htmlFor="chart-alert-target">Send to</Label>
            <Input id="chart-alert-target" value={target} onChange={(e) => setTarget(e.target.value)}
              readOnly={!canWriteAlerts} placeholder={canWriteAlerts ? "Email address or webhook URL" : undefined} />
            {!canWriteAlerts ? <p className="text-xs text-muted-foreground">{OWN_ADDRESS_SENTENCE}</p> : null}
          </div>
          <p className="text-xs text-muted-foreground">{READ_AS_YOU_SENTENCE}</p>
          {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
          {save.status === "error" ? <p role="alert" className="text-sm text-destructive">{readableErrorMessage(save.error.message)}</p> : null}
        </div>
        <DialogFooter>
          <Button variant="outline" onClick={() => onOpenChange(false)}>Cancel</Button>
          <Button onClick={() => void submit()} disabled={save.status === "pending"}>
            {save.status === "pending" ? <Loader2 className="size-4 animate-spin" /> : null}Save alert
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
