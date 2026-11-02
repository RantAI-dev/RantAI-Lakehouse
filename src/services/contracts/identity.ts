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

/**
 * What `POST /api/identity/users` returns (ADM-3 D1).
 *
 * The server creates the account AND a `local` credential, and hands back
 * a temporary password it generated itself — the caller never chooses one.
 * The password is returned EXACTLY ONCE, on this response: the server
 * keeps only the `Argon2id` hash, never logs it, never audits it, and no
 * other route ever serves it.
 *
 * `User` is flattened into the response, so the object IS the user plus
 * this one field.
 */
export type InviteUserResponse = User & {
  /**
   * The server-generated temporary password. Shown once and never
   * recoverable: the user must change it at first sign-in.
   */
  temporaryPassword: string
}

/**
 * What `POST /api/identity/users/{id}/reset-password` returns (ADM-3 D2).
 *
 * The same one-shot promise as the invite's temporary password: the reset
 * hands out a new password, flags it `must_change_password`, and ends every
 * live session the target holds.
 */
export type ResetPasswordResponse = {
  /**
   * The server-generated temporary password. Shown once and never
   * recoverable.
   */
  temporaryPassword: string
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

/**
 * What `POST /api/identity/service-identities/{id}/rotate` returns.
 * The secret must never be persisted client-side beyond this one-shot read.
 *
 * `secret` is the freshly minted raw token, returned exactly once on this
 * response — `ServiceIdentity` itself has no secret/token_hash field, so
 * every subsequent list/get route on the same identity continues to return
 * credential *metadata* only. The backend that produces this shape is
 * `lakehouse-store::identity::rotate_service_identity` plus its route
 * (`routes::identity::rotate`), both wired up in commit `840cfd6`.
 */
export type RotateServiceIdentityResponse = {
  secret: string
  identity: ServiceIdentity
}

export interface IdentityService {
  listUsers(signal?: AbortSignal): Promise<User[]>
  listRoles(signal?: AbortSignal): Promise<Role[]>
  listTenants(signal?: AbortSignal): Promise<Tenant[]>
  listServiceIdentities(signal?: AbortSignal): Promise<ServiceIdentity[]>
  inviteUser(
    input: InviteUserInput,
    signal?: AbortSignal
  ): Promise<InviteUserResponse>
  createRole(input: CreateRoleInput, signal?: AbortSignal): Promise<Role>
  createTenant(input: CreateTenantInput, signal?: AbortSignal): Promise<Tenant>
  createServiceIdentity(
    input: CreateServiceIdentityInput,
    signal?: AbortSignal
  ): Promise<ServiceIdentity>
  rotateServiceIdentity(
    id: string,
    signal?: AbortSignal
  ): Promise<RotateServiceIdentityResponse>
  /**
   * Hand an existing user a new temporary password (ADM-3 D2). The server
   * generates it and returns it once; the reset also ends every live
   * session the target holds.
   */
  resetPassword(
    id: string,
    signal?: AbortSignal
  ): Promise<ResetPasswordResponse>
}
