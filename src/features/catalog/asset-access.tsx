"use client"

import * as React from "react"
import Link from "next/link"
import { Check, Minus, Tag } from "lucide-react"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { ClassificationBadge, Pill } from "@/components/patterns/status-badge"
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
import { Label } from "@/components/ui/label"
import { useAuth } from "@/features/auth/auth-provider"
import { useServiceAction } from "@/hooks/use-service"
import { notifySuccess } from "@/lib/notify"
import { CLASSIFICATION_LABEL, type Classification } from "@/lib/status"
import { governanceService } from "@/services"
import type { AssetDetail } from "@/services/contracts/assets"
import type { CreateClassificationRuleInput } from "@/services/contracts/governance"
import { classificationTitle } from "./asset-badges"
import { lineageKey } from "./asset-lineage"

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

const selectClassName = "h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"

/** Least to most restrictive. */
const LEVELS: Classification[] = ["public", "internal", "confidential", "restricted"]

/**
 * Classifies the asset, or one of its columns, by adding a classification
 * rule for its table key. The newest rule wins, so this is also how a
 * wrong classification is corrected.
 */
function ClassifyDialog({
  asset: a,
  onClose,
  onSaved,
}: {
  asset: AssetDetail
  onClose: () => void
  onSaved: () => void
}) {
  // "" is the asset as a whole.
  const [column, setColumn] = React.useState("")
  const [level, setLevel] = React.useState<Classification>(a.classification)
  const save = useServiceAction((signal, input: CreateClassificationRuleInput) =>
    governanceService.createClassificationRule(input, signal)
  )

  async function submit() {
    const saved = await save.run({
      asset: lineageKey(a),
      ...(column ? { column } : {}),
      classification: level,
    })
    if (saved === null) return
    notifySuccess("Classification saved")
    onSaved()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Classify</DialogTitle>
          <DialogDescription>
            How sensitive {a.name} is. An asset is never less restrictive than its most
            restrictive column.
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-3">
          <div className="grid gap-1.5">
            <Label htmlFor="classify-target">Applies to</Label>
            <select
              id="classify-target"
              className={selectClassName}
              value={column}
              onChange={(e) => setColumn(e.target.value)}
            >
              <option value="">The whole asset</option>
              {a.schema.map((c) => (
                <option key={c.name} value={c.name}>
                  Column {c.name}
                </option>
              ))}
            </select>
          </div>
          <div className="grid gap-1.5">
            <Label htmlFor="classify-level">Classification</Label>
            <select
              id="classify-level"
              className={selectClassName}
              value={level}
              onChange={(e) => setLevel(e.target.value as Classification)}
            >
              {LEVELS.map((l) => (
                <option key={l} value={l}>
                  {CLASSIFICATION_LABEL[l]}
                </option>
              ))}
            </select>
          </div>
          {save.error ? <p className="text-sm text-destructive">{save.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button size="sm" onClick={() => void submit()} disabled={save.status === "pending"}>
            {save.status === "pending" ? "Saving…" : "Save"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

/** How sensitive the asset and its columns are, and on whose word. */
function ClassificationCard({ asset: a, onChanged }: { asset: AssetDetail; onChanged: () => void }) {
  const [classifying, setClassifying] = React.useState(false)
  const columns = a.schema.filter((c) => c.classification)

  return (
    <SectionCard
      size="sm"
      title="Classification"
      description={classificationTitle(a)}
      action={
        <Button size="sm" variant="outline" onClick={() => setClassifying(true)}>
          <Tag />
          Classify
        </Button>
      }
    >
      <ul className="divide-y divide-border text-sm">
        <li className="flex items-center gap-2 py-1.5">
          <span>This asset</span>
          <span className="ml-auto">
            <ClassificationBadge classification={a.classification} />
          </span>
        </li>
        {columns.map((c) => (
          <li key={c.name} className="flex items-center gap-2 py-1.5">
            <span className="font-mono text-xs">{c.name}</span>
            <span className="text-xs text-muted-foreground">column</span>
            <span className="ml-auto">
              {c.classification ? <ClassificationBadge classification={c.classification} /> : null}
            </span>
          </li>
        ))}
      </ul>
      {columns.length === 0 ? (
        <p className="mt-2 text-xs text-muted-foreground">No column is classified on its own.</p>
      ) : null}
      {classifying ? (
        <ClassifyDialog
          asset={a}
          onClose={() => setClassifying(false)}
          onSaved={() => {
            setClassifying(false)
            onChanged()
          }}
        />
      ) : null}
    </SectionCard>
  )
}

function PolicyItem({ p, canOpen }: { p: PolicySummary; canOpen: boolean }) {
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
 * The Access tab: what the signed-in person may do with the asset, how
 * sensitive it is, then the policies that mask its columns or filter its
 * rows, and for whom.
 */
export function AssetAccess({
  asset: a,
  onChanged,
}: {
  asset: AssetDetail
  /** Reloads the asset after it was classified. */
  onChanged: () => void
}) {
  const { hasPermission } = useAuth()
  const canOpen = hasPermission("policy:read")

  return (
    <div className="flex flex-col gap-2">
      <YourAccess asset={a} />
      <ClassificationCard asset={a} onChanged={onChanged} />
      <SectionCard
        size="sm"
        title="Policies"
        description="Masking and row filters bound to this asset's tables."
        action={
          canOpen ? (
            <Button size="sm" variant="outline" render={<Link href="/governance/policies" />}>
              Open policies
            </Button>
          ) : undefined
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
              <PolicyItem key={p.id} p={p} canOpen={canOpen} />
            ))}
          </ul>
        )}
      </SectionCard>
    </div>
  )
}
