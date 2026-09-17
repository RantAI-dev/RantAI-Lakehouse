"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { SqlDial } from "@/services/contracts/connectors"

/**
 * `dial` for the `sql` adapter. `Dial::parse` validates this shape
 * server-side on every `PUT /api/connectors/{id}/ingest-spec`; this form
 * adds no client-side rule the server does not already enforce (no
 * required-field checks, no port-range checks — the server's own
 * `deny_unknown_fields` struct is the single source of truth).
 */
export function SqlDialForm({
  value,
  onChange,
}: {
  value: SqlDial | null
  onChange: (next: SqlDial) => void
}) {
  const dial: SqlDial = value ?? { driver: "postgres", host: "", port: 5432, database: "", user: "", sslMode: null }
  function set<K extends keyof SqlDial>(key: K, v: SqlDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <div className="space-y-1.5">
        <Label htmlFor="sql-dial-driver">Driver</Label>
        <select
          id="sql-dial-driver"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.driver}
          onChange={(e) => set("driver", e.target.value as SqlDial["driver"])}
        >
          <option value="postgres">PostgreSQL</option>
          <option value="mysql">MySQL</option>
          <option value="mssql">SQL Server</option>
        </select>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sql-dial-host">Host</Label>
        <Input id="sql-dial-host" value={dial.host} onChange={(e) => set("host", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sql-dial-port">Port</Label>
        <Input
          id="sql-dial-port"
          type="number"
          value={dial.port}
          onChange={(e) => set("port", Number(e.target.value))}
        />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sql-dial-database">Database</Label>
        <Input id="sql-dial-database" value={dial.database} onChange={(e) => set("database", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        {/* WS3 plan review X5: user is a LITERAL string, never a secretRef
            picker -- a username is not a credential. */}
        <Label htmlFor="sql-dial-user">User</Label>
        <Input id="sql-dial-user" value={dial.user} onChange={(e) => set("user", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sql-dial-ssl-mode">SSL mode (optional)</Label>
        <Input
          id="sql-dial-ssl-mode"
          value={dial.sslMode ?? ""}
          onChange={(e) => set("sslMode", e.target.value === "" ? null : e.target.value)}
        />
      </div>
    </div>
  )
}
