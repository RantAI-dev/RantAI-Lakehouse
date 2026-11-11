"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import type { SqlDial } from "@/services/contracts/connectors"
import { ORACLE_SSL_MODES, SSL_MODE_DEFAULT, sslModeChoices, sslModeFromChoice } from "./ssl-modes"

/**
 * `dial` for the `Oracle` connector type — the same `sql` adapter/
 * `SqlDial` shape `SqlDialForm` renders, with `driver` fixed to
 * `"oracle"` (Oracle is `SqlDriver::Oracle`, not a fourth `adapter`
 * value — `ingest_spec.rs`). Kept as its own form, not a `SqlDialForm`
 * prop, for two Oracle-only facts `SqlDialForm` has no field for:
 *
 * - This form is the batch type's. Change capture from Oracle is the `Oracle CDC`
 *   type, with its own dial (`CdcDialForm`: container database, PDB, native
 *   network encryption; `SRC-4` slice 6).
 * - `sslServerCertDn` is OPERATOR-TYPED, never derived from `host` — a
 *   bare `CN=<hostname>` would not match a real certificate's full DN
 *   (`SqlDial::ssl_server_cert_dn`'s doc comment).
 */
export function OracleDialForm({
  value,
  onChange,
}: {
  value: SqlDial | null
  onChange: (next: SqlDial) => void
}) {
  const dial: SqlDial = value ?? {
    driver: "oracle",
    host: "",
    port: 1521,
    database: "",
    user: "",
    sslMode: null,
    sslServerCertDn: null,
  }
  function set<K extends keyof SqlDial>(key: K, v: SqlDial[K]) {
    onChange({ ...dial, [key]: v, driver: "oracle" })
  }
  const choices = sslModeChoices(ORACLE_SSL_MODES, dial.sslMode ?? null)
  const currentMode = choices.find((mode) => mode.value === (dial.sslMode ?? SSL_MODE_DEFAULT))
  return (
    <div className="grid gap-3">
      <p className="text-xs text-muted-foreground">
        Batch only — this build cannot run Oracle change-data-capture (LogMiner is refused
        regardless of configuration); a scheduled or manual batch pull is the only supported mode.
      </p>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="oracle-dial-host">Host</Label>
          <Input
            id="oracle-dial-host"
            value={dial.host}
            onChange={(e) => set("host", e.target.value)}
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="oracle-dial-port">Port</Label>
          <Input
            id="oracle-dial-port"
            type="number"
            value={dial.port}
            onChange={(e) => set("port", Number(e.target.value))}
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="oracle-dial-database">Database (service name)</Label>
          <Input
            id="oracle-dial-database"
            value={dial.database}
            onChange={(e) => set("database", e.target.value)}
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          {/* A literal username, never a secretRef picker -- same
              reasoning as SqlDialForm's User field. */}
          <Label htmlFor="oracle-dial-user">User</Label>
          <Input
            id="oracle-dial-user"
            value={dial.user}
            onChange={(e) => set("user", e.target.value)}
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="oracle-dial-ssl-mode">SSL mode (optional)</Label>
          {/* WEB-8 D8: the two modes the Oracle adapter accepts, plus
              "Default" (no mode sent) — a stored `null` is a real state
              (plaintext, no DN check), so it stays selectable rather than
              being read as "disable". */}
          <Select
            value={dial.sslMode ?? SSL_MODE_DEFAULT}
            onValueChange={(next) => set("sslMode", sslModeFromChoice(next ?? SSL_MODE_DEFAULT))}
          >
            <SelectTrigger id="oracle-dial-ssl-mode" aria-label="SSL mode (optional)" className="w-full">
              <SelectValue>{currentMode?.label ?? "Default (no mode sent)"}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {choices.map((mode) => (
                <SelectItem key={mode.value} value={mode.value}>
                  {mode.label}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="oracle-dial-ssl-server-cert-dn">TLS server certificate DN (required for TLS)</Label>
          <Input
            id="oracle-dial-ssl-server-cert-dn"
            value={dial.sslServerCertDn ?? ""}
            onChange={(e) => set("sslServerCertDn", e.target.value === "" ? null : e.target.value)}
            placeholder="CN=oracle.example.com,O=Example,C=US"
            autoComplete="off"
          />
          <p className="text-xs text-muted-foreground">
            Type the certificate&apos;s real Distinguished Name — this is never derived from Host
            above.
          </p>
        </div>
      </div>
    </div>
  )
}
