import { apiFetch } from "../http";
import { ServiceError } from "../errors";

/**
 * Auth client — wraps `/api/auth/*` (Task 3.2 backend, Task 3.3 frontend).
 * Not a `services/index.ts` domain (no mock counterpart makes sense for
 * authentication), so it is imported directly by `AuthProvider` and the
 * login/change-password pages rather than registered in the swap table.
 */

/** Mirrors `MeResponse` from `rust/crates/lakehouse-api/src/routes/auth.rs`. */
export type AuthUser = {
  id: string;
  name: string;
  email: string | null;
  roles: string[];
  permissions: string[];
  tenants: string[];
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
