"use client"

import * as React from "react"
import { SectionCard } from "@/components/patterns/section-card"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { useService, useServiceAction } from "@/hooks/use-service"
import { formatDateTime } from "@/lib/format"
import { notifySuccess } from "@/lib/notify"
import { connectorService } from "@/services"
import type {
  SchemaChange,
  SchemaChangeList,
  SchemaChangePolicy,
} from "@/services/contracts/connectors"
import { driverForType } from "./connector-form-parts"

/** The four policies, in the order decision 1 of `SRC-8` lists them. */
export const POLICY_CHOICES: { value: SchemaChangePolicy; label: string; line: string }[] = [
  {
    value: "apply_non_breaking",
    label: "Apply non-breaking automatically",
    line: "A new column or a wider type is applied automatically and listed here.",
  },
  {
    value: "apply_all",
    label: "Apply all",
    line: "Does the same, and also adds new tables that appear in a schema this connector already loads from.",
  },
  {
    value: "ask_first",
    label: "Ask first",
    line: "A new column or a wider type waits for approval; the table keeps loading its known columns meanwhile.",
  },
  {
    value: "pause",
    label: "Pause",
    line: "Any change stops the whole connector until someone approves it.",
  },
]

/** `SRC-8` decision 2: the note under "Apply all" for a source that cannot list its tables. */
export const APPLY_ALL_NOTE = "For this source type this works like 'Apply non-breaking automatically'."

/** `SRC-8` decision 10, in the words the API's 409 uses. */
export const CANNOT_BE_LOADED_NOTE =
  "This type change cannot be loaded into the existing column. Change the column back at the source, or remove the table from the connector and add it again under a new target."

const KIND_LABEL: Record<string, string> = {
  column_added: "Column added",
  column_removed: "Column removed",
  primary_key_changed: "Primary key changed",
  table_added: "Table added",
}

/** A change in plain words: "Column added", "Type changed from X to Y", ... */
export function changeLabel(c: SchemaChange): string {
  if (c.kind === "type_changed") {
    return `Type changed from ${c.beforeValue ?? "unknown"} to ${c.afterValue ?? "unknown"}`
  }
  return KIND_LABEL[c.kind] ?? c.kind
}

/** What a change was, beyond its label: the column, the type or key it moved between. */
function changeDetail(c: SchemaChange): React.ReactNode {
  switch (c.kind) {
    case "column_added":
      return (
        <>
          <code className="font-mono text-xs">{c.columnName}</code>
          {c.afterValue ? <span className="text-muted-foreground"> ({c.afterValue})</span> : null}
        </>
      )
    case "column_removed":
    case "type_changed":
      return <code className="font-mono text-xs">{c.columnName}</code>
    case "primary_key_changed":
      return (
        <span className="text-muted-foreground">
          {c.beforeValue ?? "none"} → {c.afterValue ?? "none"}
        </span>
      )
    default:
      return null
  }
}

/** A table that "apply all" could not add is a notice, not a table that waits. */
const isNotice = (c: SchemaChange) => c.kind === "table_added"

type Group = { objectName: string; changes: SchemaChange[] }

function groupByTable(changes: SchemaChange[]): Group[] {
  const groups = new Map<string, Group>()
  for (const c of changes) {
    const group = groups.get(c.objectName) ?? { objectName: c.objectName, changes: [] }
    group.changes.push(c)
    groups.set(c.objectName, group)
  }
  return [...groups.values()]
}

/** The tables of `list` that wait for a decision (notices do not count). */
export function waitingTables(list: SchemaChangeList | null): Set<string> {
  return new Set((list?.pending ?? []).filter((c) => !isNotice(c)).map((c) => c.objectName))
}

function PendingGroup({
  connectorId,
  group,
  onDone,
}: {
  connectorId: string
  group: Group
  onDone: () => void
}) {
  const notice = group.changes.every(isNotice)
  // The API marks every waiting change of a table it will refuse; one is
  // enough (decision 10). The console does not re-derive the rule.
  const blocked = !notice && group.changes.some((c) => !c.canApprove)
  const approve = useServiceAction(async (signal, object: string) => {
    const result = await connectorService.approveSchemaChanges(connectorId, { object }, signal)
    if (!signal.aborted) notifySuccess(notice ? "Dismissed" : "Approved; the table loads on the next run")
    return result
  })
  return (
    <li className="space-y-1.5 px-3 py-2">
      <div className="flex flex-wrap items-center gap-2">
        <span className="min-w-0 flex-1 truncate font-mono text-xs" title={group.objectName}>
          {group.objectName}
        </span>
        {blocked ? (
          <Pill tone="warning">Waiting for a decision</Pill>
        ) : (
          <Button
            type="button"
            size="sm"
            variant={notice ? "ghost" : "default"}
            disabled={approve.status === "pending"}
            onClick={async () => {
              if (await approve.run(group.objectName)) onDone()
            }}
          >
            {notice ? "Dismiss" : "Approve and resume"}
          </Button>
        )}
      </div>
      <ul className="space-y-0.5 text-sm">
        {group.changes.map((c) => (
          <li key={c.id}>
            {notice ? (
              // `afterValue` is the API's fixed "Not added: ..." sentence.
              <span>{c.afterValue ?? "Not added"}</span>
            ) : (
              <>
                <span className="font-medium">{changeLabel(c)}</span> {changeDetail(c)}
              </>
            )}
          </li>
        ))}
      </ul>
      {blocked ? <p className="text-xs text-muted-foreground">{CANNOT_BE_LOADED_NOTE}</p> : null}
      {approve.status === "error" ? (
        <p role="alert" className="text-xs text-destructive">
          {approve.error.message}
        </p>
      ) : null}
    </li>
  )
}

function RecentChange({ c, inactiveSince }: { c: SchemaChange; inactiveSince: string | null }) {
  return (
    <li className="px-3 py-2 text-sm">
      <div className="flex flex-wrap items-baseline gap-x-2">
        <span className="font-mono text-xs">{c.objectName}</span>
        <span className="font-medium">{changeLabel(c)}</span>
        {changeDetail(c)}
        <span className="ml-auto text-xs text-muted-foreground">{formatDateTime(c.detectedAt)}</span>
      </div>
      {c.kind === "table_added" && c.afterValue ? (
        <p className="text-xs text-muted-foreground">Loads into bronze.{c.afterValue}</p>
      ) : null}
      {c.kind === "type_changed" || c.kind === "column_removed" || c.kind === "column_added" ? (
        <p className="text-xs text-muted-foreground">
          {c.status === "applied" ? "Applied" : "Approved"}
          {c.kind === "column_removed" && inactiveSince ? ` · inactive since ${formatDateTime(inactiveSince)}` : ""}
        </p>
      ) : null}
    </li>
  )
}

function PolicyCard({
  connectorId,
  connectorType,
  policy,
  onChanged,
}: {
  connectorId: string
  connectorType: string
  policy: SchemaChangePolicy
  onChanged: () => void
}) {
  const [choice, setChoice] = React.useState<SchemaChangePolicy>(policy)
  // The page reloads the connector after a save; take the stored value over.
  React.useEffect(() => setChoice(policy), [policy])
  const save = useServiceAction(async (signal, next: SchemaChangePolicy) => {
    const result = await connectorService.updateConnector(connectorId, { schemaChangePolicy: next }, signal)
    if (!signal.aborted) notifySuccess("Schema change policy saved")
    return result
  })
  const listsTables = driverForType(connectorType) !== undefined
  return (
    <SectionCard
      size="sm"
      title="When the source changes"
      description="A removed column, a narrowed type or a changed primary key always makes that table wait, whatever you choose."
    >
      <fieldset className="space-y-2">
        <legend className="sr-only">Schema change policy</legend>
        {POLICY_CHOICES.map((p) => (
          <div key={p.value}>
            <label className="flex cursor-pointer items-center gap-2 text-sm font-medium">
              <input
                type="radio"
                name={`schema-change-policy-${connectorId}`}
                checked={choice === p.value}
                onChange={() => setChoice(p.value)}
              />
              {p.label}
            </label>
            <p className="ml-6 text-xs text-muted-foreground">{p.line}</p>
            {p.value === "apply_all" && !listsTables ? (
              <p className="ml-6 text-xs text-muted-foreground">{APPLY_ALL_NOTE}</p>
            ) : null}
          </div>
        ))}
      </fieldset>
      <div className="mt-3 flex items-center gap-2">
        <Button
          type="button"
          size="sm"
          disabled={choice === policy || save.status === "pending"}
          onClick={async () => {
            if (await save.run(choice)) onChanged()
          }}
        >
          {save.status === "pending" ? "Saving…" : "Save policy"}
        </Button>
        {save.status === "error" ? (
          <p role="alert" className="text-xs text-destructive">
            {save.error.message}
          </p>
        ) : null}
      </div>
    </SectionCard>
  )
}

/**
 * The connector's "Schema changes" tab (`SRC-8`, task 10): the policy, what
 * waits for a decision (one Approve per table, or none when the API says the
 * table cannot be approved), a dismissable notice for a table "apply all"
 * could not add, and the recent changes. `onWaiting` tells the page which
 * tables wait, so the ingest panel can mark them without a request per
 * table; `onChanged` tells it to reload the connector (the pause may have
 * lifted, the policy changed).
 */
export function ConnectorSchemaChangesPanel({
  connectorId,
  connectorType,
  policy,
  onChanged,
  onWaiting,
}: {
  connectorId: string
  connectorType: string
  policy: SchemaChangePolicy
  onChanged: () => void
  onWaiting?: (tables: Set<string>, groups: number) => void
}) {
  const list = useService((s) => connectorService.listSchemaChanges(connectorId, s), [connectorId], {
    keepDataOnReload: true,
  })
  const data = list.data
  React.useEffect(() => {
    if (data) onWaiting?.(waitingTables(data), groupByTable(data.pending.filter((c) => !isNotice(c))).length)
  }, [data, onWaiting])

  const done = () => {
    list.reload()
    onChanged()
  }
  const inactiveSince = (c: SchemaChange) =>
    data?.inactiveColumns.find((i) => i.objectName === c.objectName && i.columnName === c.columnName)
      ?.inactiveSince ?? null

  return (
    <div className="grid items-start gap-3 xl:grid-cols-[minmax(0,3fr)_minmax(0,2fr)]">
      <div className="flex min-w-0 flex-col gap-3">
        <SectionCard size="sm" title="Waiting for a decision">
          {list.status === "loading" ? (
            <LoadingSkeleton rows={2} />
          ) : list.status === "error" ? (
            <ErrorState error={list.error} onRetry={list.reload} />
          ) : list.data.pending.length === 0 ? (
            <p className="text-sm text-muted-foreground">Nothing is waiting for a decision.</p>
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {groupByTable(list.data.pending).map((group) => (
                <PendingGroup key={group.objectName} connectorId={connectorId} group={group} onDone={done} />
              ))}
            </ul>
          )}
        </SectionCard>
        <SectionCard size="sm" title="Recent changes">
          {list.status !== "success" ? (
            <p className="text-sm text-muted-foreground">
              {list.status === "loading" ? "Loading…" : "Could not be loaded."}
            </p>
          ) : list.data.recent.length === 0 ? (
            <p className="text-sm text-muted-foreground">No schema change has been recorded yet.</p>
          ) : (
            <ul className="divide-y divide-border rounded-md border border-border">
              {list.data.recent.map((c) => (
                <RecentChange key={c.id} c={c} inactiveSince={inactiveSince(c)} />
              ))}
            </ul>
          )}
        </SectionCard>
      </div>
      <PolicyCard connectorId={connectorId} connectorType={connectorType} policy={policy} onChanged={onChanged} />
    </div>
  )
}
