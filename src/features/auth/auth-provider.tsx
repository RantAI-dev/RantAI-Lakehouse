"use client";

import * as React from "react";
import { usePathname, useRouter } from "next/navigation";
import * as authClient from "@/services/clients/auth";
import type { AuthUser } from "@/services/clients/auth";

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
 */
const PUBLIC_PREFIXES = ["/login", "/public", "/embed"];

export function isPublicPath(pathname: string): boolean {
  return PUBLIC_PREFIXES.some((p) => pathname === p || pathname.startsWith(`${p}/`));
}

export type AuthStatus = "loading" | "authenticated" | "unauthenticated";

type AuthContextValue = {
  user: AuthUser | null;
  status: AuthStatus;
  /** True once `roles`/`permissions` are known to be authoritative (i.e. not mid-initial-load). */
  hasPermission: (permission: string) => boolean;
  hasRole: (role: string) => boolean;
  refresh: () => Promise<void>;
  logout: () => Promise<void>;
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

  const load = React.useCallback(async () => {
    try {
      const me = await authClient.me();
      setUser(me);
      setStatus("authenticated");
    } catch {
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
    await authClient.logout();
    setUser(null);
    setStatus("unauthenticated");
    router.push("/login");
  }, [router]);

  const value = React.useMemo<AuthContextValue>(
    () => ({
      user,
      status,
      hasPermission: (permission) => (user?.permissions ?? []).some((g) => permissionGrants(g, permission)),
      hasRole: (role) => user?.roles.includes(role) ?? false,
      refresh: load,
      logout,
    }),
    [user, status, load, logout]
  );

  return <AuthContext.Provider value={value}>{children}</AuthContext.Provider>;
}

/** Read the current principal / auth status. Must be used under `AuthProvider` (mounted in the root layout). */
export function useAuth(): AuthContextValue {
  const ctx = React.useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}
