"use client";

import * as React from "react";
import { usePathname, useRouter } from "next/navigation";
import * as authClient from "@/services/clients/auth";
import type { AuthUser } from "@/services/clients/auth";
import { clearActiveTenantId, readActiveTenantId, writeActiveTenantId } from "@/services/http";

/**
 * Route prefixes reachable without a session. Kept in sync with
 * `AppFrame`'s own chrome-bypass check (`app-frame.tsx`) and with what the
 * Rust backend treats as public (`GET /health`, `GET
 * /api/public/dashboard/{token}`, `POST /api/embed/data`) — see
 * `rust/crates/lakehouse-api/src/routes/mod.rs`'s public route table.
 *
 * `/login` is public by construction (that's the page that establishes a
 * session). `/public` and `/embed` back the two product surfaces meant to
 * be shared with people who never sign in (Metabase-style public dashboard
 * links and embeds) — gating them would silently break that feature.
 *
 * `/render` is the PDF print view (`BI-1`): the Gotenberg container that makes
 * the PDF has no session, so the page reads its data with a render token in
 * the address (`POST /api/render/data`) and renders without the console chrome.
 */
const PUBLIC_PREFIXES = ["/login", "/public", "/embed", "/render"];

export function isPublicPath(pathname: string): boolean {
  return PUBLIC_PREFIXES.some((p) => pathname === p || pathname.startsWith(`${p}/`));
}

export type AuthStatus = "loading" | "authenticated" | "unauthenticated";

// The active-tenant key and its read/write helpers live in `@/services/http`,
// next to `apiFetch`, which reads them to set `X-Tenant`. Re-exported here so
// existing importers of this module keep working without a second
// definition that could drift from the reader.
export {
  ACTIVE_TENANT_STORAGE_KEY,
  clearActiveTenantId,
  readActiveTenantId,
  writeActiveTenantId,
} from "@/services/http";

/**
 * Which tenant the signed-in person acts as (`ADM-15`, #483 review, D4): the
 * stored one when it is still theirs, otherwise the first they belong to,
 * otherwise none.
 *
 * One function decides, used by `load()` and by the context value, so
 * `activeTenantId` can never name a tenant the person is not in — the server
 * answers `404` for such a header (`tenant_scope.rs`). Writing the first
 * tenant when nothing (or a stale value) is stored goes one step past "ignore
 * it": with nothing stored the console shows `me.tenants[0]` and sends no
 * header, while the server takes the first of `principal.tenant_ids`, and the
 * two lists are not in the same order.
 */
export function resolveActiveTenant(
  stored: string | null,
  tenants: readonly { id: string }[]
): string | null {
  if (stored !== null && tenants.some((t) => t.id === stored)) return stored;
  return tenants[0]?.id ?? null;
}

type AuthContextValue = {
  user: AuthUser | null;
  status: AuthStatus;
  /** True once `roles`/`permissions` are known to be authoritative (i.e. not mid-initial-load). */
  hasPermission: (permission: string) => boolean;
  hasRole: (role: string) => boolean;
  refresh: () => Promise<void>;
  logout: () => Promise<void>;
  /**
   * The tenant the caller is currently acting as — the one they last picked
   * through the navbar's `TenantSwitcher` (persisted in `localStorage` as
   * `lh_active_tenant`) when it is still theirs, otherwise the first tenant
   * they belong to, otherwise `null` (`ADM-15`, #483 review).
   *
   * Always one of the signed-in person's own tenants, or `null`: [`load`]
   * corrects the stored value against `GET /api/auth/me` before the tree
   * renders, and [`resolveActiveTenant`] is what both read through.
   */
  activeTenantId: string | null;
  /**
   * Persist `id` as the active tenant and update the in-memory value so
   * the next render of every `useAuth()` consumer reflects the choice.
   * The navbar's `TenantSwitcher` wires `onSwitch` to this AND to a
   * `router.refresh()` (see `app-navbar.tsx`) — every tenant-scoped
   * list route reads `X-Tenant` at request time, so a
   * client-side re-filter would be dishonest (it would still be showing
   * data fetched under the OLD tenant's scope until the next real
   * request), which is why we force a server refetch instead.
   */
  setActiveTenant: (id: string) => void;
};

const AuthContext = React.createContext<AuthContextValue | null>(null);

/**
 * Mirrors `PermissionSet::has` (`rust/crates/lakehouse-auth/src/
 * permissions.rs`): a granted token is `"resource:action"`, and either half
 * may be a `"*"` wildcard (a Platform Admin's set is the single token
 * `"*:*"`). A plain string-equality check against `user.permissions` would
 * make every gate here silently always-hidden for that role, since
 * `"identity:write" !== "*:*"` — this reimplements the same
 * wildcard-aware match the backend's `AuthenticatedPrincipal` extractor
 * uses to decide 403s, so the UI's "hide it" and the server's "reject it"
 * agree on the same principal.
 */
function permissionGrants(granted: string, requested: string): boolean {
  const g = granted.split(":");
  const r = requested.split(":");
  if (g.length !== 2 || r.length !== 2) return granted === requested;
  const [gResource, gAction] = g;
  const [rResource, rAction] = r;
  return (gResource === "*" || gResource === rResource) && (gAction === "*" || gAction === rAction);
}

/**
 * Loads `GET /api/auth/me` once on mount and exposes the current principal
 * to the whole tree. Also owns route protection: any non-public path
 * rendered while `status === "unauthenticated"` bounces to
 * `/login?next=<path>` (see the effect below) — this is a client-side
 * redirect, so `AppFrame` additionally withholds the authenticated chrome
 * while `status` is `"loading"`/`"unauthenticated"` to avoid a flash of
 * console UI before the redirect lands (see `app-frame.tsx`).
 */
export function AuthProvider({ children }: { children: React.ReactNode }) {
  const pathname = usePathname() ?? "/";
  const router = useRouter();
  const [user, setUser] = React.useState<AuthUser | null>(null);
  const [status, setStatus] = React.useState<AuthStatus>("loading");
  // The locally-persisted active tenant id, as the last session check resolved
  // it (`ADM-15`): `null` until `load()` answers, because nothing may be read
  // or trusted before the session says whose it is. Written by `load()` when
  // the stored value is corrected, by `setActiveTenant` when the navbar picks
  // a tenant, and by `logout` when the key is cleared.
  const [storedActiveTenantId, setStoredActiveTenantId] = React.useState<string | null>(null);

  const load = React.useCallback(async () => {
    try {
      const me = await authClient.me();
      // ADM-15 (#483 review, D4): after the session answers, the stored tenant
      // is one of this person's, or absent — another person's left-over value
      // is replaced by their first tenant before anything renders. The storage
      // write comes first: `apiFetch` reads storage on every call, and pages
      // render the moment `status` flips.
      const stored = readActiveTenantId();
      const resolved = resolveActiveTenant(stored, me.tenants);
      if (resolved === null) clearActiveTenantId();
      else if (resolved !== stored) writeActiveTenantId(resolved);
      setStoredActiveTenantId(resolved);
      setUser(me);
      setStatus("authenticated");
    } catch {
      // D5: a failed session check (no session, or the network) changes
      // nothing in storage — the person's own choice survives their next
      // sign-in, and the next person's session check corrects it anyway.
      setUser(null);
      setStatus("unauthenticated");
    }
  }, []);

  React.useEffect(() => {
    void load();
  }, [load]);

  // Redirect-to-login for protected paths. Skipped entirely on public
  // paths so `/public/dashboard/*` and `/embed/*` stay reachable logged
  // out — see `PUBLIC_PREFIXES` above.
  React.useEffect(() => {
    if (status !== "unauthenticated") return;
    if (isPublicPath(pathname)) return;
    const next = pathname === "/" ? "" : `?next=${encodeURIComponent(pathname)}`;
    router.replace(`/login${next}`);
  }, [status, pathname, router]);

  const logout = React.useCallback(async () => {
    // ADM-15 (#483 review, D5): the key goes before the request, so it is gone
    // even when that request never returns — otherwise the next person to sign
    // in on this browser would start in this person's tenant.
    clearActiveTenantId();
    await authClient.logout();
    setStoredActiveTenantId(null);
    setUser(null);
    setStatus("unauthenticated");
    router.push("/login");
  }, [router]);

  const setActiveTenant = React.useCallback(
    (id: string) => {
      // ADM-15 (#483 review, D7): a tenant the person is not in is not a
      // choice. The switcher lists only their own, so this is a guard.
      if (!user?.tenants.some((t) => t.id === id)) return;
      writeActiveTenantId(id);
      setStoredActiveTenantId(id);
    },
    [user]
  );

  const value = React.useMemo<AuthContextValue>(
    () => ({
      user,
      status,
      hasPermission: (permission) => (user?.permissions ?? []).some((g) => permissionGrants(g, permission)),
      hasRole: (role) => user?.roles.includes(role) ?? false,
      refresh: load,
      logout,
      // One rule for "which tenant": the stored choice when it is still theirs,
      // else their first, else none — and `null` until the session answered,
      // so no page can act on a value the person's membership has not
      // confirmed (`ADM-15`, #483 review).
      activeTenantId: user ? resolveActiveTenant(storedActiveTenantId, user.tenants) : null,
      setActiveTenant,
    }),
    [user, status, load, logout, storedActiveTenantId, setActiveTenant]
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

/** Read the current principal / auth status. Must be used under `AuthProvider` (mounted in the root layout). */
export function useAuth(): AuthContextValue {
  const ctx = React.useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}
