"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { CdcDial, SqlDriver } from "@/services/contracts/connectors"
import { DEFAULT_SQL_PORT } from "./sql-dial-form"

/**
 * `dial` for the `cdc` adapter — `SqlDialForm`'s fields plus
 * `slotName`/`publicationName`, both required server-side with no
 * fallback (`ingest_spec.rs`'s `CdcDial`, no `Option`). This form does
 * not add a client-side required-field rule for either — `Dial::parse`
 * already rejects an empty/missing value with its own error text on
 * every `PUT`, and duplicating that check here would risk drifting from
 * it.
 *
 * `driver` works as in `SqlDialForm`: the type already names the database,
 * so the form starts there and only asks when a stored dial disagrees.
 */
export function CdcDialForm({
  value,
  onChange,
  driver,
}: {
  value: CdcDial | null
  onChange: (next: CdcDial) => void
  driver?: SqlDriver
}) {
  const initialDriver = driver ?? "postgres"
  const dial: CdcDial = value ?? {
    driver: initialDriver,
    host: "",
    port: DEFAULT_SQL_PORT[initialDriver],
    database: "",
    user: "",
    slotName: "",
    publicationName: "",
    serverId: null,
  }
  const askDriver = !driver || dial.driver !== driver
  function set<K extends keyof CdcDial>(key: K, v: CdcDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {askDriver ? (
        <div className="space-y-1.5 sm:col-span-2">
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
      ) : null}
      <div className="grid gap-3 sm:col-span-2 sm:grid-cols-[minmax(0,1fr)_7rem]">
        <div className="space-y-1.5">
          <Label htmlFor="cdc-dial-host">Host</Label>
          <Input
            id="cdc-dial-host"
            value={dial.host}
            onChange={(e) => set("host", e.target.value)}
            placeholder="db.internal.example.com"
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="cdc-dial-port">Port</Label>
          <Input
            id="cdc-dial-port"
            type="number"
            value={dial.port}
            onChange={(e) => set("port", Number(e.target.value))}
            autoComplete="off"
          />
        </div>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-database">Database</Label>
        <Input
          id="cdc-dial-database"
          value={dial.database}
          onChange={(e) => set("database", e.target.value)}
          placeholder="sales"
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-user">User</Label>
        <Input
          id="cdc-dial-user"
          value={dial.user}
          onChange={(e) => set("user", e.target.value)}
          placeholder="lakehouse_cdc"
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-slot-name">Replication slot name</Label>
        <Input
          id="cdc-dial-slot-name"
          value={dial.slotName}
          onChange={(e) => set("slotName", e.target.value)}
          placeholder="lakehouse_slot"
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-publication-name">Publication name</Label>
        <Input
          id="cdc-dial-publication-name"
          value={dial.publicationName}
          onChange={(e) => set("publicationName", e.target.value)}
          placeholder="lakehouse_pub"
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-dial-server-id">Server id (optional)</Label>
        <Input
          id="cdc-dial-server-id"
          type="number"
          value={dial.serverId ?? ""}
          onChange={(e) => set("serverId", e.target.value === "" ? null : Number(e.target.value))}
          autoComplete="off"
        />
      </div>
    </div>
  )
}
