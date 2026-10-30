"use client"

import * as React from "react"
import { saasVendorById } from "@/lib/connectors/saas-vendors"

type Item = { label: string; value: React.ReactNode }

const REST_AUTH_LABEL: Record<string, string> = {
  api_key: "API key",
  bearer: "Bearer token",
  basic: "Basic (username + password)",
  oauth2_client_credentials: "OAuth2 client credentials",
}

function text(value: unknown): string {
  if (typeof value === "string") return value.trim()
  if (typeof value === "number" && Number.isFinite(value)) return String(value)
  return ""
}

function list(value: unknown): string {
  return Array.isArray(value) ? value.map(text).filter(Boolean).join(", ") : ""
}

function hostPort(dial: Record<string, unknown>): string {
  const host = text(dial.host)
  const port = text(dial.port)
  if (!host) return ""
  return port ? `${host}:${port}` : host
}

function authType(dial: Record<string, unknown>): string {
  return text((dial.auth as { type?: unknown } | undefined)?.type)
}

/**
 * The connection settings worth checking before creating, per adapter:
 * where it dials and what it reads. An unset field shows as "—" (the
 * summary's own empty marker), so a skipped field is visible here rather
 * than as a failed save after creation.
 */
export function connectionReviewItems(adapter: string | null, dial: Record<string, unknown> | null): Item[] {
  const d = dial ?? {}
  switch (adapter) {
    case "sql":
      return [
        { label: "Server", value: hostPort(d) },
        { label: "Database", value: text(d.database) },
        { label: "User", value: text(d.user) },
        { label: "SSL mode", value: text(d.sslMode) || "Driver default" },
      ]
    case "cdc":
      if (Array.isArray(d.hosts)) {
        // MongoDB change capture (`SRC-4` slice 5): the replica set's seeds, not one host and port.
        return [
          { label: "Hosts", value: list(d.hosts) },
          { label: "Replica set", value: text(d.replicaSet) },
          { label: "Database", value: text(d.database) },
          { label: "User", value: text(d.user) },
        ]
      }
      return [
        { label: "Server", value: hostPort(d) },
        { label: "Database", value: text(d.database) },
        { label: "User", value: text(d.user) },
        { label: "TLS mode", value: text(d.sslMode) || "require" },
        ...(text(d.slotName) || text(d.publicationName)
          ? [{ label: "Slot / publication", value: [text(d.slotName), text(d.publicationName)].filter(Boolean).join(" / ") }]
          : []),
        ...(text(d.serverId) ? [{ label: "Server id", value: text(d.serverId) }] : []),
      ]
    case "files":
      return [
        { label: "Bucket", value: text(d.bucket) },
        { label: "Prefix", value: text(d.prefix) || "Whole bucket" },
        { label: "Endpoint", value: text(d.endpoint) || "Public AWS S3" },
        { label: "Format", value: text(d.format).toUpperCase() },
      ]
    case "sftp":
      return [
        { label: "Server", value: hostPort(d) },
        { label: "User", value: text(d.user) },
        { label: "Remote path", value: text(d.path) },
        { label: "Host key", value: text(d.hostKeyFingerprint) },
      ]
    case "rest": {
      const endpoints = Array.isArray(d.endpoints)
        ? (d.endpoints as { path?: unknown }[]).map((e) => text(e.path)).filter(Boolean)
        : []
      return [
        { label: "Base URL", value: text(d.baseUrl) },
        { label: "Auth", value: REST_AUTH_LABEL[authType(d)] ?? "" },
        { label: "Endpoints", value: endpoints.join(", ") },
        { label: "Pagination", value: text((d.pagination as { type?: unknown } | undefined)?.type) || "none" },
      ]
    }
    case "saas":
      return [
        { label: "Business app", value: saasVendorById(d.vendor)?.label ?? text(d.vendor) },
        { label: "Account", value: text(d.account) },
        { label: "Sign-in", value: authType(d) === "integration" ? "Integration credential" : "Browser sign-in" },
      ]
    case "sheets":
      return [
        { label: "Spreadsheet", value: text(d.spreadsheetId) },
        { label: "Ranges", value: list(d.ranges) },
      ]
    case "mongodb":
      return [
        { label: "Hosts", value: list(d.hosts) },
        { label: "Database", value: text(d.database) },
        { label: "User", value: text(d.username) },
      ]
    case "kafka":
      return [
        { label: "Brokers", value: list(d.bootstrapServers) },
        { label: "Topic", value: text(d.topic) },
        { label: "Consumer group", value: text(d.groupId) },
        { label: "Auth", value: authType(d) === "sasl_plain" ? "SASL/PLAIN" : authType(d) === "none" ? "None" : "" },
      ]
    default:
      return []
  }
}
