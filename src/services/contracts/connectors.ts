import type { Health } from "@/lib/status"

export type Connector = {
  id: string
  name: string
  type: string
  direction: "source" | "sink" | "bidirectional"
  health: Health
  environment: string
  tenant: string
  lastTestAt: string
  lastActivityAt: string
  capabilities: string[]
  owner: string
}

export type ConnectorDependent = {
  id: string
  name: string
  kind: "pipeline"
}

export type DiscoveredSchema = {
  name: string
  kind: "table" | "topic" | "prefix"
  columnsOrFields: number
}

export type ConnectorDetail = Connector & {
  discoveredAssets: number
  discoveredSchemas: DiscoveredSchema[]
  recentErrors: { at: string; message: string }[]
  dependentPipelines: ConnectorDependent[]
  auditEventId?: string
}

export type ConnectorTestResult = {
  /** Whether a real connectivity probe succeeded. Always `false` when `supported` is `false`. */
  ok: boolean
  /**
   * Whether this build knows how to dial this connector's type at all.
   * `false` for every type except PostgreSQL and S3-compatible object
   * storage today — see `rust/crates/lakehouse-api/src/connector_probe.rs`.
   */
  supported: boolean
  /** Real measured latency in milliseconds; `null` when `supported` is `false` (no attempt was made). */
  latencyMs: number | null
  message: string
  testedAt: string
}

export type CreateConnectorInput = {
  name: string
  type: string
  direction: Connector["direction"]
  host: string
  secretRef: string
  /**
   * Optional secondary reference, e.g. the secret-access-key half of an S3
   * connector's access-key/secret-key pair. Without this, an API-created S3
   * connector can never be tested — `connector_probe::probe_s3` requires
   * both `secretRef` and `secretRefSecondary` to be set.
   */
  secretRefSecondary?: string
  environment: string
  tenant: string
  residency: string
  capabilities: string[]
  owner?: string
}

export interface ConnectorService {
  listConnectors(signal?: AbortSignal): Promise<Connector[]>
  getConnector(id: string, signal?: AbortSignal): Promise<ConnectorDetail>
  createConnector(input: CreateConnectorInput, signal?: AbortSignal): Promise<Connector>
  testConnection(id: string, signal?: AbortSignal): Promise<ConnectorTestResult>
}
