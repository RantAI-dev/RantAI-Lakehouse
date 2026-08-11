import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type { Connector, ConnectorService } from "../contracts/connectors"

const CONNECTORS: Connector[] = [
  {
    id: "conn-pg-core",
    name: "postgres core-banking CDC",
    type: "PostgreSQL CDC",
    direction: "source",
    health: "healthy",
    environment: "production",
    tenant: "Nusantara Finance",
    lastTestAt: agoIso(120),
    lastActivityAt: agoIso(2),
    capabilities: ["CDC", "schema discovery", "checkpoint"],
    owner: "Data Platform",
  },
  {
    id: "conn-s3-docs",
    name: "policy documents bucket",
    type: "Object storage",
    direction: "source",
    health: "healthy",
    environment: "production",
    tenant: "Nusantara Finance",
    lastTestAt: agoIso(400),
    lastActivityAt: agoIso(40),
    capabilities: ["list", "read", "event notify"],
    owner: "Risk Analytics",
  },
  {
    id: "conn-kafka-payments",
    name: "payments events",
    type: "Kafka",
    direction: "bidirectional",
    health: "degraded",
    environment: "production",
    tenant: "Nusantara Finance",
    lastTestAt: agoIso(30),
    lastActivityAt: agoIso(1),
    capabilities: ["consume", "produce", "pushdown filters"],
    owner: "Payments Platform",
  },
]

export const mockConnectorService: ConnectorService = {
  listConnectors(signal) {
    return mockCall(() => CONNECTORS, { signal })
  },
  getConnector(id, signal) {
    return mockCall(() => {
      const c = CONNECTORS.find((x) => x.id === id)
      if (!c) throw new ServiceError("not_found", `Connector ${id} not found`)
      return {
        ...c,
        discoveredAssets: 14,
        recentErrors:
          c.health === "degraded"
            ? [{ at: agoIso(12), message: "Consumer lag growing on partition 3" }]
            : [],
        dependentPipelines: ["orders_hourly_rollup", "rt.payments_flow_mv"],
      }
    }, { signal })
  },
}
