import { describe, expect, it } from "bun:test"
import { targetChanged, targetIdentity } from "@/lib/connectors/target-identity"

// The Rust function is the authority (`Dial::target_identity`); these cases
// are the ones its own unit tests pin, so the two stay in step.

type Dial = Record<string, unknown>

/** True when changing `field` of `base` keeps the target. */
const same = (adapter: string, base: Dial, field: string, value: unknown) =>
  targetIdentity(adapter, base) === targetIdentity(adapter, { ...base, [field]: value })

const sql: Dial = { driver: "postgres", host: "db.internal", port: 5432, database: "orders", user: "reader" }
const files: Dial = { protocol: "s3", endpoint: "https://store.internal:9000", bucket: "landing", format: "csv" }
const rest: Dial = {
  baseUrl: "https://api.example.com/v1",
  auth: { type: "bearer" },
  pagination: { type: "none" },
  endpoints: [],
}
const mongo: Dial = {
  hosts: ["m1.internal:27017", "m2.internal:27017"],
  database: "shop",
  username: "reader",
  directConnection: true,
}
const kafka: Dial = {
  bootstrapServers: ["k1.internal:9092", "k2.internal:9092"],
  topic: "orders",
  auth: { type: "none" },
  groupId: "g",
  microBatchSeconds: 30,
}
const sftp: Dial = {
  host: "sftp.internal",
  port: 22,
  user: "reader",
  hostKeyFingerprint: "SHA256:abc",
  path: "/inbox",
  fileFormat: "csv",
  auth: { type: "password" },
}

describe("targetIdentity", () => {
  it("re-points a sql or cdc connector by driver, host, port or database only", () => {
    for (const adapter of ["sql", "cdc"]) {
      expect(same(adapter, sql, "driver", "mysql")).toBe(false)
      expect(same(adapter, sql, "host", "other.internal")).toBe(false)
      expect(same(adapter, sql, "port", 5433)).toBe(false)
      expect(same(adapter, sql, "database", "billing")).toBe(false)
      expect(same(adapter, sql, "user", "someone")).toBe(true)
      expect(same(adapter, sql, "sslMode", "require")).toBe(true)
      expect(same(adapter, sql, "slotName", "s")).toBe(true)
    }
  })

  it("compares host names without regard to case, and the adapter itself", () => {
    expect(same("sql", sql, "host", "DB.Internal")).toBe(true)
    expect(targetIdentity("sql", sql)).not.toBe(targetIdentity("cdc", sql))
  })

  it("re-points a files connector by protocol, endpoint or bucket", () => {
    expect(same("files", files, "protocol", "sftp")).toBe(false)
    expect(same("files", files, "endpoint", "https://other.internal:9000")).toBe(false)
    expect(same("files", files, "endpoint", "http://store.internal:9000")).toBe(false)
    expect(same("files", files, "endpoint", "https://store.internal:9001")).toBe(false)
    expect(same("files", files, "bucket", "other")).toBe(false)
    expect(same("files", files, "prefix", "2026/")).toBe(true)
    expect(same("files", files, "format", "parquet")).toBe(true)
    expect(same("files", files, "region", "eu-west-1")).toBe(true)
  })

  it("treats a URL's default port as the same target, and no endpoint as its own", () => {
    const implicit = { ...files, endpoint: "https://store.internal" }
    expect(same("files", implicit, "endpoint", "https://store.internal:443")).toBe(true)
    expect(same("files", implicit, "endpoint", "HTTPS://Store.Internal/some/path")).toBe(true)
    expect(same("files", implicit, "endpoint", "https://store.internal:80")).toBe(false)
    const none = { protocol: "s3", bucket: "landing", format: "csv" }
    expect(targetIdentity("files", none)).not.toBe(targetIdentity("files", implicit))
    expect(targetIdentity("files", none)).toBe(targetIdentity("files", { ...none, endpoint: "  " }))
  })

  it("re-points a rest connector by the scheme, host or port of its base URL", () => {
    expect(same("rest", rest, "baseUrl", "http://api.example.com/v1")).toBe(false)
    expect(same("rest", rest, "baseUrl", "https://api.other.com/v1")).toBe(false)
    expect(same("rest", rest, "baseUrl", "https://api.example.com:8443/v1")).toBe(false)
    expect(same("rest", rest, "baseUrl", "https://api.example.com:443/v1")).toBe(true)
    expect(same("rest", rest, "baseUrl", "https://API.example.com/v2/orders?x=1")).toBe(true)
    expect(same("rest", rest, "auth", { type: "basic" })).toBe(true)
    expect(same("rest", rest, "pagination", { type: "page", param: "p" })).toBe(true)
  })

  it("does not let userinfo hide the host of a base URL", () => {
    expect(same("rest", rest, "baseUrl", "https://api.example.com@evil.example/")).toBe(false)
    expect(same("rest", rest, "baseUrl", "https://someone@api.example.com/")).toBe(true)
  })

  it("re-points a mongodb connector by its host set or database", () => {
    expect(same("mongodb", mongo, "hosts", ["m1.internal:27017"])).toBe(false)
    expect(same("mongodb", mongo, "hosts", ["m1.internal:27018", "m2.internal:27017"])).toBe(false)
    expect(same("mongodb", mongo, "hosts", ["M2.internal:27017", "m1.internal:27017"])).toBe(true)
    expect(same("mongodb", mongo, "database", "billing")).toBe(false)
    expect(same("mongodb", mongo, "username", "someone")).toBe(true)
  })

  it("re-points a kafka connector by its bootstrap servers only", () => {
    expect(same("kafka", kafka, "bootstrapServers", ["k1.internal:9092"])).toBe(false)
    expect(same("kafka", kafka, "bootstrapServers", ["K2.internal:9092", "k1.internal:9092"])).toBe(true)
    expect(same("kafka", kafka, "topic", "other")).toBe(true)
    expect(same("kafka", kafka, "groupId", "other")).toBe(true)
    expect(same("kafka", kafka, "auth", { type: "sasl_plain", username: "u" })).toBe(true)
  })

  it("re-points an sftp connector by host, port or host key", () => {
    expect(same("sftp", sftp, "host", "other.internal")).toBe(false)
    expect(same("sftp", sftp, "port", 2222)).toBe(false)
    expect(same("sftp", sftp, "hostKeyFingerprint", "SHA256:xyz")).toBe(false)
    expect(same("sftp", sftp, "host", "SFTP.internal")).toBe(true)
    expect(same("sftp", sftp, "user", "someone")).toBe(true)
    expect(same("sftp", sftp, "path", "/outbox")).toBe(true)
    expect(same("sftp", sftp, "auth", { type: "public_key" })).toBe(true)
  })

  it("has no target to change for sheets", () => {
    const sheets = { spreadsheetId: "abc", ranges: ["A1:B2"] }
    expect(same("sheets", sheets, "spreadsheetId", "def")).toBe(true)
    expect(same("sheets", sheets, "ranges", ["C1:D2"])).toBe(true)
  })

  it("gives no identity for an unknown adapter or a compared field of the wrong type", () => {
    expect(targetIdentity("smtp", {})).toBeNull()
    expect(targetIdentity(null, sql)).toBeNull()
    expect(targetIdentity("sql", { ...sql, port: "5432" })).toBeNull()
    expect(targetIdentity("sql", { ...sql, host: undefined })).toBeNull()
  })
})

describe("targetChanged", () => {
  const stored = { storedAdapter: "sql", storedDial: sql }

  it("is false for a connector that has never been configured", () => {
    expect(targetChanged({ storedAdapter: null, storedDial: {}, adapter: "sql", dial: sql })).toBe(false)
    expect(targetChanged({ storedAdapter: null, storedDial: null, adapter: "sql", dial: sql })).toBe(false)
  })

  it("is true when there is no adapter but a dial it cannot be compared with", () => {
    expect(targetChanged({ storedAdapter: null, storedDial: sql, adapter: "sql", dial: sql })).toBe(true)
  })

  it("is false for a save that keeps the target and true for one that moves it", () => {
    expect(targetChanged({ ...stored, adapter: "sql", dial: { ...sql, user: "someone" } })).toBe(false)
    expect(targetChanged({ ...stored, adapter: "sql", dial: { ...sql, host: "10.0.0.9" } })).toBe(true)
    expect(targetChanged({ ...stored, adapter: "sql", dial: { ...sql, port: 5433 } })).toBe(true)
  })

  it("is true when either side cannot be compared", () => {
    expect(targetChanged({ storedAdapter: "sql", storedDial: {}, adapter: "sql", dial: sql })).toBe(true)
    expect(targetChanged({ ...stored, adapter: "sql", dial: { ...sql, host: undefined } })).toBe(true)
  })
})
