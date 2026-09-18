"use client"

import * as React from "react"
import { Building2Icon, CheckIcon } from "lucide-react"
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu"
import { Button } from "@/components/ui/button"

/**
 * Slim tenant row the picker renders. Mirrors the per-tenant fields on
 * `AuthUser.tenants` (`src/services/clients/auth.ts:23`); the picker
 * doesn't need the rest of `TenantSummary` (warehouse name, Lakekeeper
 * id, …) so a narrower local type keeps the prop signature honest.
 */
export type TenantOption = { id: string; name: string; slug: string }

/**
 * Lets an authenticated user pick which tenant they are acting as.
 *
 * Hidden for principals that belong to zero or one tenant — a dropdown
 * with one option is a dead affordance, and the navbar shouldn't surface
 * "you have one tenant" as if that were a choice. Two tenants is the
 * earliest moment a switcher starts to make sense; below that, the
 * single-tenant principal doesn't need to choose.
 *
 * Stateless on purpose: persistence (`localStorage["lh_active_tenant"]`,
 * `apiFetch`'s `X-Tenant` attachment) and the refresh that re-fetches
 * every tenant-scoped list route under the new scope live in
 * `AuthProvider` and the navbar's wiring — this component only owns the
 * trigger, the menu, and the call back into `onSwitch`.
 */
export function TenantSwitcher({
  tenants,
  activeTenantId,
  onSwitch,
}: {
  tenants: TenantOption[]
  activeTenantId: string | null
  onSwitch: (id: string) => void
}) {
  if (tenants.length <= 1) return null

  const active = tenants.find((t) => t.id === activeTenantId)

  return (
    <DropdownMenu>
      <DropdownMenuTrigger
        render={
          <Button variant="ghost" size="sm" className="gap-2">
            <Building2Icon className="size-4" />
            {active?.name ?? "Select tenant"}
          </Button>
        }
      />
      <DropdownMenuContent align="start">
        {tenants.map((tenant) => {
          const isActive = tenant.id === activeTenantId
          return (
            <button
              key={tenant.id}
              type="button"
              role="menuitemradio"
              aria-checked={isActive}
              // Radio-row layout: a check icon reserves its column even
              // when the row isn't the active one (opacity-0 keeps the
              // row's text aligned across the menu), and the hover
              // surface matches the codebase's dropdown item look
              // (`data-highlighted:bg-muted` from `DropdownMenuItem`,
              // reimplemented here because we hand-roll the radio role
              // — the menu primitive's own `MenuItem` is `menuitem`,
              // not `menuitemradio`).
              className="flex w-full items-center gap-2 rounded-md px-2 py-1.5 text-sm outline-none select-none hover:bg-muted focus-visible:bg-muted data-[variant=destructive]:text-destructive"
              onClick={() => onSwitch(tenant.id)}
            >
              <CheckIcon
                className={isActive ? "size-4 opacity-100" : "size-4 opacity-0"}
                aria-hidden
              />
              {tenant.name}
            </button>
          )
        })}
      </DropdownMenuContent>
    </DropdownMenu>
  )
}