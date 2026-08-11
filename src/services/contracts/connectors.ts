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

export type ConnectorDetail = Connector & {
  discoveredAssets: number
  recentErrors: { at: string; message: string }[]
  dependentPipelines: string[]
}

export interface ConnectorService {
  listConnectors(signal?: AbortSignal): Promise<Connector[]>
  getConnector(id: string, signal?: AbortSignal): Promise<ConnectorDetail>
}
