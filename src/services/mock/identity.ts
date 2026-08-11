import { mockCall } from "../transport"
import { agoIso } from "./mock-time"
import { createStore } from "./mutable-store"
import type {
  CreateRoleInput,
  CreateServiceIdentityInput,
  CreateTenantInput,
  IdentityService,
  InviteUserInput,
  Role,
  ServiceIdentity,
  Tenant,
  User,
} from "../contracts/identity"

const userStore = createStore<User>([
  {
    id: "u-1",
    name: "Rina Wijaya",
    email: "rina@rantai.id",
    status: "active",
    roles: ["Analyst", "Approver"],
    tenants: ["Nusantara Finance"],
    lastActivity: agoIso(9),
  },
  {
    id: "u-2",
    name: "Bayu Pratama",
    email: "bayu@rantai.id",
    status: "active",
    roles: ["Data Engineer"],
    tenants: ["Nusantara Finance"],
    lastActivity: agoIso(40),
  },
  {
    id: "u-3",
    name: "Dewi Anggraini",
    email: "dewi@rantai.id",
    status: "active",
    roles: ["Governance Admin"],
    tenants: ["Nusantara Finance", "Retail Analytics"],
    lastActivity: agoIso(60),
  },
])

const roleStore = createStore<Role>([
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
])

const tenantStore = createStore<Tenant>([
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
])

const serviceIdentityStore = createStore<ServiceIdentity>([
  {
    id: "si-1",
    name: "bi-dashboard-reader",
    scopes: ["query:read", "catalog:read"],
    environment: "production",
    expiresAt: agoIso(-60 * 24 * 30),
    rotationStatus: "current",
    lastUsedAt: agoIso(15),
  },
  {
    id: "si-2",
    name: "ingestion-worker",
    scopes: ["ingest:write", "catalog:register"],
    environment: "production",
    expiresAt: agoIso(-60 * 24 * 7),
    rotationStatus: "due",
    lastUsedAt: agoIso(2),
  },
])

export const mockIdentityService: IdentityService = {
  listUsers(signal) {
    return mockCall(() => userStore.list(), { signal })
  },
  listRoles(signal) {
    return mockCall(() => roleStore.list(), { signal })
  },
  listTenants(signal) {
    return mockCall(() => tenantStore.list(), { signal })
  },
  listServiceIdentities(signal) {
    return mockCall(() => serviceIdentityStore.list(), { signal })
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
  inviteUser(input: InviteUserInput, signal) {
    return mockCall(
      () =>
        userStore.prepend({
          id: `u-${Date.now().toString(36)}`,
          name: input.name,
          email: input.email,
          status: "active",
          roles: input.roles,
          tenants: input.tenants,
          lastActivity: agoIso(0),
        }),
      { signal, delayMs: 400 }
    )
  },
  createRole(input: CreateRoleInput, signal) {
    return mockCall(
      () =>
        roleStore.prepend({
          id: `role-${Date.now().toString(36)}`,
          name: input.name,
          members: 0,
          permissions: input.permissions,
          description: input.description,
        }),
      { signal, delayMs: 400 }
    )
  },
  createTenant(input: CreateTenantInput, signal) {
    return mockCall(
      () =>
        tenantStore.prepend({
          id: `t-${Date.now().toString(36)}`,
          name: input.name,
          slug: input.slug,
          plan: input.plan,
          residency: input.residency,
          users: 0,
          agents: 0,
          storageBytes: 0,
          quotaCompute: 5000,
          usedCompute: 0,
        }),
      { signal, delayMs: 400 }
    )
  },
  createServiceIdentity(input: CreateServiceIdentityInput, signal) {
    return mockCall(
      () =>
        serviceIdentityStore.prepend({
          id: `si-${Date.now().toString(36)}`,
          name: input.name,
          scopes: input.scopes,
          environment: input.environment,
          expiresAt: agoIso(-60 * 24 * 90),
          rotationStatus: "current",
          lastUsedAt: agoIso(0),
        }),
      { signal, delayMs: 400 }
    )
  },
}
