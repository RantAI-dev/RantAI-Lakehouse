import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * Auth client — wraps `/api/auth/*` (Task 3.2 backend, Task 3.3 frontend).
 * The login/logout/me/change-password methods are NOT a `services/index.ts`
 * domain (no mock counterpart makes sense for authentication), so they are
 * imported directly by `AuthProvider` and the login/change-password pages
 * rather than registered in the swap table.
 *
 * `providers` IS registered — the SSO admin page reads it via
 * `authService.providers`, and the page has no other way to ask "is OIDC
 * configured on this deployment" (`routes/auth.rs:731`). It is the only
 * auth method that fits the read-only, page-shaped pattern the rest of the
 * service registry follows. See WS8 §Phase F.
 */

/** Mirrors `MeResponse` from `rust/crates/lakehouse-api/src/routes/auth.rs`. */
export type AuthUser = {
  id: string;
  name: string;
  email: string | null;
  roles: string[];
  permissions: string[];
  // Slim {id, name, slug} view of every tenant the caller belongs to —
  // matches `lakehouse_store::identity::TenantSummary` exactly, so a
  // tenant-switcher can render a label without a second round trip.
  // Empty for a service principal or when the follow-up read fails (WS8
  // §Phase F).
  tenants: { id: string; name: string; slug: string }[];
};

/** Mirrors `LoginResponse`. */
export type LoginResult = {
  id: string;
  name: string;
  mustChangePassword: boolean;
};

async function parse<T>(res: Response, fallback: string): Promise<T> {
  const json = await res.json().catch(() => null);
  if (!res.ok) {
    if (res.status === 401) {
      // Same message regardless of *why* — non-enumeration mirrors the
      // backend's own guarantee (see `routes/auth.rs`'s module doc
      // comment): never reveal "no such email" vs "wrong password".
      throw new ServiceError("permission_denied", json?.error ?? "Invalid email or password.");
    }
    if (res.status === 400 || res.status === 422) {
      throw new ServiceError("invalid_request", json?.error ?? fallback);
    }
    throw new ServiceError("unavailable", json?.error ?? fallback);
  }
  return json as T;
}

export async function login(email: string, password: string): Promise<LoginResult> {
  const res = await apiFetch("/api/auth/login", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ email, password }),
  });
  return parse<LoginResult>(res, "Login failed.");
}

export async function logout(): Promise<void> {
  // Best-effort: even if this fails (network blip, already-expired
  // session) the caller clears local state and navigates to /login anyway.
  await apiFetch("/api/auth/logout", { method: "POST" }).catch(() => {});
}

export async function me(signal?: AbortSignal): Promise<AuthUser> {
  const res = await apiFetch("/api/auth/me", { signal });
  return parse<AuthUser>(res, "Failed to load session.");
}

export async function changePassword(input: {
  oldPassword?: string;
  newPassword: string;
}): Promise<void> {
  const res = await apiFetch("/api/auth/change-password", {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify(input),
  });
  await parse<unknown>(res, "Failed to change password.");
}

/**
 * Mirrors `ProvidersResponse` from `routes/auth.rs:706-711` — the
 * `camelCase` rename is enforced by the Rust struct's
 * `#[serde(rename_all = "camelCase")]`, so the wire shape is
 * `{ oidc: boolean; providerName: string | null }`. `providerName` is
 * `null` when `oidc` is `false` (the route never sets one without the
 * other), so consumers can treat `oidc === false` as the not-configured
 * signal and ignore `providerName` in that branch.
 */
export type ProvidersResponse = {
  oidc: boolean;
  providerName: string | null;
};

/**
 * `GET /api/auth/providers` (WS8 §Phase A). The OIDC runtime flag
 * is read off the API process's in-memory `AuthState`/`Config`, so the
 * SSO admin page (WS8 §Phase F) can never drift from the
 * backend's real state the way a build-time flag could.
 */
export async function providers(signal?: AbortSignal): Promise<ProvidersResponse> {
  const res = await apiFetch("/api/auth/providers", { signal });
  return parse<ProvidersResponse>(res, "Failed to load SSO providers.");
}
