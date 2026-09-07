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
  ok: boolean
  latencyMs: number
  message: string
  testedAt: string
}

export type CreateConnectorInput = {
  name: string
  type: string
  direction: Connector["direction"]
  host: string
  secretRef: string
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
