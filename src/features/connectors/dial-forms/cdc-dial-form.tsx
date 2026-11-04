"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { CdcDial, SqlDriver } from "@/services/contracts/connectors"
import { DEFAULT_SQL_PORT } from "./sql-dial-form"

/**
 * `dial` for the `cdc` adapter — `SqlDialForm`'s fields plus the setting of
 * the driver (`SRC-4` decision S3-a): `slotName` and `publicationName` for
 * PostgreSQL only, `serverId` for MySQL only, `pdbName` for Oracle only, nothing for SQL Server. A field
 * the driver does not use is not in the dial at all (the server refuses a
 * present one). This form does not add a client-side required-field rule:
 * `Dial::parse` already rejects a missing value with its own fixed sentence
 * on every `PUT`, and duplicating that check here would risk drifting from it.
 *
 * `driver` works as in `SqlDialForm`: the type already names the database,
 * so the form starts there and only asks when a stored dial disagrees.
 */
export const DEFAULT_CDC_SSL_MODE = "require"

/** The TLS modes a person can choose (`SEC-14`: a stored credential is not dialled in plain text unless
 * `disable` is chosen, and it is labelled as such). */
const CDC_SSL_MODES: { value: string; label: string }[] = [
  { value: "require", label: "Require (encrypted, certificate not checked)" },
  { value: "verify-ca", label: "Verify CA" },
  { value: "verify-full", label: "Verify CA and host name" },
  { value: "disable", label: "Disable (not encrypted)" },
]

/** Oracle takes two modes (`SRC-4` slice 6, S6-c): its JDBC driver does TLS only over TCPS with a Java trust store
 * (`SRC-37`), so `require` is Oracle's native network encryption (AES256, SHA256 checksum: encrypted, the server
 * not authenticated) and the verify modes are not offered. */
const ORACLE_SSL_MODES: { value: string; label: string }[] = [
  { value: "require", label: "Require (native network encryption, server not authenticated)" },
  { value: "disable", label: "Disable (not encrypted)" },
]

/** The dial as the driver wants it: only its own setting, PostgreSQL's two names
 * kept as typed (empty until typed), MySQL's server id only when a number was given, Oracle's
 * pluggable database only when one was typed. */
export function shapeCdcDial(dial: CdcDial): CdcDial {
  const { slotName, publicationName, serverId, pdbName, sslMode, ...rest } = dial
  // The TLS mode is always sent, `require` unless a person chose another (`SRC-4 S3 review SHOULD-FIX 1`).
  const common = { ...rest, sslMode: sslMode || DEFAULT_CDC_SSL_MODE }
  if (dial.driver === "oracle") {
    return pdbName ? { ...common, pdbName } : common
  }
  if (dial.driver === "postgres") {
    return { ...common, slotName: slotName ?? "", publicationName: publicationName ?? "" }
  }
  if (dial.driver === "mysql") {
    return typeof serverId === "number" ? { ...common, serverId } : common
  }
  return common
}

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
    sslMode: DEFAULT_CDC_SSL_MODE,
    ...(initialDriver === "postgres" ? { slotName: "", publicationName: "" } : {}),
  }
  const askDriver = !driver || dial.driver !== driver
  const isOracle = dial.driver === "oracle"
  const sslModes = isOracle ? ORACLE_SSL_MODES : CDC_SSL_MODES
  function set<K extends keyof CdcDial>(key: K, v: CdcDial[K]) {
    onChange(shapeCdcDial({ ...dial, [key]: v }))
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
            <option value="oracle">Oracle</option>
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
        <Label htmlFor="cdc-dial-database">{isOracle ? "Container database (service name)" : "Database"}</Label>
        <Input
          id="cdc-dial-database"
          value={dial.database}
          onChange={(e) => set("database", e.target.value)}
          placeholder={isOracle ? "FREE" : "sales"}
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
      <div className="space-y-1.5 sm:col-span-2">
        <Label htmlFor="cdc-dial-ssl-mode">TLS mode</Label>
        <select
          id="cdc-dial-ssl-mode"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.sslMode || DEFAULT_CDC_SSL_MODE}
          onChange={(e) => set("sslMode", e.target.value)}
        >
          {sslModes.map((m) => (
            <option key={m.value} value={m.value}>
              {m.label}
            </option>
          ))}
          {dial.sslMode && !sslModes.some((m) => m.value === dial.sslMode) ? (
            <option value={dial.sslMode}>{dial.sslMode}</option>
          ) : null}
        </select>
        <p className="text-xs text-muted-foreground">
          {isOracle
            ? "Oracle's driver cannot check the server's certificate, so there are only two modes: the connection is encrypted with Oracle's own network encryption, or it is not."
            : "Verify modes use the Kafka Connect worker's own trust store."}
          {dial.driver === "mssql" ? " For SQL Server, Verify CA also checks the host name." : ""}
        </p>
      </div>
      {isOracle ? (
        <div className="space-y-1.5 sm:col-span-2">
          <Label htmlFor="cdc-dial-pdb-name">PDB name</Label>
          <Input
            id="cdc-dial-pdb-name"
            value={dial.pdbName ?? ""}
            onChange={(e) => set("pdbName", e.target.value)}
            placeholder="FREEPDB1"
            autoComplete="off"
          />
          <p className="text-xs text-muted-foreground">
            The pluggable database to read. Leave it empty for a database that is not a container database.
          </p>
        </div>
      ) : null}
      {dial.driver === "postgres" ? (
        <>
          <div className="space-y-1.5">
            <Label htmlFor="cdc-dial-slot-name">Replication slot name</Label>
            <Input
              id="cdc-dial-slot-name"
              value={dial.slotName ?? ""}
              onChange={(e) => set("slotName", e.target.value)}
              placeholder="lakehouse_slot"
              autoComplete="off"
            />
          </div>
          <div className="space-y-1.5">
            <Label htmlFor="cdc-dial-publication-name">Publication name</Label>
            <Input
              id="cdc-dial-publication-name"
              value={dial.publicationName ?? ""}
              onChange={(e) => set("publicationName", e.target.value)}
              placeholder="lakehouse_pub"
              autoComplete="off"
            />
          </div>
        </>
      ) : null}
      {dial.driver === "mysql" ? (
        <div className="space-y-1.5 sm:col-span-2">
          <Label htmlFor="cdc-dial-server-id">Server id</Label>
          <Input
            id="cdc-dial-server-id"
            type="number"
            min={1}
            max={4294967295}
            value={dial.serverId ?? ""}
            onChange={(e) => set("serverId", e.target.value === "" ? undefined : Number(e.target.value))}
            autoComplete="off"
          />
          <p className="text-xs text-muted-foreground">A number no other MySQL replica uses (1 to 4294967295).</p>
        </div>
      ) : null}
    </div>
  )
}
