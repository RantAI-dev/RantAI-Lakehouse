"use client";

import * as React from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";

/**
 * Opening a public link (`BI-19` part A, plan D3 and D4). The public page and
 * the public embed share this: one request that sends the ticket, one that
 * trades a password for it, and one "This link does not work" page for an
 * unknown, expired, ended or switched-off link, so nobody can tell which.
 *
 * Deliberately NOT `apiFetch`: a visitor has no account, and its 401 ->
 * /login redirect would be wrong for them.
 */

/** Wire name of the header the API reads. */
export const TICKET_HEADER = "X-Public-Link-Ticket";

const KEY_PREFIX = "public-link-ticket:";

/** The ticket kept for this browser session; `null` without storage. */
export function readTicket(token: string): string | null {
  try {
    return window.sessionStorage.getItem(KEY_PREFIX + token);
  } catch {
    return null;
  }
}

function keepTicket(token: string, ticket: string | null) {
  try {
    if (ticket) window.sessionStorage.setItem(KEY_PREFIX + token, ticket);
    else window.sessionStorage.removeItem(KEY_PREFIX + token);
  } catch {
    // Without storage the password is asked again on the next load. Nothing breaks.
  }
}

export type OpenResult<T> =
  | { kind: "ok"; data: T }
  | { kind: "password" }
  | { kind: "gone" }
  | { kind: "error" };

/** `ticket` is what unlock just returned; otherwise the one kept for the session. */
export async function openPublic<T>(token: string, ticket?: string): Promise<OpenResult<T>> {
  const sent = ticket ?? readTicket(token);
  const res = await fetch(`/api/public/dashboard/${encodeURIComponent(token)}`, {
    cache: "no-store",
    headers: sent ? { [TICKET_HEADER]: sent } : undefined,
  });
  if (res.status === 404) return { kind: "gone" };
  if (res.status === 401) {
    const body = await res.json().catch(() => null);
    if (body?.error === "password_required") {
      keepTicket(token, null);
      return { kind: "password" };
    }
    return { kind: "gone" };
  }
  if (!res.ok) return { kind: "error" };
  return { kind: "ok", data: (await res.json()) as T };
}

/** Trade the password for a ticket and keep it for the session. */
export async function unlockPublic(token: string, password: string): Promise<"ok" | "wrong" | "gone" | "error"> {
  const res = await fetch(`/api/public/dashboard/${encodeURIComponent(token)}/unlock`, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify({ password }),
  });
  if (res.status === 404) return "gone";
  if (res.status === 401) return "wrong";
  if (!res.ok) return "error";
  const body = await res.json().catch(() => null);
  if (typeof body?.ticket !== "string") return "error";
  keepTicket(token, body.ticket);
  return "ok";
}

/** The one page for a link that does not work. */
export function LinkGone() {
  return (
    <div className="grid min-h-[60vh] place-content-center px-4 text-center">
      <p className="text-lg font-semibold">This link does not work</p>
      <p className="mt-1 text-sm text-muted-foreground">It may have ended, been switched off, or never existed.</p>
    </div>
  );
}

/**
 * The password page. A wrong password and a locked link read the same here as
 * they do on the server: "That password did not work."
 */
export function PasswordGate({ token, onOpen }: { readonly token: string; readonly onOpen: () => void }) {
  const [password, setPassword] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true);
    setMessage(null);
    try {
      const result = await unlockPublic(token, password);
      if (result === "ok") { onOpen(); return; }
      setMessage(result === "error" ? "Could not check the password. Please try again later." : "That password did not work.");
      if (result === "gone") onOpen();
    } catch {
      setMessage("Could not check the password. Please try again later.");
    } finally { setBusy(false); }
  }

  return (
    <div className="grid min-h-[60vh] place-content-center px-4">
      <form onSubmit={(e) => void submit(e)} className="flex w-72 flex-col gap-3 text-center">
        <p className="text-lg font-semibold">This dashboard needs a password</p>
        <Input type="password" aria-label="Password" autoComplete="off" autoFocus value={password}
          onChange={(e) => setPassword(e.target.value)} />
        <Button type="submit" disabled={busy || password.length === 0}>Open</Button>
        {message ? <p role="alert" className="text-xs text-destructive">{message}</p> : null}
      </form>
    </div>
  );
}
