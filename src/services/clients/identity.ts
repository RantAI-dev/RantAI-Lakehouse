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
  WorkspaceSettings,
} from "../contracts/identity";
import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * IdentityService NYATA — pengguna, peran, tenant, dan service identity
 * tersimpan di Postgres lewat route `/api/identity/*` (crate `lakehouse-store`).
 * Menggantikan seluruh `mock/identity.ts`; tidak ada method yang masih mock.
 *
 * Catatan: backend ini TIDAK punya lapisan autentikasi. Semua endpoint di
 * bawah — termasuk tiga POST yang membuat baris direktori sungguhan — terbuka
 * bagi siapa pun yang bisa menjangkau servisnya. Itu gap produk yang sedang
 * dieskalasi terpisah, bukan sesuatu yang ditambal di adapter ini.
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
  return request<T>(
    url,
    {
      method: "POST",
      headers: { "content-type": "application/json" },
      body: JSON.stringify(body),
      signal,
    },
    fallback
  );
}

export const postgresIdentityService: IdentityService = {
  listUsers(signal) {
    return get<User[]>("/api/identity/users", signal, "Daftar pengguna gagal dimuat");
  },
  listRoles(signal) {
    return get<Role[]>("/api/identity/roles", signal, "Daftar peran gagal dimuat");
  },
  listTenants(signal) {
    return get<Tenant[]>("/api/identity/tenants", signal, "Daftar tenant gagal dimuat");
  },
  listServiceIdentities(signal) {
    return get<ServiceIdentity[]>(
      "/api/identity/service-identities",
      signal,
      "Daftar service identity gagal dimuat"
    );
  },
  getWorkspaceSettings(signal) {
    return get<WorkspaceSettings>(
      "/api/identity/workspace-settings",
      signal,
      "Pengaturan workspace gagal dimuat"
    );
  },
  inviteUser(input: InviteUserInput, signal) {
    return post<User>("/api/identity/users", input, signal, "Undangan pengguna gagal");
  },
  createRole(input: CreateRoleInput, signal) {
    return post<Role>("/api/identity/roles", input, signal, "Pembuatan peran gagal");
  },
  createTenant(input: CreateTenantInput, signal) {
    return post<Tenant>("/api/identity/tenants", input, signal, "Pembuatan tenant gagal");
  },
  createServiceIdentity(input: CreateServiceIdentityInput, signal) {
    return post<ServiceIdentity>(
      "/api/identity/service-identities",
      input,
      signal,
      "Pembuatan service identity gagal"
    );
  },
};
