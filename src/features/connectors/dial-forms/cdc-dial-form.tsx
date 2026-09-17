"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { CdcDial } from "@/services/contracts/connectors"

/**
 * `dial` for the `cdc` adapter — `SqlDialForm`'s fields plus
 * `slotName`/`publicationName`, both required server-side with no
 * fallback (`ingest_spec.rs`'s `CdcDial`, no `Option`). This form does
 * not add a client-side required-field rule for either — `Dial::parse`
 * already rejects an empty/missing value with its own error text on
 * every `PUT`, and duplicating that check here would risk drifting from
 * it.
 */
export function CdcDialForm({
  value,
  onChange,
}: {
  value: CdcDial | null
  onChange: (next: CdcDial) => void
}) {
  const dial: CdcDial = value ?? {
    driver: "postgres",
    host: "",
    port: 5432,
    database: "",
    user: "",
    slotName: "",
    publicationName: "",
    serverId: null,
  }
  function set<K extends keyof CdcDial>(key: K, v: CdcDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-driver">Driver</Label>
        <select
          id="cdc-dial-driver"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.driver}
          onChange={(e) => set("driver", e.target.value as CdcDial["driver"])}
        >
          <option value="postgres">PostgreSQL</option>
          <option value="mysql">MySQL</option>
          <option value="mssql">SQL Server</option>
        </select>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-host">Host</Label>
        <Input id="cdc-dial-host" value={dial.host} onChange={(e) => set("host", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-port">Port</Label>
        <Input
          id="cdc-dial-port"
          type="number"
          value={dial.port}
          onChange={(e) => set("port", Number(e.target.value))}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-database">Database</Label>
        <Input id="cdc-dial-database" value={dial.database} onChange={(e) => set("database", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-user">User</Label>
        <Input id="cdc-dial-user" value={dial.user} onChange={(e) => set("user", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-slot-name">Replication slot name</Label>
        <Input id="cdc-dial-slot-name" value={dial.slotName} onChange={(e) => set("slotName", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-publication-name">Publication name</Label>
        <Input
          id="cdc-dial-publication-name"
          value={dial.publicationName}
          onChange={(e) => set("publicationName", e.target.value)}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-server-id">Server id (optional)</Label>
        <Input
          id="cdc-dial-server-id"
          type="number"
          value={dial.serverId ?? ""}
          onChange={(e) => set("serverId", e.target.value === "" ? null : Number(e.target.value))}
        />
      </div>
    </div>
  )
}
