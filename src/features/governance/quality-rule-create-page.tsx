"use client"

import * as React from "react"
import { useRouter } from "next/navigation"
import { FormPage } from "@/components/patterns/form-page"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useServiceAction } from "@/hooks/use-service"
import { notifyError, withNotify } from "@/lib/notify"
import type { Severity } from "@/lib/status"
import { governanceService } from "@/services"
import { SEVERITY_OPTIONS } from "./data-quality-columns"
import { CheckFields, ThresholdForms } from "./quality-check-fields"
import { EMPTY_FORM, KINDS, ruleThreshold, type RuleForm } from "./quality-checks"
import { TablePicker, useTableCatalog, useTableColumns } from "./table-picker"

const selectClassName = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

/**
 * `/governance/data-quality/rules/new` (`WEB-7` D2): the add-rule form,
 * moved out of the `CreateSheet` side panel onto its own page, with every
 * field it had.
 *
 * `DATA-14 (F7)` is unchanged: a check is picked by default, so the
 * threshold is always one the API can run; the typed form stays for what a
 * picker cannot say. `DATA-14 (AC1)` is unchanged too: the new rule is run
 * once straight away, so a check that already fails opens its incident at
 * once, and the rule stays whatever that run says.
 */
export function QualityRuleCreatePage() {
  const router = useRouter()
  const backHref = "/governance/data-quality"
  const { hasPermission } = useAuth()
  const [name, setName] = React.useState("")
  const [asset, setAsset] = React.useState("")
  const [formDimension, setFormDimension] = React.useState("")
  const [threshold, setThreshold] = React.useState("")
  const [typedThreshold, setTypedThreshold] = React.useState(false)
  const [checkForm, setCheckForm] = React.useState<RuleForm>(EMPTY_FORM)
  const builtThreshold = ruleThreshold(checkForm)
  const finalThreshold = typedThreshold ? threshold.trim() : (builtThreshold ?? "")
  const finalDimension =
    formDimension.trim() || (typedThreshold ? "" : KINDS[checkForm.kind].dimension)
  const [severity, setSeverity] = React.useState<Severity>("medium")
  // WEB-8 D8: the table is picked from the catalog, and its columns come
  // from that table's own schema — so the check's column field is a list
  // too, instead of the free text it had while the table was typed.
  const catalog = useTableCatalog()
  const ruleColumns = useTableColumns(catalog.idOf(asset))

  const create = useServiceAction(
    withNotify(
      {
        success: "Quality rule created",
        error: "Failed to create quality rule",
      },
      (signal, input: Parameters<typeof governanceService.createQualityRule>[0]) =>
        governanceService.createQualityRule(input, signal)
    )
  )

  async function handleCreate() {
    const result = await create.run({
      name: name.trim(),
      asset: asset.trim(),
      dimension: finalDimension,
      threshold: finalThreshold,
      severity,
    })
    if (!result) return
    if (hasPermission("query:read") && result.evaluable !== false) {
      try {
        await governanceService.runQualityRule(result.id)
      } catch (err) {
        notifyError("The rule was added, but its first run failed", err)
      }
    }
    router.push(backHref)
  }

  return (
    <FormPage
      title="Add Quality Rule"
      description="Define a quality check with dimension, threshold, and severity."
      backHref={backHref}
      backLabel="Back to Data Quality"
      submitLabel="Add rule"
      submittingLabel="Creating…"
      canSubmit={Boolean(name.trim() && asset.trim() && finalDimension && finalThreshold)}
      submitting={create.status === "pending"}
      error={create.status === "error" ? create.error.message : null}
      onSubmit={() => void handleCreate()}
      onCancel={() => router.push(backHref)}
    >
      <div className="space-y-1.5">
        <Label htmlFor="qr-name">Name</Label>
        <Input id="qr-name" value={name} onChange={(e) => setName(e.target.value)} />
      </div>
      <TablePicker id="qr-asset" label="Asset" value={asset} onChange={setAsset} catalog={catalog} />
      {typedThreshold ? (
        <div className="space-y-1.5">
          <Label htmlFor="qr-threshold">Threshold</Label>
          <Input
            id="qr-threshold"
            className="font-mono"
            value={threshold}
            onChange={(e) => setThreshold(e.target.value)}
            placeholder="email not null >= 99%"
          />
          <p className="text-xs text-muted-foreground">
            To be runnable, write it as one of: <ThresholdForms />. Name the asset as
            silver.table, serving.table or bronze.table.
          </p>
        </div>
      ) : (
        <div className="grid gap-3">
          <CheckFields
            form={checkForm}
            onChange={(patch) => setCheckForm((f) => ({ ...f, ...patch }))}
            columns={ruleColumns}
            idPrefix="qr"
          />
          <p className="text-xs text-muted-foreground">
            {builtThreshold ? (
              <>
                Threshold: <span className="font-mono text-foreground">{builtThreshold}</span>
              </>
            ) : (
              "Fill in the check to see its threshold."
            )}
          </p>
        </div>
      )}
      <Button
        type="button"
        variant="link"
        size="sm"
        className="self-start px-0"
        onClick={() => setTypedThreshold((t) => !t)}
      >
        {typedThreshold ? "Pick a check instead" : "Write the threshold myself"}
      </Button>
      <div className="space-y-1.5">
        <Label htmlFor="qr-dim">Dimension</Label>
        <Input
          id="qr-dim"
          value={formDimension}
          onChange={(e) => setFormDimension(e.target.value)}
          placeholder={typedThreshold ? "completeness" : KINDS[checkForm.kind].dimension}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="qr-severity">Severity</Label>
        <select
          id="qr-severity"
          className={selectClassName}
          value={severity}
          onChange={(e) => setSeverity(e.target.value as Severity)}
        >
          {SEVERITY_OPTIONS.map((o) => (
            <option key={o.value} value={o.value}>
              {o.label}
            </option>
          ))}
        </select>
      </div>
    </FormPage>
  )
}
