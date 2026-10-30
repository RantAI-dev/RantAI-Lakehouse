"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { CdcMongoDial } from "@/services/contracts/connectors"

/**
 * `dial` for a MongoDB change-capture connector (`SRC-4` slice 5, decision S5-i): the `cdc` adapter
 * with `driver: "mongodb"`. It is its own form beside `CdcDialForm` because the dial has another
 * shape: one `host:port` instead of a host and a port, and a required replica set name
 * instead of a slot, a publication or a server id.
 *
 * Nothing is pre-filled with an invented value: the replica set starts empty (change streams need a
 * replica set, and only the server's own configuration knows its name), and `Dial::parse` already
 * rejects a missing value on every `PUT` with its own fixed sentence, so this form adds no
 * client-side required-field rule that could drift from it. The password and the TLS choice are not
 * in this form: the password is the credential field every connector has, and `sslMode` is left out
 * of the dial, which the server reads as `require`.
 */
export function CdcMongoDialForm({
  value,
  onChange,
}: {
  value: CdcMongoDial | null
  onChange: (next: CdcMongoDial) => void
}) {
  const dial: CdcMongoDial = value ?? {
    driver: "mongodb",
    hosts: [""],
    database: "",
    user: "",
    replicaSet: "",
  }
  function set<K extends keyof CdcMongoDial>(key: K, v: CdcMongoDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  // One host (`SRC-4` slice 5 review BLOCKER 1): the API refuses any other count. The contract keeps
  // `hosts` a list so the shape can grow with a bounded check later; this form edits its first entry.
  return (
    <div className="grid gap-3">
      <div className="space-y-1.5">
        <Label htmlFor="cdc-mongo-dial-host">Host (host:port)</Label>
        <Input
          id="cdc-mongo-dial-host"
          value={dial.hosts[0] ?? ""}
          onChange={(e) => set("hosts", [e.target.value])}
          placeholder="mongo-0.internal.example.com:27017"
          autoComplete="off"
        />
        <p className="text-xs text-muted-foreground">
          One member of the replica set that change streams can be read from.
        </p>
      </div>
      <div className="space-y-1.5">
        <Label htmlFor="cdc-mongo-dial-replica-set">Replica set name</Label>
        <Input
          id="cdc-mongo-dial-replica-set"
          value={dial.replicaSet}
          onChange={(e) => set("replicaSet", e.target.value)}
          autoComplete="off"
        />
        <p className="text-xs text-muted-foreground">The name in the server&apos;s replica set configuration.</p>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="cdc-mongo-dial-database">Database</Label>
          <Input
            id="cdc-mongo-dial-database"
            value={dial.database}
            onChange={(e) => set("database", e.target.value)}
            placeholder="sales"
            autoComplete="off"
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="cdc-mongo-dial-user">User</Label>
          <Input
            id="cdc-mongo-dial-user"
            value={dial.user}
            onChange={(e) => set("user", e.target.value)}
            placeholder="lakehouse_cdc"
            autoComplete="off"
          />
        </div>
      </div>
    </div>
  )
}
