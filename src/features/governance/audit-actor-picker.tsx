"use client"

import * as React from "react"

import {
  Faceted,
  FacetedBadgeList,
  FacetedEmpty,
  FacetedGroup,
  FacetedInput,
  FacetedItem,
  FacetedList,
  FacetedTrigger,
} from "@/components/ui/faceted"
import { Command } from "@/components/ui/command"
import { PopoverContent } from "@/components/ui/popover"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { useService } from "@/hooks/use-service"
import { ACTOR_KIND_LABEL } from "@/lib/status"
import { governanceService } from "@/services"
import type { AuditActor } from "@/services/contracts/governance"

/**
 * The audit trail's principals (`plan D2`), read through
 * `governanceService.listAuditActors`. `actors` is `null` while the read is
 * in flight and when it failed — the picker then falls back to free text,
 * because a person without `identity:read` must still be able to filter the
 * trail the way the address always allowed. `note` says why there is no
 * list, and is `null` while loading and once it is ready.
 */
export function useAuditActors(): { actors: AuditActor[] | null; note: string | null } {
  const state = useService((signal) => governanceService.listAuditActors(signal), [])
  const actors = state.status === "success" ? state.data.actors : null
  const note =
    state.status === "error"
      ? "Could not read the trail's people; type the principal id."
      : null
  return { actors, note }
}

/** Whether an option matches what the person typed: a substring of the name the trail shows or of the id it sends. */
function optionMatches(option: { value: string; label: string }, search: string): boolean {
  const needle = search.trim().toLowerCase()
  if (needle === "") return true
  return option.label.toLowerCase().includes(needle) || option.value.toLowerCase().includes(needle)
}

/**
 * The person filter's picker (`plan D2`): a combobox over the principals the
 * trail records (`GET /api/governance/audit/actors`, at most 500), searched
 * by the name the trail shows, picked by `actorId` — the value the audit
 * search sends as `actor`, so the wire parameter does not change. It never
 * invents a person: a held id the list does not name (an old link's filter,
 * or a principal past the 500 cap) is offered as its own marked entry, and a
 * failed read goes back to the free-text id entry the field was, with the
 * reason, rather than an empty list that would read as "the trail has no
 * people".
 */
export function AuditActorPicker({
  label,
  value,
  onChange,
  actors,
  note,
}: {
  /** The field's name, for the trigger's, the fallback input's and the search box's labels. */
  label: string
  /** The picked principal id, as the draft holds it; empty means no filter. */
  value: string
  onChange: (next: string) => void
  /** The trail's principals; `null` while loading or when the read failed. */
  actors: AuditActor[] | null
  /** Why there is no list (a failed read); `null` while loading or once ready. */
  note: string | null
}) {
  const [search, setSearch] = React.useState("")
  const options = React.useMemo(() => {
    const known = actors ?? []
    return [
      ...known.map((actor) => ({
        value: actor.actorId,
        label: actor.actor,
        kind: ACTOR_KIND_LABEL[actor.actorKind] ?? actor.actorKind,
      })),
      // A person the filter already holds that the trail's list does not
      // name: shown, selectable back, never silently replaced.
      ...(value !== "" && !known.some((actor) => actor.actorId === value)
        ? [{ value, label: `${value} (not in the trail)`, kind: null as string | null }]
        : []),
    ]
  }, [actors, value])
  // The picker narrows the list itself, on the name and the id, instead of
  // letting cmdk's filter do it: cmdk unmounts the options a search filters
  // out and drops them from its index, so replacing the search text — not
  // just extending it — would search a shortened index and read "No person
  // matches." against people the trail has. The search box stays cmdk's
  // (`FacetedInput`, controlled), so keyboard behaviour is unchanged.
  const visible = React.useMemo(
    () => options.filter((option) => optionMatches(option, search)),
    [options, search]
  )

  if (actors === null) {
    return (
      <div className="grid gap-1">
        <Input
          aria-label={label}
          value={value}
          onChange={(e) => onChange(e.target.value)}
          placeholder="Principal id"
          className="h-8 text-xs"
        />
        {note ? <p className="text-xs text-muted-foreground">{note}</p> : null}
      </div>
    )
  }

  return (
    <Faceted value={value === "" ? undefined : value} onValueChange={(next) => onChange(next ?? "")}>
      <FacetedTrigger asChild>
        <Button
          type="button"
          variant="outline"
          size="sm"
          className="h-8 w-full min-w-0 justify-between font-normal text-xs"
          aria-label={label}
          title={value === "" ? undefined : value}
        >
          <FacetedBadgeList
            options={options}
            placeholder="Pick a person"
            className="min-w-0 flex-1"
            badgeClassName="max-w-full [&>span]:min-w-0"
          />
        </Button>
      </FacetedTrigger>
      {/* `FacetedContent` hard-codes its inner `Command` and forwards nothing
          to it; the filter is turned off here (see above), which needs a prop
          on that `Command`, so the content is built on `PopoverContent` +
          `Command` directly, with the same classes. */}
      <PopoverContent align="start" className="w-64 p-0">
        <Command shouldFilter={false}>
          <FacetedInput
            value={search}
            onValueChange={setSearch}
            placeholder="Search people"
            aria-label={`Search ${label}`}
          />
          <FacetedList>
            {visible.length === 0 ? (
              // cmdk's `FacetedEmpty` keys on its own filtered count, which
              // is off here; the picker's own count is the truth.
              <p className="py-6 text-center text-sm text-muted-foreground">
                No person matches.
              </p>
            ) : (
              <FacetedGroup>
                {visible.map((option) => (
                  <FacetedItem key={option.value} value={option.value}>
                    <span className="min-w-0 flex-1 truncate text-xs leading-snug">
                      {option.label}
                      {option.kind ? (
                        <span className="text-muted-foreground"> · {option.kind}</span>
                      ) : null}
                    </span>
                  </FacetedItem>
                ))}
              </FacetedGroup>
            )}
          </FacetedList>
        </Command>
      </PopoverContent>
    </Faceted>
  )
}
