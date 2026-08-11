export type User = {
  id: string
  name: string
  email: string
  status: "active" | "inactive"
  roles: string[]
  tenants: string[]
  lastActivity: string
}

export type Role = {
  id: string
  name: string
  members: number
  permissions: string
  description: string
}

export type Tenant = {
  id: string
  name: string
  slug: string
  plan: string
  residency: string
  users: number
  agents: number
  storageBytes: number
  quotaCompute: number
  usedCompute: number
}

export type ServiceIdentity = {
  id: string
  name: string
  scopes: string[]
  environment: string
  expiresAt: string
  rotationStatus: "current" | "due" | "expired"
  lastUsedAt: string
}

export type WorkspaceSettings = {
  workspaceName: string
  defaultEnvironment: string
  defaultTenant: string
  interfaceTheme: "dark" | "light" | "system"
  serviceAdapter: "mock" | "http"
  auditRetentionDays: number
  queryResultRetentionDays: number
}

export interface IdentityService {
  listUsers(signal?: AbortSignal): Promise<User[]>
  listRoles(signal?: AbortSignal): Promise<Role[]>
  listTenants(signal?: AbortSignal): Promise<Tenant[]>
  listServiceIdentities(signal?: AbortSignal): Promise<ServiceIdentity[]>
  getWorkspaceSettings(signal?: AbortSignal): Promise<WorkspaceSettings>
}
