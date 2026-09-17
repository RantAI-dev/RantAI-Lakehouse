"use client"

import * as React from "react"
import { Input } from "@/components/ui/input"
import { Label } from "@/components/ui/label"
import type { RestAuth, RestDial, RestEndpoint, RestPagination } from "@/services/contracts/connectors"

const AUTH_TYPES: RestAuth["type"][] = ["api_key", "bearer", "oauth2_client_credentials", "basic"]

function defaultAuth(type: RestAuth["type"]): RestAuth {
  switch (type) {
    case "api_key":
      return { type: "api_key", header: "" }
    case "oauth2_client_credentials":
      return { type: "oauth2_client_credentials", tokenUrl: "" }
    case "bearer":
      return { type: "bearer" }
    case "basic":
      return { type: "basic" }
  }
}

function defaultPagination(type: RestPagination["type"]): RestPagination {
  switch (type) {
    case "page":
      return { type: "page", param: "" }
    case "cursor":
      return { type: "cursor", cursorField: "" }
    case "none":
      return { type: "none" }
  }
}

/**
 * `dial` for the `rest` adapter. The bearer/basic auth types carry their
 * actual secret as the connector's own `secretRef` (set elsewhere in the
 * wizard) — this form deliberately renders no field for either, matching
 * `RestAuth::Bearer`/`RestAuth::Basic` having no payload at all in
 * `ingest_spec.rs`.
 */
export function RestDialForm({
  value,
  onChange,
}: {
  value: RestDial | null
  onChange: (next: RestDial) => void
}) {
  const dial: RestDial = value ?? {
    baseUrl: "",
    auth: { type: "api_key", header: "" },
    pagination: { type: "none" },
    endpoints: [{ path: "", recordsPath: null }],
  }
  function set<K extends keyof RestDial>(key: K, v: RestDial[K]) {
    onChange({ ...dial, [key]: v })
  }
  function setEndpoint(index: number, next: RestEndpoint) {
    set(
      "endpoints",
      dial.endpoints.map((e, i) => (i === index ? next : e))
    )
  }
  function addEndpoint() {
    set("endpoints", [...dial.endpoints, { path: "", recordsPath: null }])
  }
  function removeEndpoint(index: number) {
    set(
      "endpoints",
      dial.endpoints.filter((_, i) => i !== index)
    )
  }

  return (
    <div className="grid gap-3">
      <div className="space-y-1.5">
        <Label htmlFor="rest-dial-base-url">Base URL</Label>
        <Input
          id="rest-dial-base-url"
          value={dial.baseUrl}
          onChange={(e) => set("baseUrl", e.target.value)}
          placeholder="https://api.example.com"
        />
      </div>

      <div className="space-y-1.5">
        <Label htmlFor="rest-dial-auth-type">Auth type</Label>
        <select
          id="rest-dial-auth-type"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.auth.type}
          onChange={(e) => set("auth", defaultAuth(e.target.value as RestAuth["type"]))}
        >
          {AUTH_TYPES.map((t) => (
            <option key={t} value={t}>
              {t}
            </option>
          ))}
        </select>
      </div>
      {dial.auth.type === "api_key" ? (
        <div className="space-y-1.5">
          <Label htmlFor="rest-dial-auth-header">Header name</Label>
          <Input
            id="rest-dial-auth-header"
            value={dial.auth.header}
            onChange={(e) => set("auth", { type: "api_key", header: e.target.value })}
            placeholder="X-API-Key"
          />
        </div>
      ) : null}
      {dial.auth.type === "oauth2_client_credentials" ? (
        <div className="space-y-1.5">
          <Label htmlFor="rest-dial-auth-token-url">Token URL</Label>
          <Input
            id="rest-dial-auth-token-url"
            value={dial.auth.tokenUrl}
            onChange={(e) => set("auth", { type: "oauth2_client_credentials", tokenUrl: e.target.value })}
          />
        </div>
      ) : null}
      {/* bearer/basic carry no extra field here — their secret is the
          connector's own secretRef, entered elsewhere in the wizard. */}

      <div className="space-y-1.5">
        <Label htmlFor="rest-dial-pagination-type">Pagination</Label>
        <select
          id="rest-dial-pagination-type"
          className="h-8 w-full rounded-lg border border-input bg-transparent px-2.5 text-sm"
          value={dial.pagination.type}
          onChange={(e) => set("pagination", defaultPagination(e.target.value as RestPagination["type"]))}
        >
          <option value="none">None</option>
          <option value="page">Page number</option>
          <option value="cursor">Cursor</option>
        </select>
      </div>
      {dial.pagination.type === "page" ? (
        <div className="space-y-1.5">
          <Label htmlFor="rest-dial-pagination-param">Page query parameter</Label>
          <Input
            id="rest-dial-pagination-param"
            value={dial.pagination.param}
            onChange={(e) => set("pagination", { type: "page", param: e.target.value })}
          />
        </div>
      ) : null}
      {dial.pagination.type === "cursor" ? (
        <div className="space-y-1.5">
          <Label htmlFor="rest-dial-pagination-cursor-field">Cursor field (JSON path)</Label>
          <Input
            id="rest-dial-pagination-cursor-field"
            value={dial.pagination.cursorField}
            onChange={(e) => set("pagination", { type: "cursor", cursorField: e.target.value })}
          />
        </div>
      ) : null}

      <div className="space-y-2">
        <p className="text-sm font-medium">Endpoints</p>
        {dial.endpoints.map((endpoint, index) => (
          <div key={index} className="grid gap-2 sm:grid-cols-2">
            <div className="space-y-1.5">
              <Label htmlFor={`rest-dial-endpoint-${index}-path`}>Path</Label>
              <Input
                id={`rest-dial-endpoint-${index}-path`}
                value={endpoint.path}
                onChange={(e) => setEndpoint(index, { ...endpoint, path: e.target.value })}
                placeholder="/v1/records"
              />
            </div>
            <div className="space-y-1.5">
              <Label htmlFor={`rest-dial-endpoint-${index}-records-path`}>Records path (optional)</Label>
              <Input
                id={`rest-dial-endpoint-${index}-records-path`}
                value={endpoint.recordsPath ?? ""}
                onChange={(e) =>
                  setEndpoint(index, { ...endpoint, recordsPath: e.target.value === "" ? null : e.target.value })
                }
                placeholder="data.records"
              />
            </div>
            {dial.endpoints.length > 1 ? (
              <button
                type="button"
                className="text-left text-xs text-muted-foreground underline sm:col-span-2"
                onClick={() => removeEndpoint(index)}
              >
                Remove endpoint
              </button>
            ) : null}
          </div>
        ))}
        <button type="button" className="text-xs text-primary underline" onClick={addEndpoint}>
          Add endpoint
        </button>
      </div>
    </div>
  )
}
