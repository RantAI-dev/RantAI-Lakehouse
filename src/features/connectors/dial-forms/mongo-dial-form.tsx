"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import { RequiredMarker } from "@/components/ui/required"
import { RequiredLegend } from "@/components/patterns/required-legend"
import type { MongoDial } from "@/services/contracts/connectors"

/**
 * `dial` for the `mongodb` adapter. `MongoDial::direct_connection` MUST
 * be `true` server-side (`ingest_spec.rs`, `Dial::parse`) — this build
 * refuses `mongodb+srv` and replica-set discovery outright, so this form
 * renders no discovery toggle at all, only the fixed, honest statement
 * below and the explicit seed-host list `directConnection: true` dials.
 * `username` is a literal field, never a `secretRef` picker — same
 * reasoning as `SqlDialForm`'s `user` field.
 */
export function MongoDialForm({
  value,
  onChange,
  showLegend = true,
}: {
  value: MongoDial | null
  onChange: (next: MongoDial) => void
  /** UX-1-R7: a parent step can opt out of its own legend. */
  showLegend?: boolean
}) {
  const dial: MongoDial = value ?? {
    hosts: [""],
    database: "",
    username: "",
    directConnection: true,
  }
  function set<K extends keyof MongoDial>(key: K, v: MongoDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  function setHost(index: number, host: string) {
    set(
      "hosts",
      dial.hosts.map((h, i) => (i === index ? host : h))
    )
  }
  function addHost() {
    set("hosts", [...dial.hosts, ""])
  }
  function removeHost(index: number) {
    set(
      "hosts",
      dial.hosts.filter((_, i) => i !== index)
    )
  }

  return (
    <div className="grid gap-3">
      {/* UX-1-R7: opt out when a parent step already explains the marker. */}
      {showLegend ? <RequiredLegend /> : null}
      <p className="text-xs text-muted-foreground">
        Direct connection to explicit seed hosts only — this build refuses `mongodb+srv` and
        replica-set discovery; there is no toggle for either.
      </p>
      <div className="space-y-2">
        <p className="flex items-center gap-1.5 text-sm font-medium">Seed hosts (host:port) <RequiredMarker /></p>
        {/* UX-1-R5: `Dial::parse` runs `check_hostname` over EVERY entry, so
            every rendered row is required — not just the first. Each row
            gets a visible associated label; Remove drops an unused row. */}
        <p className="text-xs text-muted-foreground">
          Every listed seed host is dialed and must be filled in; use Remove for a row you do not need.
        </p>
        {dial.hosts.map((host, index) => (
          <div key={index} className="flex items-center gap-2">
            <Label htmlFor={`mongo-dial-host-${index}`} required className="shrink-0 text-xs">
              Seed host {index + 1}
            </Label>
            <Input
              id={`mongo-dial-host-${index}`}
              aria-required
              className="min-w-0 flex-1"
              value={host}
              onChange={(e) => setHost(index, e.target.value)}
              placeholder="mongo-0.internal:27017"
              autoComplete="off"
            />
            {dial.hosts.length > 1 ? (
              <button
                type="button"
                className="text-xs text-muted-foreground underline"
                onClick={() => removeHost(index)}
              >
                Remove
              </button>
            ) : null}
          </div>
        ))}
        <button type="button" className="text-xs text-primary underline" onClick={addHost}>
          Add host
        </button>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="mongo-dial-database" required>Database</Label>
          <Input
            id="mongo-dial-database"
            aria-required
            value={dial.database}
            onChange={(e) => set("database", e.target.value)}
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          {/* A literal username, never a secretRef picker -- same
              reasoning as SqlDialForm's User field. */}
          <Label htmlFor="mongo-dial-username" required>Username</Label>
          <Input
            id="mongo-dial-username"
            aria-required
            value={dial.username}
            onChange={(e) => set("username", e.target.value)}
            autoComplete="off"
          />
        </div>
      </div>
    </div>
  )
}
