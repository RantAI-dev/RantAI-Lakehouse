"use client"

import Link from "next/link"
import { Check, Minus } from "lucide-react"
import { EmptyState } from "@/components/patterns/page-states"
import { SectionCard } from "@/components/patterns/section-card"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useAuth } from "@/features/auth/auth-provider"
import type { AssetDetail } from "@/services/contracts/assets"

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
 * The Access tab: what the signed-in person may do with the asset, then
 * the policies that mask its columns or filter its rows, and for whom.
 */
export function AssetAccess({ asset: a }: { asset: AssetDetail }) {
  const { hasPermission } = useAuth()
  const canOpen = hasPermission("policy:read")

  return (
    <div className="flex flex-col gap-2">
      <YourAccess asset={a} />
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
