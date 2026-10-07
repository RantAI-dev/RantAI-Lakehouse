"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { SqlDial, SqlDriver } from "@/services/contracts/connectors"

/** The port each driver listens on out of the box. */
export const DEFAULT_SQL_PORT: Record<SqlDriver, number> = {
  postgres: 5432,
  mysql: 3306,
  mssql: 1433,
  oracle: 1521,
}

/**
 * `dial` for the `sql` adapter. `Dial::parse` validates this shape
 * server-side on every `PUT /api/connectors/{id}/ingest-spec`; this form
 * adds no client-side rule the server does not already enforce (no
 * required-field checks, no port-range checks — the server's own
 * `deny_unknown_fields` struct is the single source of truth).
 *
 * `driver` is the one the connector type already implies (PostgreSQL,
 * MySQL, MariaDB, SQL Server). When given, the form starts on that driver
 * and its default port and does not ask again; the select only comes back
 * if a stored dial disagrees with the type, so the mismatch stays visible
 * and fixable.
 */
export function SqlDialForm({
  value,
  onChange,
  driver,
}: {
  value: SqlDial | null
  onChange: (next: SqlDial) => void
  driver?: SqlDriver
}) {
  const initialDriver = driver ?? "postgres"
  const dial: SqlDial = value ?? {
    driver: initialDriver,
    host: "",
    port: DEFAULT_SQL_PORT[initialDriver],
    database: "",
    user: "",
    sslMode: null,
  }
  const askDriver = !driver || dial.driver !== driver
  function set<K extends keyof SqlDial>(key: K, v: SqlDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      {askDriver ? (
        <div className="space-y-1.5 sm:col-span-2">
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
      ) : null}
      <div className="grid gap-3 sm:col-span-2 sm:grid-cols-[minmax(0,1fr)_7rem]">
        <div className="space-y-1.5">
          <Label htmlFor="sql-dial-host">Host</Label>
          <Input
            id="sql-dial-host"
            value={dial.host}
            onChange={(e) => set("host", e.target.value)}
            placeholder="db.internal.example.com"
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="sql-dial-port">Port</Label>
          <Input
            id="sql-dial-port"
            type="number"
            value={dial.port}
            onChange={(e) => set("port", Number(e.target.value))}
            autoComplete="off"
          />
        </div>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sql-dial-database">Database</Label>
        <Input
          id="sql-dial-database"
          value={dial.database}
          onChange={(e) => set("database", e.target.value)}
          placeholder="sales"
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5">
        {/* WS3 plan review X5: user is a LITERAL string, never a secretRef
            picker -- a username is not a credential. */}
        <Label htmlFor="sql-dial-user">User</Label>
        <Input
          id="sql-dial-user"
          value={dial.user}
          onChange={(e) => set("user", e.target.value)}
          placeholder="lakehouse_reader"
          autoComplete="off"
        />
      </div>
      <div className="space-y-1.5 sm:col-span-2">
        <Label htmlFor="sql-dial-ssl-mode">SSL mode (optional)</Label>
        <Input
          id="sql-dial-ssl-mode"
          value={dial.sslMode ?? ""}
          onChange={(e) => set("sslMode", e.target.value === "" ? null : e.target.value)}
          placeholder={dial.driver === "postgres" ? "require" : "Leave blank for the driver default"}
          autoComplete="off"
        />
      </div>
    </div>
  )
}
