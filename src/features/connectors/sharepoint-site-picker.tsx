"use client"

import * as React from "react"
import { ErrorState } from "@/components/patterns/page-states"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useDiscoveryList } from "./use-discovery"

/** Rows shown of a long list: the person narrows it with the filter. */
const SHOWN = 100

/**
 * The site picker of a SharePoint connector (`SRC-3` decision 5, slice 5 review BLOCKER 14): "the lists of the
 * sites the user picks". The sites to choose from are the tenant's own, read by the orchestrator through
 * discovery with `schema: "fields"` (Graph's site search); each is shown by its title, and the pick that is
 * kept is its id. It owns no state of the connector: `picked` are the site ids in the dial's `objects`,
 * `onToggle` adds or takes one away, and they become effective when the connector's tables are saved, because
 * discovery lists the lists of the SAVED sites.
 */
export function SharepointSitePicker({
  connectorId,
  picked,
  savedPicked,
  onToggle,
  pollMs,
}: {
  connectorId: string
  picked: readonly string[]
  savedPicked: readonly string[]
  onToggle: (siteId: string) => void
  pollMs?: number
}) {
  const sites = useDiscoveryList(connectorId, pollMs)
  const [filter, setFilter] = React.useState("")
  const answer = sites.data?.state === "done" ? sites.data : null
  const found = (answer?.objects ?? []).filter((o) => o.kind === "site")
  const needle = filter.trim().toLowerCase()
  const shown = found.filter((o) => needle === "" || (o.label ?? o.name).toLowerCase().includes(needle))
  const foundIds = new Set(found.map((o) => o.name))
  // A site that is picked but not in the list (not read yet, or no longer found) is still shown, by its id.
  const unlisted = picked.filter((id) => !foundIds.has(id))
  const unsaved = JSON.stringify(picked) !== JSON.stringify(savedPicked)

  return (
    <div className="space-y-2 rounded-md border border-border p-3" aria-label="Sites">
      <p className="text-sm font-medium">Sites</p>
      <p className="text-xs text-muted-foreground">
        SharePoint lists live in sites. Pick the sites to read, save, then find tables to see their lists. With no site
        picked, the first 100 sites found are used.
      </p>
      <div className="flex items-center gap-2">
        <Button
          type="button"
          size="sm"
          variant="outline"
          disabled={sites.status === "pending"}
          onClick={() => void sites.run({ schema: "fields" })}
        >
          {sites.status === "pending" ? "Reading the sites…" : "Find sites"}
        </Button>
        <span className="text-xs text-muted-foreground" data-testid="sites-picked">
          {picked.length === 0 ? "No site picked" : `${picked.length} picked`}
        </span>
      </div>
      {sites.status === "error" ? <ErrorState error={sites.error} /> : null}
      {sites.data?.state === "failed" ? (
        <p className="text-xs text-muted-foreground">Cannot read the sites · {sites.data.reason ?? "The sites could not be read."}</p>
      ) : null}
      {found.length > 0 ? (
        <>
          <Input
            aria-label="Filter sites"
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            placeholder={`Filter ${found.length} sites`}
            className="h-7 text-xs"
            autoComplete="off"
          />
          <ul className="max-h-40 space-y-0.5 overflow-y-auto">
            {shown.slice(0, SHOWN).map((o) => (
              <li key={o.name}>
                <label className="flex cursor-pointer items-center gap-2 rounded px-1 py-0.5 text-xs hover:bg-muted/50" title={o.name}>
                  <input type="checkbox" checked={picked.includes(o.name)} onChange={() => onToggle(o.name)} />
                  <span className="min-w-0 flex-1 truncate">{o.label ?? o.name}</span>
                </label>
              </li>
            ))}
          </ul>
          {shown.length > SHOWN ? <p className="text-xs text-muted-foreground">Showing {SHOWN} of {shown.length}; filter to narrow.</p> : null}
        </>
      ) : null}
      {unlisted.length > 0 ? (
        <ul className="space-y-0.5">
          {unlisted.map((id) => (
            <li key={id}>
              <label className="flex cursor-pointer items-center gap-2 rounded px-1 py-0.5 text-xs hover:bg-muted/50">
                <input type="checkbox" checked onChange={() => onToggle(id)} />
                <span className="min-w-0 flex-1 truncate font-mono">{id}</span>
              </label>
            </li>
          ))}
        </ul>
      ) : null}
      {unsaved ? (
        <p className="text-xs text-muted-foreground" data-testid="sites-unsaved">
          Save the tables to list the lists of these sites.
        </p>
      ) : null}
    </div>
  )
}
