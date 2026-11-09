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

/**
 * One entry of the central permission catalog (`WEB-8` D1). Served by
 * `GET /api/identity/permissions` in catalog order.
 */
export type Permission = {
  name: string
  area: string
  description: string
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

/**
 * What `PUT /api/identity/users/{id}/roles` takes (ADM-4 D1).
 *
 * The list is the user's COMPLETE local role set, not a delta: what the
 * user holds after the call is exactly what this names. Roles mapped
 * from an identity provider's groups are not stored and are not part of
 * it — the server re-resolves those from the provider's claims on every
 * request, and the user keeps the union of the two.
 */
export type SetUserRolesInput = {
  /** Role names. An empty list leaves the user with no local role. */
  roles: string[]
}

export type CreateRoleInput = {
  name: string
  permissions: string
  description: string
}

/**
 * What a role is rewritten with (WEB-3 S3a-D2). The name must not collide
 * with another role's. A rename is not rewritten into a policy's free-text
 * `subjects`, so a policy that named the old name stops applying to that
 * role.
 */
export type UpdateRoleInput = {
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
  /**
   * The central list of permission strings the API checks (`WEB-8` D1/D4).
   */
  listPermissions(signal?: AbortSignal): Promise<Permission[]>
  listUsers(signal?: AbortSignal): Promise<User[]>
  listRoles(signal?: AbortSignal): Promise<Role[]>
  listTenants(signal?: AbortSignal): Promise<Tenant[]>
  listServiceIdentities(signal?: AbortSignal): Promise<ServiceIdentity[]>
  inviteUser(
    input: InviteUserInput,
    signal?: AbortSignal
  ): Promise<InviteUserResponse>
  createRole(input: CreateRoleInput, signal?: AbortSignal): Promise<Role>
  /**
   * Rewrite a role's name, permissions and description; needs
   * `identity:write`. 409 when another role has the name.
   */
  updateRole(id: string, input: UpdateRoleInput, signal?: AbortSignal): Promise<Role>
  /**
   * Remove a role nobody holds; needs `identity:write`. 409 with the API's
   * own sentence when people still hold it, or when it is a built-in role.
   */
  deleteRole(id: string, signal?: AbortSignal): Promise<void>
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
  /**
   * Replace a user's local role set (ADM-4 D1).
   *
   * `roles` is the user's COMPLETE set of local role names, not a delta:
   * a role left out of the list is a role the user no longer holds. The
   * server refuses an unknown role name, and refuses the change when it
   * would leave the deployment without an active administrator.
   */
  setUserRoles(
    id: string,
    roles: string[],
    signal?: AbortSignal
  ): Promise<User>
  /**
   * Take a user out of a tenant (ADM-4 D2) — the counterpart of giving
   * one to a tenant.
   */
  removeUserFromTenant(
    id: string,
    tenantId: string,
    signal?: AbortSignal
  ): Promise<User>
  /**
   * Switch a user off (ADM-4 D3): they can no longer sign in, and every
   * session they hold ends at once. Their data stays.
   */
  deactivateUser(id: string, signal?: AbortSignal): Promise<User>
  /**
   * Switch a user back on (ADM-4 D3).
   */
  reactivateUser(id: string, signal?: AbortSignal): Promise<User>
}
