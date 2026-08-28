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
 */

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
  const res = await fetch(input, init);
  if (res.status === 401 && !resolveUrl(input).startsWith("/api/auth/")) {
    redirectToLogin();
  }
  return res;
}
