import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import type { IdentityService } from "../contracts/identity"

export const mockIdentityService: IdentityService = {
  listUsers(signal) {
    return mockCall(
      () => [
        {
          id: "u-1",
          name: "Rina Wijaya",
          email: "rina@rantai.id",
          status: "active" as const,
          roles: ["Analyst", "Approver"],
          tenants: ["Nusantara Finance"],
          lastActivity: agoIso(9),
        },
        {
          id: "u-2",
          name: "Bayu Pratama",
          email: "bayu@rantai.id",
          status: "active" as const,
          roles: ["Data Engineer"],
          tenants: ["Nusantara Finance"],
          lastActivity: agoIso(40),
        },
        {
          id: "u-3",
          name: "Dewi Anggraini",
          email: "dewi@rantai.id",
          status: "active" as const,
          roles: ["Governance Admin"],
          tenants: ["Nusantara Finance", "Retail Analytics"],
          lastActivity: agoIso(60),
        },
      ],
      { signal }
    )
  },
  listRoles(signal) {
    return mockCall(
      () => [
        {
          id: "role-analyst",
          name: "Analyst",
          members: 24,
          permissions: "query:read, catalog:read, lineage:read",
          description: "Read governed data and run approved queries.",
        },
        {
          id: "role-approver",
          name: "Approver",
          members: 6,
          permissions: "agent:approve, policy:review",
          description: "Approve L3 agent actions and policy changes.",
        },
        {
          id: "role-admin",
          name: "Governance Admin",
          members: 3,
          permissions: "policy:*, residency:*, audit:read",
          description: "Manage policies, residency, and classifications.",
        },
      ],
      { signal }
    )
  },
  listTenants(signal) {
    return mockCall(
      () => [
        {
          id: "t-nusantara",
          name: "Nusantara Finance",
          slug: "nusantara-finance",
          plan: "Enterprise",
          residency: "Jakarta (ID) + Singapore (SG)",
          users: 42,
          agents: 6,
          storageBytes: 64 * 1024 ** 4,
          quotaCompute: 20000,
          usedCompute: 12400,
        },
        {
          id: "t-retail",
          name: "Retail Analytics",
          slug: "retail-analytics",
          plan: "Standard",
          residency: "Singapore (SG)",
          users: 18,
          agents: 2,
          storageBytes: 12 * 1024 ** 4,
          quotaCompute: 8000,
          usedCompute: 4200,
        },
      ],
      { signal }
    )
  },
  listServiceIdentities(signal) {
    return mockCall(
      () => [
        {
          id: "si-1",
          name: "bi-dashboard-reader",
          scopes: ["query:read", "catalog:read"],
          environment: "production",
          expiresAt: agoIso(-60 * 24 * 30),
          rotationStatus: "current" as const,
          lastUsedAt: agoIso(15),
        },
        {
          id: "si-2",
          name: "ingestion-worker",
          scopes: ["ingest:write", "catalog:register"],
          environment: "production",
          expiresAt: agoIso(-60 * 24 * 7),
          rotationStatus: "due" as const,
          lastUsedAt: agoIso(2),
        },
      ],
      { signal }
    )
  },
  getWorkspaceSettings(signal) {
    return mockCall(
      () => ({
        workspaceName: "Rantai Lake",
        defaultEnvironment: "production",
        defaultTenant: "nusantara-finance",
        interfaceTheme: "dark" as const,
        serviceAdapter: "mock" as const,
        auditRetentionDays: 365,
        queryResultRetentionDays: 30,
      }),
      { signal }
    )
  },
}
