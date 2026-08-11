import { ServiceError } from "../errors"
import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import { createStore } from "./mutable-store"
import type {
  Connector,
  ConnectorService,
  CreateConnectorInput,
} from "../contracts/connectors"

const store = createStore<Connector>([
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
])

const DEPENDENTS: Record<
  string,
  { id: string; name: string; kind: "pipeline" | "streaming" }[]
> = {
  "conn-pg-core": [
    { id: "pl-orders-rollup", name: "orders_hourly_rollup", kind: "pipeline" },
  ],
  "conn-s3-docs": [
    { id: "pl-policy-docs", name: "credit_policy_ingest", kind: "pipeline" },
  ],
  "conn-kafka-payments": [
    { id: "sj-payments-flow", name: "rt.payments_flow_mv", kind: "streaming" },
  ],
}

const SCHEMAS: Record<
  string,
  { name: string; kind: "table" | "topic" | "prefix"; columnsOrFields: number }[]
> = {
  "conn-pg-core": [
    { name: "public.orders_events", kind: "table", columnsOrFields: 18 },
    { name: "public.accounts", kind: "table", columnsOrFields: 22 },
  ],
  "conn-s3-docs": [
    { name: "s3://docs/credit-policy/", kind: "prefix", columnsOrFields: 0 },
  ],
  "conn-kafka-payments": [
    { name: "payments.events", kind: "topic", columnsOrFields: 12 },
    { name: "payments.settlements", kind: "topic", columnsOrFields: 9 },
  ],
}

export const mockConnectorService: ConnectorService = {
  listConnectors(signal) {
    return mockCall(() => store.list(), { signal })
  },
  getConnector(id, signal) {
    return mockCall(() => {
      const c = store.get(id)
      if (!c) throw new ServiceError("not_found", `Connector ${id} not found`)
      const schemas = SCHEMAS[id] ?? []
      return {
        ...c,
        discoveredAssets: schemas.length || 14,
        discoveredSchemas: schemas,
        recentErrors:
          c.health === "degraded"
            ? [{ at: agoIso(12), message: "Consumer lag growing on partition 3" }]
            : [],
        dependentPipelines: DEPENDENTS[id] ?? [],
        auditEventId: `aud-conn-${id}`,
      }
    }, { signal })
  },
  createConnector(input: CreateConnectorInput, signal) {
    return mockCall(
      () => {
        const connector: Connector = {
          id: `conn-${Date.now().toString(36)}`,
          name: input.name,
          type: input.type,
          direction: input.direction,
          health: "healthy",
          environment: input.environment,
          tenant: input.tenant,
          lastTestAt: agoIso(0),
          lastActivityAt: agoIso(0),
          capabilities: input.capabilities,
          owner: input.owner ?? "Current user",
        }
        return store.prepend(connector)
      },
      { signal, delayMs: 500 }
    )
  },
  testConnection(id, signal) {
    return mockCall(() => {
      const c = store.get(id)
      if (!c) throw new ServiceError("not_found", `Connector ${id} not found`)
      const ok = c.health !== "unhealthy"
      store.update(id, { lastTestAt: agoIso(0) })
      return {
        ok,
        latencyMs: ok ? 84 : 2400,
        message: ok
          ? "Connection succeeded. Schema discovery available."
          : "Connection failed: endpoint unreachable.",
        testedAt: agoIso(0),
      }
    }, { signal, delayMs: 700 })
  },
}
