"use client"

import * as React from "react"
import Link from "next/link"
import { Check, Minus, Plus, XIcon } from "lucide-react"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Badge } from "@/components/ui/badge"
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
import { Checkbox } from "@/components/ui/checkbox"
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { PolicyActions } from "@/features/governance/policy-actions"
import { useService, useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { governanceService, identityService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import type { CreatePolicyInput } from "@/services/contracts/governance"
import { AssetGrants } from "./asset-grants"
import { policyCondition, policyTable, roleNames, type PolicyForm } from "./asset-policy"

type PolicySummary = AssetDetail["policySummary"][number]

/** What each permission lets the signed-in person do on this page. */
const ABILITIES = [
  { permission: "catalog:read", label: "See this asset and its schema" },
  { permission: "query:read", label: "Read its rows: sample, column statistics, queries" },
  { permission: "lineage:read", label: "See where it comes from and what reads it" },
  { permission: "catalog:write", label: "Edit its description, owner and tags" },
] as const

/** What the signed-in person may do with the asset, and what is hidden from them. */
function YourAccess({ asset: a }: { asset: AssetDetail }) {
  const { hasPermission } = useAuth()
  const masked = a.schema.filter((c) => c.masked).map((c) => c.name)
  const missing = ABILITIES.some((ability) => !hasPermission(ability.permission))

  return (
    <SectionCard
      size="sm"
      title="Your access"
      description={
        missing ? "Use Request access above to ask for what you do not hold." : undefined
      }
    >
      <ul className="divide-y divide-border text-sm">
        {ABILITIES.map(({ permission, label }) => {
          const granted = hasPermission(permission)
          return (
            <li key={permission} className="flex items-center gap-2 py-1.5">
              {granted ? (
                <Check className="size-4 shrink-0 text-emerald-600 dark:text-emerald-400" aria-hidden />
              ) : (
                <Minus className="size-4 shrink-0 text-muted-foreground" aria-hidden />
              )}
              <span className={granted ? undefined : "text-muted-foreground"}>{label}</span>
              <span className="ml-auto flex shrink-0 items-center gap-2">
                <span className="font-mono text-xs text-muted-foreground">{permission}</span>
                <Pill tone={granted ? "success" : "neutral"}>{granted ? "Granted" : "Not granted"}</Pill>
              </span>
            </li>
          )
        })}
        <li className="flex flex-wrap items-center gap-2 py-1.5">
          <span>Columns masked for you</span>
          <span className="ml-auto flex flex-wrap justify-end gap-1">
            {masked.length === 0 ? (
              <span className="text-muted-foreground">None</span>
            ) : (
              masked.map((name) => (
                <Pill key={name} tone="warning" className="font-mono">
                  {name}
                </Pill>
              ))
            )}
          </span>
        </li>
      </ul>
    </SectionCard>
  )
}

/**
 * Adds a masking / row-filter policy for this asset's table without
 * leaving the page: the roles it applies to, the columns they see as
 * `***`, and the rows they are limited to.
 */
function AddPolicyDialog({
  asset: a,
  onClose,
  onSaved,
}: {
  asset: AssetDetail
  onClose: () => void
  onSaved: () => void
}) {
  const table = policyTable(a)
  const [form, setForm] = React.useState<PolicyForm>({ roles: "", mask: [], rowFilter: "" })
  const [name, setName] = React.useState(`govern_${table.replace(".", "_")}`)
  const [activate, setActivate] = React.useState(false)
  const save = useServiceAction((signal, input: CreatePolicyInput) =>
    governanceService.createPolicy(input, signal)
  )
  const condition = policyCondition(table, form)
  // WEB-8 D8: the roles are the ones the deployment has, from
  // `identityService.listRoles`. Without that read the field stays the free
  // text it was, with the reason, rather than an empty list of checkboxes.
  const rolesState = useService((signal) => identityService.listRoles(signal), [])
  const roles = rolesState.status === "success" ? rolesState.data : null
  // WEB-8 S2 T8 (D8): the row filter grammar comes from the server's own
  // validator constants (functions, operators and, since S2 review round 1 S2,
  // forms). While it loads, or when the read fails, there is no help line at
  // all — a guessed or stale list would invite a save the server refuses.
  const grammarState = useService((signal) => governanceService.getRowFilterGrammar(signal), [])
  const grammar = grammarState.status === "success" ? grammarState.data : null
  const chosenRoles = roleNames(form.roles)
  // WEB-8 S2 review round 1, S1: a saved role the deployment no longer has
  // must not be invisible. It stays in `form.roles` (and so in the saved
  // `subjects`) until the person removes it, shown as a removable "Not
  // recognised" chip the way `permission-picker.tsx` shows an unknown
  // permission token.
  const knownRoleNames = roles?.map((role) => role.name) ?? []
  const unknownRoles = chosenRoles.filter((name) => !knownRoleNames.includes(name))

  function toggleRole(roleName: string) {
    setForm((f) => {
      const current = roleNames(f.roles)
      const next = current.includes(roleName)
        ? current.filter((r) => r !== roleName)
        : [...current, roleName]
      return { ...f, roles: next.join(", ") }
    })
  }

  function toggle(column: string) {
    setForm((f) => ({
      ...f,
      mask: f.mask.includes(column) ? f.mask.filter((c) => c !== column) : [...f.mask, column],
    }))
  }

  async function submit() {
    if (!condition || !name.trim()) return
    const saved = await save.run({
      name: name.trim(),
      kind: condition.mask.length > 0 ? "Column mask" : "Row filter",
      subjects: condition.roles.join(", "),
      resources: table,
      effect: "Permit with obligation",
      conditions: JSON.stringify(condition),
      activate,
    })
    if (saved === null) return
    notifySuccess(activate ? "Policy added and enforced" : "Policy saved as a draft")
    onSaved()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Add policy</DialogTitle>
          <DialogDescription>
            What the roles below may see of <span className="font-mono">{table}</span>.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="policy-roles">Roles it applies to</Label>
            {roles === null ? (
              <>
                <Input
                  id="policy-roles"
                  value={form.roles}
                  onChange={(e) => setForm((f) => ({ ...f, roles: e.target.value }))}
                  placeholder="Analyst, Dashboard Viewer"
                />
                {rolesState.status === "error" ? (
                  <p className="text-xs text-muted-foreground">
                    Could not read the roles; type them as a comma-separated list.
                  </p>
                ) : null}
              </>
            ) : roles.length === 0 ? (
              <p className="text-xs text-muted-foreground">No roles defined</p>
            ) : (
              <ul className="grid max-h-40 gap-1 overflow-y-auto rounded-md border border-border p-1">
                {roles.map((role) => (
                  <li key={role.id} className="flex items-center gap-2 px-1.5 py-1 text-sm">
                    <Checkbox
                      id={`policy-role-${role.id}`}
                      checked={chosenRoles.includes(role.name)}
                      aria-label={role.name}
                      onCheckedChange={() => toggleRole(role.name)}
                    />
                    <label
                      htmlFor={`policy-role-${role.id}`}
                      className="flex-1 cursor-pointer truncate text-xs select-none"
                    >
                      {role.name}
                    </label>
                  </li>
                ))}
              </ul>
            )}
            {roles !== null && unknownRoles.length > 0 ? (
              <div className="space-y-1.5" data-testid="unknown-roles">
                <Label className="text-xs font-medium text-muted-foreground">Not recognised</Label>
                <div className="flex flex-wrap gap-1.5">
                  {unknownRoles.map((name) => (
                    <Badge
                      key={name}
                      variant="outline"
                      className="gap-1.5 border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-400 py-1 px-2 font-mono text-xs"
                    >
                      <span>{name}</span>
                      <span className="text-[10px] text-muted-foreground font-sans">
                        (Not recognised)
                      </span>
                      <button
                        type="button"
                        className="hover:text-foreground inline-flex items-center justify-center cursor-pointer"
                        onClick={() => toggleRole(name)}
                        aria-label={`Remove ${name}`}
                        title={`Remove ${name}`}
                      >
                        <XIcon className="size-3" />
                      </button>
                    </Badge>
                  ))}
                </div>
              </div>
            ) : null}
          </div>
          <fieldset className="grid gap-1.5">
            <legend className="mb-1.5 text-sm font-medium">Columns to mask</legend>
            <div className="flex max-h-40 flex-wrap gap-x-4 gap-y-1 overflow-y-auto">
              {a.schema.map((c) => (
                <label key={c.name} className="flex items-center gap-1.5 text-sm">
                  <input
                    type="checkbox"
                    checked={form.mask.includes(c.name)}
                    onChange={() => toggle(c.name)}
                  />
                  <span className="font-mono text-xs">{c.name}</span>
                </label>
              ))}
            </div>
          </fieldset>
          <div className="grid gap-1.5">
            <Label htmlFor="policy-filter">Row filter (optional)</Label>
            <Input
              id="policy-filter"
              value={form.rowFilter}
              onChange={(e) => setForm((f) => ({ ...f, rowFilter: e.target.value }))}
              placeholder="region = 'ID'"
              aria-describedby={grammar ? "policy-filter-help" : undefined}
              className="font-mono"
            />
            {grammar ? (
              <p id="policy-filter-help" className="text-xs text-muted-foreground">
                Functions: {grammar.functions.join(", ")}. Operators:{" "}
                {grammar.operators.join(", ")}. Forms: {grammar.forms.join(", ")}.
              </p>
            ) : null}
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="policy-name">Name</Label>
            <Input id="policy-name" value={name} onChange={(e) => setName(e.target.value)} />
          </div>
          <label className="flex items-center gap-2 text-sm">
            <input type="checkbox" checked={activate} onChange={(e) => setActivate(e.target.checked)} />
            Enforce it now
            <span className="text-xs text-muted-foreground">
              — otherwise it is saved as a draft that changes nothing yet
            </span>
          </label>
          {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={!condition || !name.trim() || save.status === "pending"}
          >
            {save.status === "pending" ? "Saving…" : activate ? "Add and enforce" : "Save draft"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function PolicyItem({
  p,
  canOpen,
  onChanged,
}: {
  p: PolicySummary
  canOpen: boolean
  onChanged: () => void
}) {
  return (
    <li className="flex flex-col gap-1.5 py-2.5">
      <div className="flex flex-wrap items-center gap-2">
        {canOpen ? (
          <Link
            href={`/governance/policies?id=${encodeURIComponent(p.id)}`}
            className="font-medium text-primary hover:underline"
          >
            {p.name}
          </Link>
        ) : (
          <span className="font-medium">{p.name}</span>
        )}
        {p.kind ? <Pill tone="neutral">{p.kind}</Pill> : null}
        {p.status && p.status !== "ready" ? (
          <Pill tone="warning">{p.status} · not enforced</Pill>
        ) : null}
        {p.appliesToYou ? <Pill tone="info">Applies to you</Pill> : null}
        <span className="ml-auto text-xs text-muted-foreground">{p.effect}</span>
        <PolicyActions
          // Every `policySummary` entry passed `PolicyCondition::parse_opt`
          // in `routes::catalog_governance::summarize_policies` — a policy
          // with no enforceable condition binds no table and is left out —
          // so these rows are enforceable by construction.
          policy={{ id: p.id, name: p.name, status: p.status ?? "draft", enforceable: true }}
          onChanged={onChanged}
        />
      </div>
      <dl className="grid gap-x-3 gap-y-1 text-xs sm:grid-cols-[max-content_1fr]">
        {p.table ? (
          <>
            <dt className="text-muted-foreground">Table</dt>
            <dd className="font-mono">{p.table}</dd>
          </>
        ) : null}
        {p.roles ? (
          <>
            <dt className="text-muted-foreground">Roles</dt>
            <dd>{p.roles.join(", ")}</dd>
          </>
        ) : null}
        {p.mask && p.mask.length > 0 ? (
          <>
            <dt className="text-muted-foreground">Masks</dt>
            <dd className="flex flex-wrap gap-1">
              {p.mask.map((column) => (
                <Pill key={column} tone="warning" className="font-mono">
                  {column}
                </Pill>
              ))}
            </dd>
          </>
        ) : null}
        {p.rowFilter ? (
          <>
            <dt className="text-muted-foreground">Row filter</dt>
            <dd className="font-mono">{p.rowFilter}</dd>
          </>
        ) : null}
      </dl>
    </li>
  )
}

/**
 * The Access tab: what the signed-in person may do with the asset, its
 * grants, then the policies that mask its columns or filter its rows, and
 * for whom. How sensitive a column is, and a mask on one column, are set on
 * the column itself in the Schema tab (`WEB-3` S3-D1, S3-D2: one place per
 * fact); this tab keeps the policies, which may span several columns.
 */
export function AssetAccess({
  asset: a,
  onChanged,
}: {
  asset: AssetDetail
  /** Reloads the asset after a policy changed. */
  onChanged: () => void
}) {
  const { hasPermission } = useAuth()
  const canOpen = hasPermission("policy:read")
  const [addingPolicy, setAddingPolicy] = React.useState(false)

  return (
    <div className="flex flex-col gap-2">
      <YourAccess asset={a} />
      <AssetGrants asset={a} />
      <SectionCard
        size="sm"
        title="Policies"
        description="Masking and row filters bound to this asset's tables."
        action={
          <div className="flex flex-wrap justify-end gap-1.5">
            {hasPermission("policy:write") ? (
              <Button size="sm" variant="outline" onClick={() => setAddingPolicy(true)}>
                <Plus />
                Add policy
              </Button>
            ) : null}
            {canOpen ? (
              <Button size="sm" variant="ghost" render={<Link href="/governance/policies" />}>
                Open access rules
              </Button>
            ) : null}
          </div>
        }
      >
        {a.policySummary.length === 0 ? (
          <EmptyState
            title="No policy binds this asset"
            description="No column is masked and no row is filtered for any role."
            className="py-4"
          />
        ) : (
          <ul className="divide-y divide-border text-sm">
            {a.policySummary.map((p) => (
              <PolicyItem key={p.id} p={p} canOpen={canOpen} onChanged={onChanged} />
            ))}
          </ul>
        )}
        {addingPolicy ? (
          <AddPolicyDialog
            asset={a}
            onClose={() => setAddingPolicy(false)}
            onSaved={() => {
              setAddingPolicy(false)
              onChanged()
            }}
          />
        ) : null}
      </SectionCard>
    </div>
  )
}
