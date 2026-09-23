import type {
  CreateRoleInput,
  CreateServiceIdentityInput,
  CreateTenantInput,
  IdentityService,
  InviteUserInput,
  Role,
  RotateServiceIdentityResponse,
  ServiceIdentity,
  Tenant,
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

export const postgresIdentityService: IdentityService = {
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
  inviteUser(input: InviteUserInput, signal) {
    return post<User>("/api/identity/users", input, signal, "Failed to invite user");
  },
  createRole(input: CreateRoleInput, signal) {
    return post<Role>("/api/identity/roles", input, signal, "Failed to create role");
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
};
