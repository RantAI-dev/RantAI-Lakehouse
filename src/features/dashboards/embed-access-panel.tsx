"use client";

import * as React from "react";
import { ChevronRight, Globe2, Plus, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Textarea } from "@/components/ui/textarea";
import { addOrigin } from "@/lib/embed-origins";
import { dashboardService } from "@/services";
import { relativeTime } from "./share-embed";

/** What a failed call says: the server's fixed text, or a plain fallback. */
function messageOf(err: unknown, fallback: string): string {
  return err instanceof Error && err.message ? err.message : fallback;
}

/**
 * A collapsed-by-default section. Native `<details>`, styled like the run
 * rows in the connector panel; `hint` sits next to the title so a state can
 * be read without opening it.
 */
export function Disclosure({
  title, hint, children,
}: {
  readonly title: string;
  readonly hint?: string;
  readonly children: React.ReactNode;
}) {
  return (
    <details className="group rounded-md border border-border bg-background/50">
      <summary className="flex cursor-pointer list-none items-center gap-2 px-2.5 py-1.5 text-xs font-medium outline-none focus-visible:ring-[3px] focus-visible:ring-ring/50 [&::-webkit-details-marker]:hidden">
        <ChevronRight className="size-3.5 shrink-0 text-muted-foreground transition-transform group-open:rotate-90" />
        <span>{title}</span>
        {hint ? <span className="ml-auto truncate text-[11px] font-normal text-muted-foreground">{hint}</span> : null}
      </summary>
      <div className="space-y-2 border-t border-border p-2.5">{children}</div>
    </details>
  );
}

/**
 * The sites allowed to show this dashboard's embed pages (SEC-12). Both embed
 * pages (the public link's iframe and the signed one) answer with
 * `frame-ancestors` built from this list; with none listed, no site can frame
 * them. The server validates and normalises each entry, and its message is
 * shown as it comes.
 */
export function AllowedSitesEditor({
  board, saved, onSaved,
}: {
  readonly board: string;
  readonly saved: string[];
  readonly onSaved: (origins: string[]) => void;
}) {
  const [sites, setSites] = React.useState<string[]>(saved);
  const [draft, setDraft] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [error, setError] = React.useState("");
  const [done, setDone] = React.useState(false);

  // Start from what the server has whenever the dialog loads a board's state.
  React.useEffect(() => { setSites(saved); setError(""); }, [saved]);
  // "Saved" is brief.
  React.useEffect(() => {
    if (!done) return;
    const t = setTimeout(() => setDone(false), 2500);
    return () => clearTimeout(t);
  }, [done]);

  const dirty = sites.length !== saved.length || sites.some((s, i) => s !== saved[i]);

  function add() {
    if (!draft.trim()) return;
    // Same rule the server applies, so an invalid site never becomes a chip.
    const next = addOrigin(sites, draft);
    if (!next.ok) { setError(next.message); return; }
    setSites(next.origins);
    setDraft("");
    setError("");
    setDone(false);
  }

  async function save() {
    setBusy(true);
    setError("");
    try {
      const stored = await dashboardService.setEmbedOrigins(board, sites);
      setSites(stored);
      onSaved(stored);
      setDone(true);
    } catch (err) {
      setError(messageOf(err, "Allowed sites could not be saved."));
    } finally { setBusy(false); }
  }

  return (
    <div className="space-y-2 rounded-lg border border-border bg-muted/20 p-3">
      <div className="flex items-start gap-3">
        <Globe2 className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">Sites allowed to show embeds</p>
          <p className="text-xs text-muted-foreground">An embed renders only inside a page on these sites.</p>
        </div>
      </div>
      {sites.length > 0 ? (
        <ul className="flex flex-wrap gap-1.5" aria-label="Allowed sites">
          {sites.map((s) => (
            <li key={s} className="flex max-w-full items-center gap-1 rounded-md border border-border bg-background py-0.5 pr-0.5 pl-2 text-xs">
              <span className="truncate font-mono">{s}</span>
              <Button size="icon-xs" variant="ghost" aria-label={`Remove ${s}`} disabled={busy}
                onClick={() => { setSites((cur) => cur.filter((x) => x !== s)); setDone(false); }}>
                <X />
              </Button>
            </li>
          ))}
        </ul>
      ) : saved.length === 0 ? (
        <p className="text-xs text-muted-foreground">No site is allowed yet, so the embed cannot be shown anywhere.</p>
      ) : null}
      <div className="flex items-center gap-2">
        <Input value={draft} placeholder="https://app.example.com" aria-label="Site to allow" disabled={busy}
          title="The viewer's browser enforces this list; it does not replace the token. http://localhost is accepted for development."
          onChange={(e) => { setDraft(e.target.value); setError(""); }}
          onKeyDown={(e) => { if (e.key === "Enter") { e.preventDefault(); add(); } }} />
        <Button size="sm" variant="outline" disabled={busy || !draft.trim()} onClick={add}><Plus /> Add</Button>
      </div>
      <p className="text-[11px] text-muted-foreground">https://host or https://host:port, no wildcards.</p>
      {error ? <p role="alert" className="text-xs text-destructive">{error}</p> : null}
      {done && !dirty ? <p role="status" className="text-xs text-emerald-600 dark:text-emerald-400">Saved</p> : null}
      {dirty ? <Button size="sm" disabled={busy} onClick={() => void save()}>Save sites</Button> : null}
    </div>
  );
}

/**
 * Withdraw signed embed tokens (SEC-12): every one of the dashboard's tokens
 * at once, or one token by pasting it. A withdrawal is honoured on the next
 * request the embed makes, and in any case within a minute.
 */
export function WithdrawTokens({
  board, revokedBefore, onRevokedAll,
}: {
  readonly board: string;
  readonly revokedBefore: number | null;
  readonly onRevokedAll: (revokedBefore: number) => void;
}) {
  const [confirming, setConfirming] = React.useState(false);
  const [busy, setBusy] = React.useState(false);
  const [allMessage, setAllMessage] = React.useState<{ ok: boolean; text: string } | null>(null);
  const [token, setToken] = React.useState("");
  const [oneMessage, setOneMessage] = React.useState<{ ok: boolean; text: string } | null>(null);

  async function revokeAll() {
    setBusy(true);
    setAllMessage(null);
    try {
      const at = await dashboardService.revokeAllEmbedTokens(board);
      onRevokedAll(at);
      setAllMessage({ ok: true, text: "All embed tokens issued so far are withdrawn. They stop working within a minute; tokens your server signs from now on work." });
    } catch (err) {
      setAllMessage({ ok: false, text: messageOf(err, "Embed tokens could not be withdrawn.") });
    } finally { setBusy(false); setConfirming(false); }
  }

  async function revokeOne() {
    setBusy(true);
    setOneMessage(null);
    try {
      const jti = await dashboardService.revokeEmbedToken(board, token.trim());
      setOneMessage({ ok: true, text: `Token withdrawn (id ${jti}). Other tokens for this dashboard still work.` });
      setToken("");
    } catch (err) {
      setOneMessage({ ok: false, text: messageOf(err, "The token could not be withdrawn.") });
    } finally { setBusy(false); }
  }

  const hint = revokedBefore ? `last withdrawn ${relativeTime(revokedBefore)}` : undefined;

  return (
    <Disclosure title="Withdraw tokens" hint={hint}>
      <div className="space-y-1.5">
        {confirming ? (
          <div role="alertdialog" aria-label="Confirm withdrawing all embed tokens" className="space-y-2 rounded-md border border-destructive/40 bg-destructive/5 p-2.5">
            <p className="text-xs text-foreground">
              Every embed token already issued for this dashboard stops working, within a minute. Embeds that use one show an error
              until your server signs a new token. This cannot be undone.
            </p>
            <div className="flex items-center gap-2">
              <Button size="sm" variant="destructive" disabled={busy} onClick={() => void revokeAll()}>Withdraw all</Button>
              <Button size="sm" variant="ghost" disabled={busy} onClick={() => setConfirming(false)}>Cancel</Button>
            </div>
          </div>
        ) : (
          <Button size="sm" variant="outline" disabled={busy} onClick={() => { setAllMessage(null); setConfirming(true); }}>Withdraw all embed tokens</Button>
        )}
        {allMessage ? (
          <p role={allMessage.ok ? "status" : "alert"} className={allMessage.ok ? "text-xs text-emerald-600 dark:text-emerald-400" : "text-xs text-destructive"}>{allMessage.text}</p>
        ) : null}
        {revokedBefore ? <p className="text-[11px] text-muted-foreground">Tokens issued before {new Date(revokedBefore * 1000).toLocaleString()} are withdrawn.</p> : null}
      </div>

      <div className="space-y-1.5">
        <Textarea value={token} onChange={(e) => setToken(e.target.value)} placeholder="Paste a token to withdraw it" aria-label="Token to withdraw"
          className="min-h-12 font-mono text-[11px]" disabled={busy} />
        <p className="text-[11px] text-muted-foreground">The token needs a jti; one without can only be withdrawn with all the others.</p>
        <Button size="sm" variant="outline" disabled={busy || !token.trim()} onClick={() => void revokeOne()}>Withdraw this token</Button>
        {oneMessage ? (
          <p role={oneMessage.ok ? "status" : "alert"} className={oneMessage.ok ? "text-xs text-emerald-600 dark:text-emerald-400" : "text-xs text-destructive"}>{oneMessage.text}</p>
        ) : null}
      </div>
    </Disclosure>
  );
}
