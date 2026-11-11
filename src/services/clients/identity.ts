import type {
  CreateRoleInput,
  CreateServiceIdentityInput,
  CreateTenantInput,
  IdentityService,
  InviteUserInput,
  Permission,
  InviteUserResponse,
  ResetPasswordResponse,
  Role,
  RotateServiceIdentityResponse,
  RoleTableAccess,
  ServiceIdentity,
  SetRoleTableAccessInput,
  Tenant,
  UpdateRoleInput,
  UpdateUserInput,
  User,
} from "../contracts/identity";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * IdentityService is real — users, roles, tenants, and service identities
 * are stored in Postgres via the `/api/identity/*` route (crate
 * `lakehouse-store`). Replaces all of `mock/identity.ts`; no method here is
 * still mocked.
 *
 * Note: this backend has NO authentication layer. Every endpoint below —
 * including the three POSTs that create real directory rows — is open to
 * anyone who can reach the service. That is a product gap being escalated
 * separately, not something patched over in this adapter.
 */

/** Map an error response body onto the ServiceError code its status implies. */
function errorFor(status: number, message: string): ServiceError {
  if (status === 404) return new ServiceError("not_found", message);
  if (status === 400 || status === 409 || status === 422)
    return new ServiceError("invalid_request", message);
  if (status === 401 || status === 403)
    return new ServiceError("permission_denied", message);
  return new ServiceError("unavailable", message);
}

async function request<T>(
  url: string,
  init: RequestInit,
  fallback: string
): Promise<T> {
  const res = await apiFetch(url, init);
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    throw errorFor(res.status, json?.error ?? fallback);
  }
  return json as T;
}

function get<T>(url: string, signal: AbortSignal | undefined, fallback: string): Promise<T> {
  return request<T>(url, { signal }, fallback);
}

function post<T>(
  url: string,
  body: unknown,
  signal: AbortSignal | undefined,
  fallback: string
): Promise<T> {
  // Empty-body POSTs (`rotateServiceIdentity` is the only caller today)
  // pass `null`; we still set `content-type: application/json` so the
  // request looks like every other POST in this adapter.
  return request<T>(
    url,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: body === null || body === undefined ? undefined : JSON.stringify(body),
      signal,
    },
    fallback
  );
}

/** `PATCH` a JSON body (ADM-11 D8 user rename). */
function patch<T>(
  url: string,
  body: unknown,
  signal: AbortSignal | undefined,
  fallback: string
): Promise<T> {
  return request<T>(
    url,
    {
      method: "PATCH",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      signal,
    },
    fallback
  );
}

/** `PUT` a JSON body. ADM-4 D1's role change is the only caller today. */
function put<T>(
  url: string,
  body: unknown,
  signal: AbortSignal | undefined,
  fallback: string
): Promise<T> {
  return request<T>(
    url,
    {
      method: "PUT",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      signal,
    },
    fallback
  );
}

/** `DELETE` a resource. ADM-4 D2's tenant removal is the only caller. */
function del<T>(
  url: string,
  signal: AbortSignal | undefined,
  fallback: string
): Promise<T> {
  return request<T>(url, { method: "DELETE", signal }, fallback);
}

export const postgresIdentityService: IdentityService = {
  listPermissions(signal) {
    return get<Permission[]>(
      "/api/identity/permissions",
      signal,
      "Failed to load permission list"
    );
  },
  listUsers(signal) {
    return get<User[]>("/api/identity/users", signal, "Failed to load user list");
  },
  listRoles(signal) {
    return get<Role[]>("/api/identity/roles", signal, "Failed to load role list");
  },
  listTenants(signal) {
    return get<Tenant[]>("/api/identity/tenants", signal, "Failed to load tenant list");
  },
  listServiceIdentities(signal) {
    return get<ServiceIdentity[]>(
      "/api/identity/service-identities",
      signal,
      "Failed to load service identity list"
    );
  },
  // ADM-3 D1: the invite comes back as the created user PLUS the
  // server-generated temporary password, shown once. The password is never
  // persisted client-side beyond the one-shot dialog that displays it.
  inviteUser(input: InviteUserInput, signal) {
    return post<InviteUserResponse>(
      "/api/identity/users",
      input,
      signal,
      "Failed to invite user"
    );
  },
  createRole(input: CreateRoleInput, signal) {
    return post<Role>("/api/identity/roles", input, signal, "Failed to create role");
  },
  // WEB-3 S3a-D2: a role can be rewritten and removed. The API's 409
  // sentence (still held / built-in) reaches the console through
  // `ServiceError.message` unchanged.
  updateRole(id, input: UpdateRoleInput, signal) {
    return put<Role>(
      `/api/identity/roles/${encodeURIComponent(id)}`,
      input,
      signal,
      "Failed to update role"
    );
  },
  deleteRole(id, signal) {
    return del<void>(
      `/api/identity/roles/${encodeURIComponent(id)}`,
      signal,
      "Failed to delete role"
    );
  },
  // GOV-7 D7: what a role may read. A `*:*` role and a name that is not a
  // plain identifier are refused on the write with a 400; `errorFor` keeps
  // the API's own sentence in `ServiceError.message`, so the page shows it
  // unchanged.
  getRoleTableAccess(id, signal) {
    return get<RoleTableAccess>(
      `/api/identity/roles/${encodeURIComponent(id)}/table-access`,
      signal,
      "Failed to load the role's tables"
    );
  },
  setRoleTableAccess(id, input: SetRoleTableAccessInput, signal) {
    return put<RoleTableAccess>(
      `/api/identity/roles/${encodeURIComponent(id)}/table-access`,
      input,
      signal,
      "Failed to save the role's tables"
    );
  },
  createTenant(input: CreateTenantInput, signal) {
    return post<Tenant>("/api/identity/tenants", input, signal, "Failed to create tenant");
  },
  createServiceIdentity(input: CreateServiceIdentityInput, signal) {
    return post<ServiceIdentity>(
      "/api/identity/service-identities",
      input,
      signal,
      "Failed to create service identity"
    );
  },
  // Route was added in commit `840cfd6`. Returns the freshly
  // minted token in `secret` exactly once — see the contract type's
  // doc comment for why this shape is the only honest read-back. A 404
  // maps to `not_found`, anything else falls through to `errorFor`'s
  // generic mapping.
  rotateServiceIdentity(id: string, signal) {
    return post<RotateServiceIdentityResponse>(
      `/api/identity/service-identities/${id}/rotate`,
      null,
      signal,
      "Failed to rotate service identity"
    );
  },
  // ADM-3 D2: hand an existing user a new temporary password. The server
  // generates it and returns it exactly once — see the contract type's
  // doc comment. The route takes no body, so this is an empty-body POST
  // like `rotateServiceIdentity`.
  resetPassword(id: string, signal) {
    return post<ResetPasswordResponse>(
      `/api/identity/users/${id}/reset-password`,
      null,
      signal,
      "Failed to reset password"
    );
  },
  // ADM-4 D1: replace a user's local role set. The body carries the
  // user's COMPLETE set of role names — a role left out is a role the
  // user no longer holds.
  setUserRoles(id: string, roles: string[], signal) {
    return put<User>(
      `/api/identity/users/${id}/roles`,
      { roles },
      signal,
      "Failed to update roles"
    );
  },
  // ADM-4 D2: take a user out of a tenant.
  removeUserFromTenant(id: string, tenantId: string, signal) {
    return del<User>(
      `/api/identity/users/${id}/tenants/${tenantId}`,
      signal,
      "Failed to remove the user from the tenant"
    );
  },
  // ADM-4 D3: switch a user off, and back on again. Both routes take no
  // body, so both are empty-body POSTs.
  updateUser(id: string, input: UpdateUserInput, signal) {
    return patch<User>(
      `/api/identity/users/${id}`,
      input,
      signal,
      "Failed to update user"
    );
  },
  deactivateUser(id: string, signal) {
    return post<User>(
      `/api/identity/users/${id}/deactivate`,
      null,
      signal,
      "Failed to deactivate the user"
    );
  },
  reactivateUser(id: string, signal) {
    return post<User>(
      `/api/identity/users/${id}/reactivate`,
      null,
      signal,
      "Failed to reactivate the user"
    );
  },
};
