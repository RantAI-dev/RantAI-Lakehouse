/**
 * Thin wrapper around the global `fetch`, shared by every service client
 * adapter in `src/services/clients/*`. It is a drop-in replacement for
 * `fetch` — same signature, same `Response` returned unchanged — so every
 * call site keeps its existing `res.ok` / `res.json()` / error-message
 * handling verbatim. The one thing it adds: when a same-origin `/api/*`
 * call comes back `401`, it redirects the browser to
 * `/login?next=<current path>`.
 *
 * This is the single choke point for "a session expires mid-session must
 * not show 55 different broken states" (Task 3.3) — every domain adapter
 * (`assets`, `agents`, `governance`, `ops`, ...) already funnels its raw
 * `fetch` calls through here, so none of the feature pages that consume
 * them via `useService` need their own 401 handling.
 *
 * Deliberately NOT triggered for `/api/auth/*` calls (login / logout / me /
 * change-password): a 401 from `/api/auth/login` is an ordinary "wrong
 * credentials" response, and a 401 from `/api/auth/me` is the expected
 * shape of "not logged in yet" that `AuthProvider` itself probes for and
 * reacts to. Excluding that prefix is also what prevents a redirect loop
 * out of the login page itself.
 *
 * It also attaches `X-Tenant` when an active tenant is selected: the
 * tenant picker (`TenantSwitcher`) persists the
 * chosen tenant id to `localStorage` as a per-browser convenience — nothing
 * server-trusted hangs off it, the server still derives the authoritative
 * tenant scope from the session — so a request made before any tenant is
 * chosen, or in an environment where `localStorage` throws (private
 * browsing, SSR), goes out with no `X-Tenant` header at all rather than an
 * empty one.
 */

/**
 * The one place the active-tenant key lives. The tenant switcher WRITES it
 * (through `writeActiveTenantId`) and `apiFetch` READS it; if those two
 * sides ever named different keys, `X-Tenant` would silently stop being
 * sent and every request would fall back to the principal's first tenant.
 * Exported from here, next to its reader, rather than re-declared by the
 * writer.
 */
export const ACTIVE_TENANT_STORAGE_KEY = "lh_active_tenant";

/**
 * The locally-persisted active tenant id, or `null` when unset, on the
 * server (SSR), or when storage itself throws (private browsing) — in every
 * one of those cases the request simply goes out without `X-Tenant`.
 */
export function readActiveTenantId(): string | null {
  if (typeof window === "undefined") return null;
  try {
    return window.localStorage.getItem(ACTIVE_TENANT_STORAGE_KEY);
  } catch {
    return null;
  }
}

/**
 * Persist the active tenant id. Best-effort: when storage is unavailable
 * the switch still takes effect in the current tab through React state;
 * only persistence across reloads is lost. Never an authorization input —
 * membership is resolved server-side from the session.
 */
export function writeActiveTenantId(id: string): void {
  if (typeof window === "undefined") return;
  try {
    window.localStorage.setItem(ACTIVE_TENANT_STORAGE_KEY, id);
  } catch {
    // See the doc comment: a lost write only costs persistence.
  }
}

let redirecting = false;

function isLoginPath(pathname: string): boolean {
  return pathname === "/login" || pathname.startsWith("/login/");
}

function redirectToLogin(): void {
  if (typeof window === "undefined") return;
  if (redirecting) return;
  if (isLoginPath(window.location.pathname)) return;
  redirecting = true;
  const next = window.location.pathname + window.location.search;
  window.location.assign(`/login?next=${encodeURIComponent(next)}`);
}

function resolveUrl(input: RequestInfo | URL): string {
  if (typeof input === "string") return input;
  if (input instanceof URL) return input.toString();
  return input.url;
}

/** Reset the internal "already redirecting" latch. Test-only. */
export function __resetApiFetchRedirectStateForTests(): void {
  redirecting = false;
}

export async function apiFetch(input: RequestInfo | URL, init?: RequestInit): Promise<Response> {
  const headers = new Headers(init?.headers);
  const activeTenantId = readActiveTenantId();
  if (activeTenantId) {
    headers.set("X-Tenant", activeTenantId);
  }
  const res = await fetch(input, { ...init, headers });
  if (res.status === 401 && !resolveUrl(input).startsWith("/api/auth/")) {
    redirectToLogin();
  }
  return res;
}
