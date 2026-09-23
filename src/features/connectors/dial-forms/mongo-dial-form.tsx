"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
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
}: {
  value: MongoDial | null
  onChange: (next: MongoDial) => void
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
      <p className="text-xs text-muted-foreground">
        Direct connection to explicit seed hosts only — this build refuses `mongodb+srv` and
        replica-set discovery; there is no toggle for either.
      </p>
      <div className="space-y-2">
        <p className="text-sm font-medium">Seed hosts (host:port)</p>
        {dial.hosts.map((host, index) => (
          <div key={index} className="flex items-center gap-2">
            <Input
              id={`mongo-dial-host-${index}`}
              aria-label={`Seed host ${index + 1}`}
              value={host}
              onChange={(e) => setHost(index, e.target.value)}
              placeholder="mongo-0.internal:27017"
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
          <Label htmlFor="mongo-dial-database">Database</Label>
          <Input
            id="mongo-dial-database"
            value={dial.database}
            onChange={(e) => set("database", e.target.value)}
          />
        </div>
        <div className="space-y-1.5">
          {/* A literal username, never a secretRef picker -- same
              reasoning as SqlDialForm's User field. */}
          <Label htmlFor="mongo-dial-username">Username</Label>
          <Input
            id="mongo-dial-username"
            value={dial.username}
            onChange={(e) => set("username", e.target.value)}
          />
        </div>
      </div>
    </div>
  )
}
