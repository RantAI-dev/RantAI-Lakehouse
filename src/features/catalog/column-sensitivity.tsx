"use client"

import * as React from "react"
import Link from "next/link"
import { ConfirmActionDialog } from "@/components/patterns/confirm-action-dialog"
import { SelectField } from "@/components/patterns/select-field"
import { ClassificationBadge } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useServiceAction } from "@/hooks/use-service"
import { columnMasks, sharedMaskSentence, type ColumnMask } from "@/lib/column-masks"
import { notifySuccess, withNotify } from "@/lib/notify"
import { CLASSIFICATION_LABEL, type Classification } from "@/lib/status"
import { governanceService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import { lineageKey } from "./asset-lineage"
import { policyCondition, policyTable } from "./asset-policy"

type PolicySummary = AssetDetail["policySummary"][number]

/** Least to most restrictive, as the Access tab offered them. */
export const SENSITIVITY_LEVELS: Classification[] = ["public", "internal", "confidential", "restricted"]

/** A row of the section: a label on the left, what it says and its controls beside it. */
function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="grid gap-1.5 py-2 sm:grid-cols-[8rem_minmax(0,1fr)] sm:gap-3">
      <div className="text-xs font-medium text-muted-foreground sm:pt-1.5">{label}</div>
      <div className="flex min-w-0 flex-col gap-2">{children}</div>
    </div>
  )
}

/**
 * How sensitive one column is (its classification rule), and for whom it is
 * masked (the enforced column-mask policies that name it), with the controls
 * to change both, in the opened column on the Schema tab (`WEB-3` S3-D1).
 * The two facts were split between the Schema tab's pills and the Access
 * tab's classification card and policy dialog; they now live with the column.
 *
 * - A classification is a rule keyed by the table's lineage key. Set creates
 *   one; Remove deletes the rule in force; Change deletes it and then
 *   creates the new one, in that order, and the page shows the new level
 *   only after both succeeded (the asset reloads; nothing is shown early).
 * - A mask is a policy keyed by the table its reads go to (`policyTable`),
 *   written with the Access tab's own `policyCondition`, so it is one the
 *   engine enforces. Remove deletes only a policy that masks this column and
 *   nothing else; a wider one is named and left to the Access tab.
 *
 * Set / Change / Remove need `governance:write`, the mask controls
 * `policy:write`, as the API demands; a reader sees the facts without them.
 * The roles a policy names are sent only with `policy:read`
 * (`AssetDetail.policySummary`), so without it the section says so instead
 * of saying the column is masked for no one.
 */
export function ColumnSensitivity({
  asset: a,
  column,
  onChanged,
}: {
  asset: AssetDetail
  column: string
  /** Reloads the asset after a classification or a mask changed. */
  onChanged: () => void
}) {
  const { hasPermission } = useAuth()
  const c = a.schema.find((s) => s.name === column)
  const masks = columnMasks(a.policySummary, column)
  return (
    <section
      aria-label={`Sensitivity and masking of ${column}`}
      className="rounded-lg border border-border bg-background px-3 py-1"
    >
      <h3 className="pt-2 text-sm font-semibold">Sensitivity and masking</h3>
      <div className="divide-y divide-border">
        <Row label="Sensitivity">
          <SensitivityControls
            asset={a}
            column={column}
            level={c?.classification}
            canWrite={hasPermission("governance:write")}
            onChanged={onChanged}
          />
        </Row>
        <Row label="Masked for">
          <MaskList
            asset={a}
            column={column}
            masks={masks}
            maskedForYou={c?.masked === true}
            canReadPolicies={hasPermission("policy:read")}
            canWrite={hasPermission("policy:write")}
            onChanged={onChanged}
          />
        </Row>
      </div>
    </section>
  )
}

/**
 * Set / Change / Remove for one classification rule: a column's, or, with no
 * `column`, the whole table's (`WEB-3` S3-T5a, review round 1 decision 1:
 * the table-level control moved to the About card, with the same calls and
 * no column). `level` is the level that rule gives, not the header's.
 */
export function SensitivityControls({
  asset: a,
  column,
  level,
  canWrite,
  onChanged,
}: {
  asset: AssetDetail
  /** Absent for the table-level rule. */
  column?: string
  level: Classification | undefined
  canWrite: boolean
  onChanged: () => void
}) {
  // The rule in force for this column (or, with no column, for the table:
  // the API sends a table rule without a `column` key). An older API build
  // sends no rules, and then the level is shown without a Change or Remove
  // that could not work.
  const rule = (a.classificationRules ?? []).find((r) => r.column === column)
  const what = column ? `column ${column}` : "the whole table"
  const fieldId = column ? `sensitivity-${column}` : "sensitivity-table"
  const [editing, setEditing] = React.useState(false)
  const [choice, setChoice] = React.useState<Classification>(level ?? "confidential")
  const [removing, setRemoving] = React.useState(false)
  // Whether a failed Change had already removed the old rule: then the
  // column may have no level now, and the message says both halves.
  const [oldRemoved, setOldRemoved] = React.useState(false)

  // One action for Set and Change: Change removes the rule in force first, so
  // the column keeps one rule, then adds the new one.
  const save = useServiceAction(async (signal, next: Classification) => {
    setOldRemoved(false)
    if (rule) {
      await governanceService.deleteClassificationRule(rule.id, signal)
      setOldRemoved(true)
    }
    return governanceService.createClassificationRule(
      { asset: lineageKey(a), ...(column ? { column } : {}), classification: next },
      signal
    )
  })
  const remove = useServiceAction(
    withNotify(
      { success: "Classification removed", error: "Failed to remove the classification" },
      (signal, id: string) => governanceService.deleteClassificationRule(id, signal)
    )
  )

  async function submit() {
    if ((await save.run(choice)) === null) {
      // After a failed Change the page reloads, so it shows what is stored
      // now (the old rule may be gone), not the old level.
      if (rule) onChanged()
      return
    }
    notifySuccess(
      column
        ? `Column ${column} is ${CLASSIFICATION_LABEL[choice]}`
        : `${a.name} as a whole is ${CLASSIFICATION_LABEL[choice]}`
    )
    setEditing(false)
    onChanged()
  }

  async function confirmRemove() {
    if (!rule) return
    // `run` resolves to `null` only on failure, which `withNotify` reports.
    if ((await remove.run(rule.id)) === null) return
    setRemoving(false)
    onChanged()
  }

  return (
    <>
      <div className="flex flex-wrap items-center gap-2 text-sm">
        {level ? (
          <ClassificationBadge classification={level} />
        ) : (
          <span className="text-muted-foreground">
            {column
              ? `Not set for this column. The table as a whole is ${CLASSIFICATION_LABEL[a.classification]}.`
              : "Not set for the whole table."}
          </span>
        )}
        {canWrite && !editing ? (
          <span className="flex gap-1">
            {!level || rule ? (
              <Button
                size="sm"
                variant="outline"
                onClick={() => {
                  setChoice(level ?? "confidential")
                  save.reset()
                  setEditing(true)
                }}
              >
                {level ? "Change" : "Set"}
              </Button>
            ) : null}
            {rule ? (
              <Button size="sm" variant="ghost" onClick={() => setRemoving(true)}>
                Remove
              </Button>
            ) : null}
          </span>
        ) : null}
      </div>
      {editing ? (
        <div className="flex flex-wrap items-end gap-2">
          <div className="grid gap-1">
            <Label htmlFor={fieldId} className="text-xs">
              Sensitivity of {column ?? "the whole table"}
            </Label>
            <SelectField
              id={fieldId}
              value={choice}
              options={SENSITIVITY_LEVELS.map((l) => ({
                value: l,
                label: CLASSIFICATION_LABEL[l],
              }))}
              onValueChange={(next) => setChoice(next as Classification)}
            />
          </div>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={save.status === "pending" || choice === level}
          >
            {save.status === "pending" ? "Saving…" : "Save"}
          </Button>
          <Button size="sm" variant="ghost" onClick={() => setEditing(false)} disabled={save.status === "pending"}>
            Cancel
          </Button>
          <p className="w-full text-xs text-muted-foreground">
            The table is never less restrictive than its most restrictive column.
          </p>
        </div>
      ) : null}
      {save.error ? (
        <p className="text-sm text-destructive">
          {oldRemoved
            ? `The new level was not saved: ${save.error.message} The old level was already removed, so ${column ? "the column" : "the table"} shows what is stored now.`
            : `The level was not saved: ${save.error.message}`}
        </p>
      ) : null}
      <ConfirmActionDialog
        open={removing}
        onOpenChange={(open) => (open ? undefined : setRemoving(false))}
        title="Remove classification"
        description={`Remove the ${level ? CLASSIFICATION_LABEL[level] : ""} classification of ${what}?`}
        impact={
          column
            ? "An older rule for the column applies again if there is one. Otherwise the column has no level of its own."
            : "An older rule for the table applies again if there is one. Otherwise the table takes its most restrictive column's level, or the default."
        }
        confirmLabel="Remove"
        confirming={remove.status === "pending"}
        destructive
        onConfirm={() => void confirmRemove()}
      />
    </>
  )
}

function PolicyName({ policy, canOpen }: { policy: PolicySummary; canOpen: boolean }) {
  return canOpen ? (
    <Link
      href={`/governance/policies?id=${encodeURIComponent(policy.id)}`}
      className="font-medium text-primary hover:underline"
    >
      {policy.name}
    </Link>
  ) : (
    <span className="font-medium">{policy.name}</span>
  )
}

function MaskList({
  asset: a,
  column,
  masks,
  maskedForYou,
  canReadPolicies,
  canWrite,
  onChanged,
}: {
  asset: AssetDetail
  column: string
  masks: ColumnMask<PolicySummary>[]
  maskedForYou: boolean
  canReadPolicies: boolean
  canWrite: boolean
  onChanged: () => void
}) {
  const [removing, setRemoving] = React.useState<PolicySummary | null>(null)
  const [adding, setAdding] = React.useState(false)
  const remove = useServiceAction(
    withNotify(
      { success: `The mask on ${column} is removed`, error: "Failed to remove the mask" },
      (signal, id: string) => governanceService.deletePolicy(id, signal)
    )
  )

  async function confirmRemove() {
    if (!removing) return
    // `run` resolves to `null` only on failure, which `withNotify` reports.
    if ((await remove.run(removing.id)) === null) return
    setRemoving(null)
    onChanged()
  }

  return (
    <>
      {masks.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {canReadPolicies
            ? "No one. Every role that can read the table sees this column's values."
            : maskedForYou
              ? "You. Who else it is masked for is shown with policy:read."
              : "Not for you. Who it is masked for is shown with policy:read."}
        </p>
      ) : (
        <ul className="flex flex-col gap-2 text-sm">
          {masks.map((m) => (
            <li key={m.policy.id} className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span className="font-medium">
                {m.policy.roles && m.policy.roles.length > 0
                  ? m.policy.roles.join(", ")
                  : "Roles shown with policy:read"}
              </span>
              <span className="text-xs text-muted-foreground">
                by <PolicyName policy={m.policy} canOpen={canReadPolicies} />
              </span>
              {m.kind === "own" && canWrite ? (
                <Button
                  size="sm"
                  variant="ghost"
                  className="ml-auto"
                  aria-label={`Remove the mask ${m.policy.name}`}
                  onClick={() => setRemoving(m.policy)}
                >
                  Remove
                </Button>
              ) : null}
              {m.kind === "shared" ? (
                <span className="w-full text-xs text-muted-foreground">
                  {sharedMaskSentence(m)}{" "}
                  <Link href={`/data/assets/${encodeURIComponent(a.id)}?tab=access`} className="text-primary hover:underline">
                    Change it on the Access tab
                  </Link>
                </span>
              ) : null}
            </li>
          ))}
        </ul>
      )}
      {canWrite && !adding ? (
        <Button size="sm" variant="outline" className="w-fit" onClick={() => setAdding(true)}>
          Add a mask
        </Button>
      ) : null}
      {adding ? (
        <AddMaskForm
          asset={a}
          column={column}
          onCancel={() => setAdding(false)}
          onSaved={() => {
            setAdding(false)
            onChanged()
          }}
        />
      ) : null}
      <ConfirmActionDialog
        open={removing !== null}
        onOpenChange={(open) => (open ? undefined : setRemoving(null))}
        title="Remove the mask"
        description={`Delete ${removing?.name ?? "the policy"}, which masks ${column} for ${removing?.roles?.join(", ") ?? "its roles"}?`}
        impact={`From the next query on, those roles read ${column} unmasked.`}
        confirmLabel="Remove mask"
        confirming={remove.status === "pending"}
        destructive
        onConfirm={() => void confirmRemove()}
      />
    </>
  )
}

/** `mask_silver_orders_email_for_analyst`: policy names are unique, so the roles are in it too. */
export function maskPolicyName(table: string, column: string, roles: string[]): string {
  const slug = (s: string) => s.toLowerCase().replace(/[^a-z0-9]+/g, "_").replace(/^_+|_+$/g, "")
  return [`mask_${slug(table)}_${slug(column)}`, roles.length > 0 ? `for_${roles.map(slug).join("_")}` : ""]
    .filter(Boolean)
    .join("_")
}

/**
 * Masks the column for the roles typed, as an enforced Column mask policy:
 * the same `createPolicy` call and condition the Access tab's Add policy
 * dialog writes. It is enforced at once, because the list above shows
 * enforced masks; a draft would be added and not appear.
 */
function AddMaskForm({
  asset: a,
  column,
  onCancel,
  onSaved,
}: {
  asset: AssetDetail
  column: string
  onCancel: () => void
  onSaved: () => void
}) {
  const table = policyTable(a)
  const [roles, setRoles] = React.useState("")
  const [name, setName] = React.useState("")
  const [nameTouched, setNameTouched] = React.useState(false)
  const condition = policyCondition(table, { roles, mask: [column], rowFilter: "" })
  const shownName = nameTouched ? name : maskPolicyName(table, column, condition?.roles ?? [])
  const save = useServiceAction((signal, input: Parameters<typeof governanceService.createPolicy>[0]) =>
    governanceService.createPolicy(input, signal)
  )

  async function submit() {
    if (!condition || !shownName.trim()) return
    const saved = await save.run({
      name: shownName.trim(),
      kind: "Column mask",
      subjects: condition.roles.join(", "),
      resources: table,
      effect: "Permit with obligation",
      conditions: JSON.stringify(condition),
      activate: true,
    })
    if (saved === null) return
    notifySuccess(`${column} is masked for ${condition.roles.join(", ")}`)
    onSaved()
  }

  return (
    <div className="grid gap-2 rounded-lg border border-border p-3">
      <div className="grid gap-1">
        <Label htmlFor={`mask-roles-${column}`} className="text-xs">
          Mask {column} for these roles
        </Label>
        <Input
          id={`mask-roles-${column}`}
          value={roles}
          onChange={(e) => setRoles(e.target.value)}
          placeholder="Analyst, Dashboard Viewer"
        />
        <p className="text-xs text-muted-foreground">
          They read it as *** from the next query on. Separate roles with commas.
        </p>
      </div>
      <div className="grid gap-1">
        <Label htmlFor={`mask-name-${column}`} className="text-xs">
          Policy name
        </Label>
        <Input
          id={`mask-name-${column}`}
          value={shownName}
          onChange={(e) => {
            setNameTouched(true)
            setName(e.target.value)
          }}
          className="font-mono text-xs"
        />
      </div>
      {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
      <div className="flex gap-2">
        <Button
          size="sm"
          onClick={() => void submit()}
          disabled={!condition || !shownName.trim() || save.status === "pending"}
        >
          {save.status === "pending" ? "Adding…" : "Add mask"}
        </Button>
        <Button size="sm" variant="ghost" onClick={onCancel} disabled={save.status === "pending"}>
          Cancel
        </Button>
      </div>
    </div>
  )
}
