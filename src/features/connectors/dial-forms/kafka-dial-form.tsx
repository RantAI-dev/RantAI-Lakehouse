"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { KafkaAuth, KafkaDial } from "@/services/contracts/connectors"

const AUTH_TYPES: KafkaAuth["type"][] = ["sasl_plain", "none"]

function defaultAuth(type: KafkaAuth["type"]): KafkaAuth {
  switch (type) {
    case "sasl_plain":
      return { type: "sasl_plain", username: "" }
    case "none":
      return { type: "none" }
  }
}

/**
 * `dial` for the `kafka` adapter. Auth options are exactly the two
 * `KafkaAuth` variants (`ingest_spec.rs`) — no third option this form
 * invents. `sasl_plain`'s `username` is dial configuration, not a
 * secret (only the password is, carried as the connector's own
 * `secretRef`) — `KafkaAuth::SaslPlain`'s doc comment — so this form
 * renders a username input for it; `none` has no credential at all
 * (`secret_field_names` maps `("kafka", "none")` to no fields), stated
 * here rather than silently offering a secret picker that would never
 * be read.
 */
export function KafkaDialForm({
  value,
  onChange,
}: {
  value: KafkaDial | null
  onChange: (next: KafkaDial) => void
}) {
  const dial: KafkaDial = value ?? {
    bootstrapServers: [""],
    topic: "",
    auth: { type: "sasl_plain", username: "" },
    groupId: "",
    microBatchSeconds: 30,
  }
  function set<K extends keyof KafkaDial>(key: K, v: KafkaDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  function setBroker(index: number, broker: string) {
    set(
      "bootstrapServers",
      dial.bootstrapServers.map((b, i) => (i === index ? broker : b))
    )
  }
  function addBroker() {
    set("bootstrapServers", [...dial.bootstrapServers, ""])
  }
  function removeBroker(index: number) {
    set(
      "bootstrapServers",
      dial.bootstrapServers.filter((_, i) => i !== index)
    )
  }

  return (
    <div className="grid gap-3">
      <div className="space-y-2">
        <p className="text-sm font-medium">Bootstrap servers (host:port)</p>
        {dial.bootstrapServers.map((broker, index) => (
          <div key={index} className="flex items-center gap-2">
            <Input
              id={`kafka-dial-broker-${index}`}
              aria-label={`Bootstrap server ${index + 1}`}
              value={broker}
              onChange={(e) => setBroker(index, e.target.value)}
              placeholder="kafka-0.internal:9092"
            />
            {dial.bootstrapServers.length > 1 ? (
              <button
                type="button"
                className="text-xs text-muted-foreground underline"
                onClick={() => removeBroker(index)}
              >
                Remove
              </button>
            ) : null}
          </div>
        ))}
        <button type="button" className="text-xs text-primary underline" onClick={addBroker}>
          Add broker
        </button>
      </div>
      <div className="grid gap-3 sm:grid-cols-2">
        <div className="space-y-1.5">
          <Label htmlFor="kafka-dial-topic">Topic</Label>
          <Input id="kafka-dial-topic" value={dial.topic} onChange={(e) => set("topic", e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="kafka-dial-group-id">Consumer group id</Label>
          <Input id="kafka-dial-group-id" value={dial.groupId} onChange={(e) => set("groupId", e.target.value)} />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="kafka-dial-micro-batch-seconds">Micro-batch cap (seconds)</Label>
          <Input
            id="kafka-dial-micro-batch-seconds"
            type="number"
            value={dial.microBatchSeconds}
            onChange={(e) => set("microBatchSeconds", Number(e.target.value))}
          />
        </div>
        <div className="space-y-1.5">
          <Label htmlFor="kafka-dial-auth-type">Auth type</Label>
          <select
            id="kafka-dial-auth-type"
            className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
            value={dial.auth.type}
            onChange={(e) => set("auth", defaultAuth(e.target.value as KafkaAuth["type"]))}
          >
            {AUTH_TYPES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </div>
        {dial.auth.type === "sasl_plain" ? (
          <div className="space-y-1.5 sm:col-span-2">
            {/* SASL/PLAIN username is dial configuration, not a secret --
                only the password is (the connector's own secretRef). */}
            <Label htmlFor="kafka-dial-auth-username">SASL username</Label>
            <Input
              id="kafka-dial-auth-username"
              value={dial.auth.username}
              onChange={(e) => set("auth", { type: "sasl_plain", username: e.target.value })}
            />
          </div>
        ) : (
          <p className="text-xs text-muted-foreground sm:col-span-2">
            No credential for a `none`-auth (PLAINTEXT) broker — leave the connector&apos;s
            `secretRef` unset.
          </p>
        )}
      </div>
    </div>
  )
}
