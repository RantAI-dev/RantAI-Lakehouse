"use client";

import * as React from "react";
import { BellRing, Loader2, Trash2 } from "lucide-react";
import { FormField, FormGroup, FormHint } from "@/components/patterns/form-fields";
import { HelpHint } from "@/components/patterns/help-hint";
import { SelectField } from "@/components/patterns/select-field";
import { Button } from "@/components/ui/button";
import { Dialog, DialogContent, DialogFooter, DialogHeader, DialogTitle } from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/components/ui/tooltip";
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog";
import { useService, useServiceAction } from "@/hooks/use-service";
import { readableErrorMessage } from "@/lib/notify-message";
import { notifySuccess } from "@/lib/notify";
import { formatNumber } from "@/lib/format";
import { cn } from "@/lib/utils";
import { alertRuleService } from "@/services";
import type { AlertCondition, AlertRule } from "@/services/contracts/alerts";
import { displayColumn } from "@/lib/chart-joins";
import { OTHER_FLAG } from "@/lib/chart-shape";
import { alertNumberShape } from "./chart-alert";

const CONDITION_LABELS: Record<AlertCondition, string> = {
  above: "the value goes above",
  below: "the value goes below",
  any_rows: "the chart returns any rows",
};

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
  /**
   * Whether the chart has a latest point to compare (not sorted by value).
   * False offers only "returns any rows" (BI-19 owner QA 2026-10-28, QA-6);
   * the server's save-time check stays the authority. Absent means true.
   */
  readonly latestPoint?: boolean;
  /** Result column to the legend's label ("sum of visitors"); a column not in it reads as itself (QA-7). */
  readonly labels?: Readonly<Record<string, string>>;
};

/** The sentence for a chart with no single number (a pie, a table). */
export const NO_SINGLE_NUMBER_SENTENCE =
  "This chart has no single number, so an alert on it can only ask whether it returns any rows.";

/** The sentence for a chart sorted by value. */
export const NO_LATEST_POINT_SENTENCE =
  "This chart is sorted by value, so it has no latest point to compare; it can only alert when it returns rows.";

/** A webhook URL's host, for a row with no room for the whole URL. */
function urlHost(url: string): string {
  try {
    return new URL(url).host || url;
  } catch {
    return url;
  }
}

/**
 * One line about a saved alert: what it watches for, and where it sends when
 * that is not the person's own address (owner QA 2026-11-16: their own email
 * on every row said nothing they did not know).
 */
function alertSummary(rule: AlertRule, ownAddress: string | null): string {
  const when =
    rule.condition === "any_rows" ? "returns any rows"
    : rule.condition === "below" ? `below ${formatNumber(rule.threshold)}`
    : `above ${formatNumber(rule.threshold)}`;
  const target = rule.target ?? "";
  if (!target || target.toLowerCase() === (ownAddress ?? "").toLowerCase()) return when;
  // A webhook reads as its host; the whole URL is in the row's title.
  return `${when} · to ${rule.channel === "webhook" ? urlHost(target) : target}`;
}

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
  const sortedByValue = shape === "series" && chart.latestPoint === false;
  const conditions: AlertCondition[] = shape === "none" || sortedByValue ? ["any_rows"] : ["above", "below", "any_rows"];
  // QA-7: the top-N "Other" bucket's flag is not a series; the rest read as the legend does.
  const seriesChoices = chart.measures.filter((m) => m !== OTHER_FLAG);
  const [condition, setCondition] = React.useState<AlertCondition>(conditions[0]);
  const [threshold, setThreshold] = React.useState("");
  const [series, setSeries] = React.useState(seriesChoices[0] ?? "");
  const [target, setTarget] = React.useState(userEmail ?? "");
  const [name, setName] = React.useState(`Alert: ${chart.title}`);
  const [problem, setProblem] = React.useState<string | null>(null);
  const save = useServiceAction((signal, input: Parameters<typeof alertRuleService.createRule>[0]) =>
    alertRuleService.createRule(input, signal));

  // BI-43 D1: the person's own alerts on this chart, with remove. A failed
  // read leaves the section out; it never blocks saving a new alert.
  const mine = useService((signal) => alertRuleService.listMine(chart.id, signal), [chart.id]);
  const [removing, setRemoving] = React.useState<AlertRule | null>(null);
  const remove = useServiceAction((signal, id: string) => alertRuleService.deleteRule(id, signal));
  const [removeProblem, setRemoveProblem] = React.useState<string | null>(null);
  const ownAlerts = mine.status === "success" ? mine.data : [];

  async function confirmRemove() {
    if (!removing) return;
    setRemoveProblem(null);
    const done = await remove.run(removing.id);
    if (done !== null) {
      notifySuccess("Alert removed");
      setRemoving(null);
      mine.reload();
    } else {
      setRemoving(null);
      setRemoveProblem("The alert could not be removed. Try again.");
    }
  }

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

  // Owner QA 2026-11-16: the read-as-you rule and the filters are one quiet
  // line above the button, its detail behind the ⓘ.
  const readLine = filters.length
    ? `Read as you, with the ${filters.length} dashboard filter${filters.length === 1 ? "" : "s"} on screen now`
    : "Read as you";
  const onlyAnyRows = shape === "none" ? NO_SINGLE_NUMBER_SENTENCE : sortedByValue ? NO_LATEST_POINT_SENTENCE : null;

  return (
    <>
    <Dialog open={open} onOpenChange={onOpenChange}>
      {/*
        Owner QA 2026-11-16: the content ran past the dialog's right edge. The
        old body was a CSS grid with an implicit `auto` column, which grows to
        the widest unbreakable line (a truncated alert row is `nowrap`); every
        container here is a flex column that keeps the dialog's width and lets
        a row shrink (`min-w-0`).
      */}
      <DialogContent className="max-h-[85vh] gap-5 overflow-y-auto sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="truncate leading-snug" title={`Alert me · ${chart.title}`}>Alert me · {chart.title}</DialogTitle>
        </DialogHeader>
        <div className="flex min-w-0 flex-col gap-5">
          {ownAlerts.length > 0 ? (
            <FormGroup title="Your alerts on this chart" className="gap-1.5">
              <ul className="-mx-2 flex min-w-0 flex-col">
                {ownAlerts.map((rule) => {
                  const summary = alertSummary(rule, userEmail);
                  return (
                    <li key={rule.id} className="flex h-9 min-w-0 items-center gap-2.5 rounded-lg pr-1 pl-2 transition-colors duration-150 ease-out hover:bg-muted/60 motion-reduce:transition-none">
                      <BellRing className="size-3.5 shrink-0 text-muted-foreground" aria-hidden />
                      {/* The name gives way first; the condition keeps up to two thirds of the row. */}
                      <span className="flex min-w-0 flex-1 text-sm" title={rule.channel === "webhook" && rule.target ? `${rule.name} · ${summary} (${rule.target})` : `${rule.name} · ${summary}`}>
                        <span className="min-w-0 truncate">{rule.name}</span>
                        <span className="max-w-[65%] shrink-0 truncate pl-1 text-muted-foreground">· {summary}</span>
                      </span>
                      <Tooltip>
                        <TooltipTrigger
                          render={
                            <Button variant="ghost" size="icon-xs" aria-label={`Remove ${rule.name}`}
                              className="text-muted-foreground hover:text-destructive" onClick={() => setRemoving(rule)} />
                          }
                        >
                          <Trash2 aria-hidden />
                        </TooltipTrigger>
                        <TooltipContent>Remove alert</TooltipContent>
                      </Tooltip>
                    </li>
                  );
                })}
              </ul>
              {removeProblem ? <p role="alert" className="text-xs text-destructive">{removeProblem}</p> : null}
            </FormGroup>
          ) : null}
          <FormGroup title={ownAlerts.length > 0 ? "New alert" : undefined} className="gap-4">
            <FormField id="chart-alert-name" label="Name">
              <Input id="chart-alert-name" value={name} onChange={(e) => setName(e.target.value)} />
            </FormField>
            <FormField
              id="chart-alert-condition"
              label="When"
              hint={onlyAnyRows ? <>Only “returns any rows” for this chart <HelpHint>{onlyAnyRows}</HelpHint></> : null}
            >
              <SelectField
                id="chart-alert-condition"
                className="w-full"
                value={condition}
                options={conditions.map((c) => ({ value: c, label: CONDITION_LABELS[c] }))}
                onValueChange={(next) => setCondition(next as AlertCondition)}
              />
            </FormField>
            {needsLine ? (
              <FormField id="chart-alert-line" label="Line">
                <Input id="chart-alert-line" inputMode="decimal" value={threshold} onChange={(e) => setThreshold(e.target.value)} />
              </FormField>
            ) : null}
            {shape === "series" && needsLine ? (
              <FormField id="chart-alert-series" label="Series" hint="Its latest point is the value the alert reads.">
                <SelectField
                  id="chart-alert-series"
                  className="w-full"
                  value={series}
                  placeholder="Pick a series"
                  options={seriesChoices.map((m) => ({ value: m, label: chart.labels?.[m] ?? displayColumn(m) }))}
                  onValueChange={setSeries}
                />
              </FormField>
            ) : null}
            <FormField
              id="chart-alert-target"
              label="Send to"
              hint={!canWriteAlerts ? <>Only your own address <HelpHint>{OWN_ADDRESS_SENTENCE}</HelpHint></> : null}
            >
              <Input id="chart-alert-target" value={target} onChange={(e) => setTarget(e.target.value)}
                readOnly={!canWriteAlerts} placeholder={canWriteAlerts ? "Email address or webhook URL" : undefined}
                className={cn(!canWriteAlerts && "bg-muted/50 text-muted-foreground focus-visible:ring-0")} />
            </FormField>
          </FormGroup>
          {problem ? <p role="alert" className="text-sm text-destructive">{problem}</p> : null}
          {save.status === "error" ? <p role="alert" className="text-sm text-destructive">{readableErrorMessage(save.error.message)}</p> : null}
        </div>
        <DialogFooter className="flex-col gap-3 sm:flex-row sm:items-center sm:justify-between">
          <FormHint>
            <span className="min-w-0 truncate">{readLine}</span> <HelpHint>{READ_AS_YOU_SENTENCE}</HelpHint>
          </FormHint>
          <Button onClick={() => void submit()} disabled={save.status === "pending"} className="w-full sm:w-auto">
            {save.status === "pending" ? <Loader2 className="size-4 animate-spin" aria-hidden /> : null}Save alert
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
      <ConfirmActionDialog
        open={removing !== null}
        onOpenChange={(o) => { if (!o) setRemoving(null); }}
        title="Remove alert?"
        description={`"${removing?.name ?? ""}" stops watching this chart and sends nothing more.`}
        confirmLabel="Remove alert"
        destructive
        confirming={remove.status === "pending"}
        onConfirm={() => void confirmRemove()}
      />
    </>
  );
}
