"use client"

import * as React from "react"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogClose,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { SEVERITY_LABEL, type Severity } from "@/lib/status"
import { governanceService } from "@/services"
import type { QualityRule, UpdateQualityRuleInput } from "@/services/contracts/governance"

const selectClassName = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

/** What the dialog needs of a rule — a list row and an asset's check both have it. */
export type EditableRule = {
  id: string
  name: string
  asset: string
  threshold: string
  severity: string
}

const isSeverity = (s: string): s is Severity => s in SEVERITY_LABEL

/**
 * The rewrite the form describes, or `null` while it leaves the table or
 * the threshold empty, or changes nothing.
 */
export function ruleRewrite(
  rule: EditableRule,
  form: { asset: string; threshold: string; severity: Severity }
): UpdateQualityRuleInput | null {
  const [asset, threshold] = [form.asset.trim(), form.threshold.trim()]
  if (!asset || !threshold) return null
  if (asset === rule.asset && threshold === rule.threshold && form.severity === rule.severity) {
    return null
  }
  return { asset, threshold, severity: form.severity }
}

/**
 * Rewrites an authored quality rule in place — the table it checks, its
 * threshold, its severity — which is how a rule the evaluator cannot read
 * becomes one it can. The name stays: it is what the rule is known by.
 */
export function EditRuleDialog({
  rule,
  onClose,
  onSaved,
}: {
  rule: EditableRule
  onClose: () => void
  /** The rule as it now reads, with whether it can be run. */
  onSaved: (rule: QualityRule) => void
}) {
  const [asset, setAsset] = React.useState(rule.asset)
  const [threshold, setThreshold] = React.useState(rule.threshold)
  const [severity, setSeverity] = React.useState<Severity>(
    isSeverity(rule.severity) ? rule.severity : "medium"
  )
  const save = useServiceAction((signal, input: UpdateQualityRuleInput) =>
    governanceService.updateQualityRule(rule.id, input, signal)
  )
  const rewrite = ruleRewrite(rule, { asset, threshold, severity })
  // Results describe what the rule checked; a different check has none yet.
  const clearsResults =
    asset.trim() !== rule.asset || threshold.trim() !== rule.threshold

  async function submit() {
    if (!rewrite) return
    const saved = await save.run(rewrite)
    if (saved === null) return
    notifySuccess(
      saved.evaluable === false
        ? "Rule saved — it still can't be run as written"
        : "Rule saved"
    )
    onSaved(saved)
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Edit quality rule</DialogTitle>
          <DialogDescription>
            What <span className="font-mono">{rule.name}</span> checks, and how much a failure
            matters.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="edit-rule-asset">Table</Label>
            <Input
              id="edit-rule-asset"
              className="font-mono"
              value={asset}
              onChange={(e) => setAsset(e.target.value)}
              placeholder="silver.orders"
            />
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="edit-rule-threshold">Threshold</Label>
            <Input
              id="edit-rule-threshold"
              className="font-mono"
              value={threshold}
              onChange={(e) => setThreshold(e.target.value)}
              placeholder="email not null >= 95%"
            />
            <p className="text-xs text-muted-foreground">
              To be runnable, write it as: <span className="font-mono">rows &gt;= 1000</span>,{" "}
              <span className="font-mono">column not null &gt;= 95%</span>,{" "}
              <span className="font-mono">column unique</span> or{" "}
              <span className="font-mono">column between 0 and 100</span>, and name the table as
              silver.table, serving.table or bronze.table.
            </p>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="edit-rule-severity">Severity</Label>
            <select
              id="edit-rule-severity"
              className={selectClassName}
              value={severity}
              onChange={(e) => setSeverity(e.target.value as Severity)}
            >
              {(Object.keys(SEVERITY_LABEL) as Severity[]).map((s) => (
                <option key={s} value={s}>
                  {SEVERITY_LABEL[s]}
                </option>
              ))}
            </select>
          </div>
          {clearsResults ? (
            <p className="text-xs text-muted-foreground">
              The results recorded for this rule are cleared: they describe what it checked
              before. It reads &quot;not run yet&quot; until it runs again.
            </p>
          ) : null}
          {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={!rewrite || save.status === "pending"}
          >
            {save.status === "pending" ? "Saving…" : "Save rule"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
