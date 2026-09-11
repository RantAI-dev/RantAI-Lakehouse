export type User = {
  id: string
  name: string
  email: string
  status: "active" | "inactive"
  roles: string[]
  tenants: string[]
  // Nothing writes app_user.last_activity_at (no login, session, or token
  // use records it), so the server never serves a fabricated value here.
  lastActivity: string | null
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
  // The server reports "expired" once the credential is past expiresAt,
  // matching the check service-token authentication already applies.
  rotationStatus: "current" | "due" | "expired"
  // Nothing writes service_identity.last_used_at, so the server never
  // serves a fabricated value here.
  lastUsedAt: string | null
}

export type InviteUserInput = {
  name: string
  email: string
  roles: string[]
  tenants: string[]
}

export type CreateRoleInput = {
  name: string
  permissions: string
  description: string
}

export type CreateTenantInput = {
  name: string
  slug: string
  plan: string
  residency: string
}

export type CreateServiceIdentityInput = {
  name: string
  scopes: string[]
  environment: string
}

export interface IdentityService {
  listUsers(signal?: AbortSignal): Promise<User[]>
  listRoles(signal?: AbortSignal): Promise<Role[]>
  listTenants(signal?: AbortSignal): Promise<Tenant[]>
  listServiceIdentities(signal?: AbortSignal): Promise<ServiceIdentity[]>
  inviteUser(input: InviteUserInput, signal?: AbortSignal): Promise<User>
  createRole(input: CreateRoleInput, signal?: AbortSignal): Promise<Role>
  createTenant(input: CreateTenantInput, signal?: AbortSignal): Promise<Tenant>
  createServiceIdentity(
    input: CreateServiceIdentityInput,
    signal?: AbortSignal
  ): Promise<ServiceIdentity>
}
