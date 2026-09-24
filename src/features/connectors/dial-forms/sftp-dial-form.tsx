"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { SftpAuth, SftpDial } from "@/services/contracts/connectors"

const AUTH_TYPES: SftpAuth["type"][] = ["password", "public_key"]

/**
 * `dial` for the `sftp` adapter. `hostKeyFingerprint` is REQUIRED with no
 * fallback (`SftpDial::host_key_fingerprint`, `ingest_spec.rs`) — this
 * build never auto-adds an unknown host key (`paramiko.AutoAddPolicy` is
 * refused outright), so this form marks the input `required` and shows
 * how to obtain the pinned value before it can be entered honestly.
 */
export function SftpDialForm({
  value,
  onChange,
}: {
  value: SftpDial | null
  onChange: (next: SftpDial) => void
}) {
  const dial: SftpDial = value ?? {
    host: "",
    port: 22,
    user: "",
    hostKeyFingerprint: "",
    path: "",
    fileFormat: "csv",
    auth: { type: "password" },
  }
  function set<K extends keyof SftpDial>(key: K, v: SftpDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  return (
    <div className="grid gap-3 sm:grid-cols-2">
      <div className="space-y-1.5">
        <Label htmlFor="sftp-dial-host">Host</Label>
        <Input id="sftp-dial-host" value={dial.host} onChange={(e) => set("host", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sftp-dial-port">Port</Label>
        <Input
          id="sftp-dial-port"
          type="number"
          value={dial.port}
          onChange={(e) => set("port", Number(e.target.value))}
        />
      </div>
      <div className="space-y-1.5">
        {/* A literal username, never a secretRef picker -- same
            reasoning as SqlDialForm's User field. */}
        <Label htmlFor="sftp-dial-user">User</Label>
        <Input id="sftp-dial-user" value={dial.user} onChange={(e) => set("user", e.target.value)} />
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sftp-dial-path">Remote path</Label>
        <Input id="sftp-dial-path" value={dial.path} onChange={(e) => set("path", e.target.value)} />
      </div>
      <div className="space-y-1.5 sm:col-span-2">
        <Label htmlFor="sftp-dial-host-key-fingerprint">Host key fingerprint</Label>
        <Input
          id="sftp-dial-host-key-fingerprint"
          value={dial.hostKeyFingerprint}
          onChange={(e) => set("hostKeyFingerprint", e.target.value)}
          placeholder="SHA256:base64hash"
          required
        />
        <p className="text-xs text-muted-foreground">
          Required — this build never auto-adds an unknown host key. Obtain it with{" "}
          <code className="rounded bg-muted px-1 py-0.5">
            ssh-keyscan &lt;host&gt; | ssh-keygen -lf -
          </code>{" "}
          and copy the <code className="rounded bg-muted px-1 py-0.5">SHA256:…</code> value.
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="sftp-dial-auth-type">Auth type</Label>
        <select
          id="sftp-dial-auth-type"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.auth.type}
          onChange={(e) => set("auth", { type: e.target.value as SftpAuth["type"] })}
        >
          {AUTH_TYPES.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
        {dial.auth.type === "public_key" ? (
          <p className="text-xs text-muted-foreground">
            The private key PEM is the connector&apos;s secret — set the credential kind to
            &quot;Private key&quot; in the next step.
          </p>
        ) : null}
      </div>
    </div>
  )
}
