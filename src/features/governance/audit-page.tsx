"use client"

import * as React from "react"
import { parseAsString, parseAsStringLiteral, useQueryStates } from "nuqs"
import { DataTable } from "@/components/data-table/data-table"
import { DetailDrawer } from "@/components/patterns/detail-drawer"
import { MetadataList } from "@/components/patterns/metadata-list"
import { PageHeader } from "@/components/patterns/page-header"
import { ErrorState, LoadingSkeleton } from "@/components/patterns/page-states"
import { SelectField } from "@/components/patterns/select-field"
import { OutcomeBadge, Pill } from "@/components/patterns/status-badge"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useDataTable } from "@/hooks/use-data-table"
import { useService } from "@/hooks/use-service"
import { saveBlob } from "@/lib/browser-file"
import { formatCost, formatDateTime } from "@/lib/format"
import {
  ACTOR_KIND_LABEL,
  ENGINE_CATEGORY_LABEL,
} from "@/lib/status"
import { governanceService } from "@/services"
import { toServiceError } from "@/services/errors"
import type { AuditEvent, AuditSearchQuery } from "@/services/contracts/governance"
import { getAuditColumns } from "./audit-columns"

/** The eight outcomes an audit row stores, as the server filters on them. */
const OUTCOMES = [
  { value: "allowed", label: "Allowed" },
  { value: "executed", label: "Executed" },
  { value: "refused", label: "Refused" },
  { value: "needs_confirmation", label: "Needs confirmation" },
  { value: "needs_approval", label: "Needs approval" },
  { value: "approved", label: "Approved" },
  { value: "rejected", label: "Rejected" },
  { value: "failed", label: "Failed" },
] as const

/**
 * The filters and the open event live in the address, so a link reproduces
 * what the sender saw. `event` is the `?event=<id>` link other pages build.
 */
const URL_STATE = {
  actor: parseAsString.withDefault(""),
  action: parseAsString.withDefault(""),
  resourceKind: parseAsString.withDefault(""),
  resourceId: parseAsString.withDefault(""),
  // A value outside the eight reads as no outcome filter.
  outcome: parseAsStringLiteral(OUTCOMES.map((o) => o.value)),
  from: parseAsString.withDefault(""),
  to: parseAsString.withDefault(""),
  event: parseAsString.withDefault(""),
}

const DAY_MS = 24 * 60 * 60 * 1000

/** A date input's `YYYY-MM-DD` as the instant its UTC day starts, or `undefined` for anything else. */
function dayStart(date: string): string | undefined {
  const at = Date.parse(`${date}T00:00:00Z`)
  return /^\d{4}-\d{2}-\d{2}$/.test(date) && !Number.isNaN(at)
    ? `${date}T00:00:00Z`
    : undefined
}

/** The instant the day after `date` starts: the server's `to` is exclusive, so the typed day stays in. */
function dayAfter(date: string): string | undefined {
  return dayStart(date) === undefined
    ? undefined
    : new Date(Date.parse(`${date}T00:00:00Z`) + DAY_MS)
        .toISOString()
        .replace(".000Z", "Z")
}

type AuditFilters = {
  actor: string
  action: string
  resourceKind: string
  resourceId: string
  outcome: string
  from: string
  to: string
}

/** The request a set of filters makes; an empty filter is left out. */
function searchQueryOf(filters: AuditFilters): AuditSearchQuery {
  return {
    actor: filters.actor.trim(),
    action: filters.action.trim(),
    resourceKind: filters.resourceKind.trim(),
    resourceId: filters.resourceId.trim(),
    outcome: filters.outcome,
    from: dayStart(filters.from),
    to: dayAfter(filters.to),
  }
}

/** The sentence under the Export button when the server cut the file. */
function exportCutSentence(total: number | null): string {
  const matching = total === null ? "more" : total.toLocaleString("en-US")
  return `The export holds the newest 100,000 of ${matching} matching events. Narrow the time range to get the rest.`
}

export function EventDetail({ event }: { readonly event: AuditEvent }) {
  return (
    <>
      <div className="flex flex-wrap items-center gap-2">
        <OutcomeBadge outcome={event.outcome} />
        <span className="text-xs text-muted-foreground">
          {formatDateTime(event.at)}
        </span>
      </div>
      <MetadataList
        items={[
          {
            label: "Actor",
            value: `${event.actor} (${ACTOR_KIND_LABEL[event.actorKind]})`,
          },
          {
            label: "Delegated actor",
            value: event.delegatedActor ?? "—",
          },
          {
            label: "Tenant",
            value: <span className="font-mono text-xs">{event.tenant}</span>,
          },
          { label: "Action", value: event.action },
          { label: "Resource", value: event.resource },
          { label: "Policy decision", value: event.policyDecision },
          // QA LH-DATA-15-03 F1 (plan D9): a grant's entry says who got
          // which permission; only the rows that carry a detail show it.
          ...(event.detail ? [{ label: "Detail", value: event.detail }] : []),
          ...(event.engineCategory
            ? [
                {
                  label: "Engine",
                  value: ENGINE_CATEGORY_LABEL[event.engineCategory],
                },
              ]
            : []),
          {
            label: "Estimated cost",
            value: event.estimatedCost != null ? formatCost(event.estimatedCost) : "—",
          },
          {
            label: "Actual cost",
            value: event.actualCost != null ? formatCost(event.actualCost) : "—",
          },
          { label: "Approval", value: event.approvalId ?? "—" },
          { label: "Timestamp", value: formatDateTime(event.at) },
        ]}
      />
      {event.obligations.length > 0 ? (
        <div>
          <p className="text-xs font-medium text-muted-foreground">Obligations</p>
          <div className="mt-1.5 flex flex-wrap gap-1.5">
            {event.obligations.map((o) => (
              <Pill key={o} tone="neutral">{o}</Pill>
            ))}
          </div>
        </div>
      ) : null}
    </>
  )
}

const FIELD_LABEL = "flex flex-col gap-1 text-xs text-muted-foreground"

export function AuditPage() {
  const [url, setUrl] = useQueryStates(URL_STATE)
  const filters: AuditFilters = {
    actor: url.actor,
    action: url.action,
    resourceKind: url.resourceKind,
    resourceId: url.resourceId,
    outcome: url.outcome ?? "",
    from: url.from,
    to: url.to,
  }
  // The form is a draft; Search copies it into the address, and the address
  // is what the request is built from.
  const [draft, setDraft] = React.useState<AuditFilters>(filters)
  const filtersKey = JSON.stringify(filters)
  React.useEffect(() => {
    setDraft(filters)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [filtersKey])
  const query = React.useMemo(
    () => searchQueryOf(JSON.parse(filtersKey) as AuditFilters),
    [filtersKey]
  )

  // The first page follows the filters; later pages are held here and thrown
  // away when the filters change (`key` says which filters they belong to).
  const first = useService((s) => governanceService.searchAudit(query, s), [
    filtersKey,
  ])
  const [more, setMore] = React.useState<{
    key: string
    events: AuditEvent[]
    cursor: string | null
  } | null>(null)
  const [loadingMore, setLoadingMore] = React.useState(false)
  const [moreError, setMoreError] = React.useState<string | null>(null)
  const extra = more?.key === filtersKey ? more : null
  const events = React.useMemo(
    () =>
      first.status === "success"
        ? [...first.data.events, ...(extra?.events ?? [])]
        : [],
    [first.status, first.data, extra]
  )
  const cursor =
    first.status === "success"
      ? extra
        ? extra.cursor
        : first.data.nextCursor
      : null

  async function loadMore() {
    if (!cursor || loadingMore) return
    setLoadingMore(true)
    setMoreError(null)
    try {
      const page = await governanceService.searchAudit({ ...query, cursor })
      setMore((prev) => ({
        key: filtersKey,
        events: [
          ...(prev?.key === filtersKey ? prev.events : []),
          ...page.events,
        ],
        cursor: page.nextCursor,
      }))
    } catch (err) {
      setMoreError(toServiceError(err).message)
    } finally {
      setLoadingMore(false)
    }
  }

  // The open event: one on the loaded pages, or, for a link to an older event,
  // the one the server returns by id.
  const onPage = url.event ? events.find((e) => e.id === url.event) : undefined
  const needsFetch = url.event !== "" && first.status === "success" && !onPage
  const linked = useService(
    (s) =>
      needsFetch
        ? governanceService.getAuditEvent(url.event, s)
        : Promise.resolve(null),
    [url.event, needsFetch]
  )
  const selected = onPage ?? (needsFetch ? linked.data : null)
  // A link to an event the server does not have opens nothing; the page says
  // so and keeps the list. GOV-1 review T-2: only a 404 (the client returns
  // `null`) says that. Any other failure says what failed, because it tells
  // nothing about whether the event exists.
  const linkedGone =
    needsFetch && linked.status === "success" && linked.data === null
  const linkedError =
    needsFetch && linked.status === "error"
      ? toServiceError(linked.error).message
      : null

  const columns = React.useMemo(
    () => getAuditColumns({ onSelect: (e) => void setUrl({ event: e.id }) }),
    // GOV-1 review T-1: `setUrl` is stable, so this still builds once.
    [setUrl]
  )

  const { table } = useDataTable({
    data: events,
    columns,
    paginationMode: "infinite",
    manualPagination: false,
    manualSorting: true,
    manualFiltering: true,
    // The server returns the events newest first; sorting the loaded pages in
    // the browser would reorder only what is loaded.
    enableSorting: false,
    persistKey: "/audit",
    initialState: {
      columnPinning: { right: ["actions"] },
    },
  })

  const [exporting, setExporting] = React.useState(false)
  const [exportNote, setExportNote] = React.useState<
    { kind: "cut" | "failed"; text: string } | null
  >(null)
  async function exportCsv() {
    setExporting(true)
    setExportNote(null)
    try {
      const file = await governanceService.exportAuditCsv(query)
      saveBlob(file.blob, file.fileName)
      if (file.truncated) {
        setExportNote({ kind: "cut", text: exportCutSentence(file.total) })
      }
    } catch (err) {
      setExportNote({ kind: "failed", text: toServiceError(err).message })
    } finally {
      setExporting(false)
    }
  }

  const hasFilter = Object.values(filters).some((v) => v !== "")
  const set = (key: keyof AuditFilters) => (value: string) =>
    setDraft((d) => ({ ...d, [key]: value }))

  return (
    <div className="flex flex-col gap-4">
      <PageHeader
        title="Audit"
        description="Immutable events with actor chains, policy decisions, cost, and approvals."
        actions={
          <Button
            variant="outline"
            size="sm"
            disabled={exporting}
            onClick={() => void exportCsv()}
          >
            Export CSV
          </Button>
        }
      />
      <form
        className="flex flex-col gap-2 rounded-md border bg-card px-3 py-3"
        onSubmit={(e) => {
          e.preventDefault()
          void setUrl({
            actor: draft.actor.trim(),
            action: draft.action.trim(),
            resourceKind: draft.resourceKind.trim(),
            resourceId: draft.resourceId.trim(),
            outcome: OUTCOMES.find((o) => o.value === draft.outcome)?.value ?? null,
            from: draft.from,
            to: draft.to,
          })
        }}
      >
        <div className="grid gap-3 sm:grid-cols-2 lg:grid-cols-4">
          <label className={FIELD_LABEL}>
            Person
            <Input
              aria-label="Person"
              value={draft.actor}
              onChange={(e) => set("actor")(e.target.value)}
              placeholder="Person id"
              className="h-8 text-xs"
            />
          </label>
          <label className={FIELD_LABEL}>
            Action
            <Input
              aria-label="Action"
              value={draft.action}
              onChange={(e) => set("action")(e.target.value)}
              placeholder="dashboard.share or dashboard."
              className="h-8 text-xs"
            />
          </label>
          <label className={FIELD_LABEL}>
            Object kind
            <Input
              aria-label="Object kind"
              value={draft.resourceKind}
              onChange={(e) => set("resourceKind")(e.target.value)}
              placeholder="dashboard"
              className="h-8 text-xs"
            />
          </label>
          <label className={FIELD_LABEL}>
            Object id
            <Input
              aria-label="Object id"
              value={draft.resourceId}
              onChange={(e) => set("resourceId")(e.target.value)}
              className="h-8 text-xs"
            />
          </label>
          <label className={FIELD_LABEL}>
            Outcome
            <SelectField
              aria-label="Outcome"
              value={draft.outcome}
              onValueChange={set("outcome")}
              options={[{ value: "", label: "Any outcome" }, ...OUTCOMES]}
              size="sm"
              className="w-full text-xs"
            />
          </label>
          <label className={FIELD_LABEL}>
            From
            <Input
              type="date"
              aria-label="From date"
              value={draft.from}
              onChange={(e) => set("from")(e.target.value)}
              className="h-8 text-xs"
            />
          </label>
          <label className={FIELD_LABEL}>
            To
            <Input
              type="date"
              aria-label="To date"
              value={draft.to}
              onChange={(e) => set("to")(e.target.value)}
              className="h-8 text-xs"
            />
          </label>
          <div className="flex items-end gap-2">
            <Button type="submit" size="sm">
              Search
            </Button>
            {hasFilter ? (
              <Button
                type="button"
                variant="ghost"
                size="sm"
                onClick={() =>
                  void setUrl({
                    actor: null,
                    action: null,
                    resourceKind: null,
                    resourceId: null,
                    outcome: null,
                    from: null,
                    to: null,
                  })
                }
              >
                Clear filters
              </Button>
            ) : null}
          </div>
        </div>
        <p className="text-xs text-muted-foreground">
          Person is the id of a user, service or agent. An action matches whole;
          end it with a dot (<span className="font-mono">dashboard.</span>) to
          match every action of that family. Dates are UTC days, and the To day
          is included.
        </p>
      </form>
      {exportNote ? (
        <p
          role={exportNote.kind === "failed" ? "alert" : "status"}
          className="rounded-md border bg-muted/40 px-3 py-2 text-sm"
        >
          {exportNote.text}
        </p>
      ) : null}
      {linkedGone ? (
        <p role="status" className="rounded-md border bg-muted/40 px-3 py-2 text-sm">
          No audit event has the id {url.event}.
        </p>
      ) : null}
      {linkedError ? (
        <p role="alert" className="rounded-md border bg-muted/40 px-3 py-2 text-sm">
          {linkedError}
        </p>
      ) : null}
      {first.status === "loading" ? <LoadingSkeleton /> : null}
      {first.status === "error" ? (
        <ErrorState error={first.error} onRetry={first.reload} />
      ) : null}
      {first.status === "success" ? (
        <>
          <DataTable
            table={table}
            infinite={{
              // A button loads the next page, so the scroll never does.
              onLoadMore: () => {},
              hasNextPage: cursor !== null,
              isFetchingNextPage: loadingMore,
              // GOV-1 review T-3: the server gives no total, so none is passed.
              loadedCount: events.length,
            }}
          />
          {events.length === 0 ? (
            <p className="py-4 text-center text-sm text-muted-foreground">
              No audit event matches these filters.
            </p>
          ) : null}
          {moreError ? (
            <p role="alert" className="text-center text-sm text-destructive">
              {moreError}
            </p>
          ) : null}
          {cursor !== null ? (
            <div className="flex justify-center">
              <Button
                variant="outline"
                size="sm"
                disabled={loadingMore}
                onClick={() => void loadMore()}
              >
                Load more
              </Button>
            </div>
          ) : null}
        </>
      ) : null}
      <DetailDrawer
        open={selected != null}
        onOpenChange={(open) => {
          if (!open) void setUrl({ event: null })
        }}
        title={selected ? selected.action : ""}
        description={selected ? `Audit event ${selected.id}` : undefined}
        wide
      >
        {selected ? <EventDetail event={selected} /> : null}
      </DetailDrawer>
    </div>
  )
}
