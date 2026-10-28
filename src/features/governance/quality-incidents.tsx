"use client"

import * as React from "react"
import Link from "next/link"
import { SectionCard } from "@/components/patterns/section-card"
import { EmptyState } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
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
import { Textarea } from "@/components/ui/textarea"
import { useAuth } from "@/features/auth/auth-provider"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatRelativeTime } from "@/lib/format"
import { notifySuccess } from "@/lib/notify"
import { governanceService } from "@/services"
import type { QualityIncident } from "@/services/contracts/governance"

/** The words an incident kind is shown with. */
export const INCIDENT_KIND_LABEL: Record<string, string> = {
  check: "Check failed",
  volume_anomaly: "Volume anomaly",
  freshness_anomaly: "Freshness anomaly",
}

export type IncidentGroup = {
  asset: string
  incidents: QualityIncident[]
  /** When the oldest incident of the table opened. */
  oldest: string
}

/**
 * The incidents grouped by table, the table with the oldest incident first
 * (what has been wrong longest is read first); inside a table, oldest first.
 * Times are `YYYY-MM-DDTHH:MM:SS…Z`, so they order as text.
 */
export function groupByTable(incidents: QualityIncident[]): IncidentGroup[] {
  const byAsset = new Map<string, QualityIncident[]>()
  for (const incident of incidents) {
    byAsset.set(incident.asset, [...(byAsset.get(incident.asset) ?? []), incident])
  }
  return [...byAsset.entries()]
    .map(([asset, list]) => {
      const incidents = [...list].sort((a, b) => a.openedAt.localeCompare(b.openedAt))
      return { asset, incidents, oldest: incidents[0].openedAt }
    })
    .sort((a, b) => a.oldest.localeCompare(b.oldest) || a.asset.localeCompare(b.asset))
}

/** A reason is required; blank text is not one. */
export function resolutionReason(text: string): string | null {
  const reason = text.trim()
  return reason === "" ? null : reason
}

function ResolveDialog({
  incident,
  onClose,
  onResolved,
}: {
  incident: QualityIncident
  onClose: () => void
  onResolved: () => void
}) {
  const [text, setText] = React.useState("")
  const resolve = useServiceAction((signal, reason: string) =>
    governanceService.resolveQualityIncident(incident.id, reason, signal)
  )
  const reason = resolutionReason(text)

  async function submit() {
    if (!reason) return
    const resolved = await resolve.run(reason)
    if (resolved === null) return
    notifySuccess("Incident resolved")
    onResolved()
  }

  return (
    <Dialog open onOpenChange={(next) => (next ? undefined : onClose())}>
      <DialogContent className="sm:max-w-md">
        <DialogHeader>
          <DialogTitle>Resolve incident</DialogTitle>
          <DialogDescription>
            <span className="font-mono">{incident.asset}</span>: {incident.detail}
          </DialogDescription>
        </DialogHeader>
        <div className="grid gap-1.5">
          <Label htmlFor="incident-resolution">Why is it resolved?</Label>
          <Textarea
            id="incident-resolution"
            rows={3}
            value={text}
            onChange={(e) => setText(e.target.value)}
            placeholder="The source was corrected and reloaded"
          />
          <p className="text-xs text-muted-foreground">
            The reason is kept with the incident. A check that fails again opens a new incident.
          </p>
          {resolve.error ? <p className="text-sm text-destructive">{resolve.error.message}</p> : null}
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Cancel</DialogClose>
          <Button
            size="sm"
            onClick={() => void submit()}
            disabled={!reason || resolve.status === "pending"}
          >
            {resolve.status === "pending" ? "Resolving…" : "Resolve"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}

function IncidentRow({
  incident,
  onResolve,
}: {
  incident: QualityIncident
  onResolve?: (incident: QualityIncident) => void
}) {
  return (
    <li className="flex items-start justify-between gap-3 py-2">
      <div className="min-w-0">
        <div className="flex flex-wrap items-center gap-2">
          <Pill tone={incident.kind === "check" ? "destructive" : "warning"}>
            {INCIDENT_KIND_LABEL[incident.kind] ?? incident.kind}
          </Pill>
          {/* PR #106 review R1 SHOULD-FIX DATA-14 3 / D2: show stale badge on incidents page */}
          {incident.status === "stale" ? <Pill tone="neutral">stale</Pill> : null}
          {incident.ruleName ? (
            <span className="font-mono text-xs text-muted-foreground">{incident.ruleName}</span>
          ) : null}
        </div>
        <p className="mt-1 text-sm">{incident.detail}</p>
        <p className="mt-0.5 text-xs text-muted-foreground">
          Opened {formatRelativeTime(incident.openedAt)} · last seen{" "}
          {formatRelativeTime(incident.lastSeenAt)}
          {incident.status === "resolved" ? (
            <>
              {" "}
              · resolved {formatRelativeTime(incident.resolvedAt)}
              {incident.resolvedBy ? ` by ${incident.resolvedBy}` : ""}: {incident.resolution}
            </>
          ) : null}
        </p>
      </div>
      {onResolve && (incident.status === "open" || incident.status === "stale") ? (
        <Button size="sm" variant="outline" onClick={() => onResolve(incident)}>
          Resolve
        </Button>
      ) : null}
    </li>
  )
}

/**
 * What is wrong now, by table (`DATA-14`, F7): the open quality incidents,
 * grouped by table with the oldest first, each with what was found and a way
 * to resolve it with a reason (`governance:write`); and, one click away, the
 * resolved ones as history. On an asset's Quality tab pass `asset` to list
 * that table's alone.
 */
export function QualityIncidents({ asset }: { asset?: string }) {
  const { hasPermission } = useAuth()
  const [view, setView] = React.useState<"open" | "resolved">("open")
  const [resolving, setResolving] = React.useState<QualityIncident | null>(null)
  const state = useService(
    (signal) => governanceService.listQualityIncidents({ status: view, asset }, signal),
    [view, asset]
  )
  const canResolve = hasPermission("governance:write")
  const groups = state.status === "success" ? groupByTable(state.data) : []
  const count = state.status === "success" ? state.data.length : null

  return (
    <SectionCard
      size="sm"
      title={view === "open" ? "Open incidents" : "Resolved incidents"}
      description={
        count === null
          ? undefined
          : view === "open"
            ? count === 0
              ? "Nothing is wrong right now."
              : `${count} open across ${groups.length} table${groups.length === 1 ? "" : "s"}.`
            : `${count} resolved, newest first.`
      }
      action={
        <div className="flex gap-1.5">
          <Button
            size="sm"
            variant={view === "open" ? "secondary" : "ghost"}
            onClick={() => setView("open")}
          >
            Open
          </Button>
          <Button
            size="sm"
            variant={view === "resolved" ? "secondary" : "ghost"}
            onClick={() => setView("resolved")}
          >
            Resolved
          </Button>
        </div>
      }
    >
      {state.status === "loading" ? (
        <p className="text-sm text-muted-foreground">Loading incidents…</p>
      ) : null}
      {state.status === "error" ? (
        <p className="text-sm text-destructive" role="alert">
          The incidents could not be loaded: {state.error.message}
        </p>
      ) : null}
      {state.status === "success" && groups.length === 0 ? (
        <EmptyState
          title={view === "open" ? "No open incidents" : "No resolved incidents"}
          description={
            view === "open"
              ? "A failed check, or a load far smaller or later than usual, opens one here."
              : "Resolved incidents stay here as history."
          }
          className="py-4"
        />
      ) : null}
      <div className="flex flex-col gap-3">
        {groups.map((group) => (
          <section key={group.asset} className="rounded-lg border border-border px-3">
            <header className="flex flex-wrap items-baseline justify-between gap-2 border-b border-border py-2">
              <Link
                href={`/data?q=${encodeURIComponent(group.asset)}`}
                className="font-mono text-sm font-medium hover:underline"
              >
                {group.asset}
              </Link>
              <span className="text-xs text-muted-foreground">
                {group.incidents.length} {view === "open" ? "open" : "resolved"} · oldest opened{" "}
                {formatRelativeTime(group.oldest)}
              </span>
            </header>
            <ul className="divide-y divide-border">
              {group.incidents.map((incident) => (
                <IncidentRow
                  key={incident.id}
                  incident={incident}
                  onResolve={canResolve ? setResolving : undefined}
                />
              ))}
            </ul>
          </section>
        ))}
      </div>
      {resolving ? (
        <ResolveDialog
          incident={resolving}
          onClose={() => setResolving(null)}
          onResolved={() => {
            setResolving(null)
            state.reload()
          }}
        />
      ) : null}
    </SectionCard>
  )
}
