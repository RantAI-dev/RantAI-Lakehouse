"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { CdcMongoDial } from "@/services/contracts/connectors"

/**
 * `dial` for a MongoDB change-capture connector (`SRC-4` slice 5, decision S5-i): the `cdc` adapter
 * with `driver: "mongodb"`. It is its own form beside `CdcDialForm` because the dial has another
 * shape: a list of `host:port` seeds instead of one host and port, and a required replica set name
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
  function setHost(index: number, host: string) {
    set(
      "hosts",
      dial.hosts.map((h, i) => (i === index ? host : h))
    )
  }
  return (
    <div className="grid gap-3">
      <div className="space-y-2">
        <p className="text-sm font-medium">Replica set hosts (host:port)</p>
        {dial.hosts.map((host, index) => (
          <div key={index} className="flex items-center gap-2">
            <Input
              id={`cdc-mongo-dial-host-${index}`}
              aria-label={`Host ${index + 1}`}
              value={host}
              onChange={(e) => setHost(index, e.target.value)}
              placeholder="mongo-0.internal.example.com:27017"
              autoComplete="off"
            />
            {dial.hosts.length > 1 ? (
              <button
                type="button"
                className="text-xs text-muted-foreground underline"
                onClick={() =>
                  set(
                    "hosts",
                    dial.hosts.filter((_, i) => i !== index)
                  )
                }
              >
                Remove
              </button>
            ) : null}
          </div>
        ))}
        <button type="button" className="text-xs text-primary underline" onClick={() => set("hosts", [...dial.hosts, ""])}>
          Add host
        </button>
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
