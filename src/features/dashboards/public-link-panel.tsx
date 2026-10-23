"use client";

import * as React from "react";
import { Check, Code2, Copy, Globe, Link2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { copyText as copyToClipboard } from "@/lib/copy-text";
import { readableErrorMessage } from "@/lib/notify-message";
import { dashboardService } from "@/services";
import type { PublicLink, PublicLinkState } from "@/services/contracts/dashboards";

/** `BI-19` part A, decision A1: 30 days is chosen for you; a year is the most. */
export const EXPIRY_CHOICES = ["1", "7", "30", "90", "date"] as const;
export type ExpiryChoice = (typeof EXPIRY_CHOICES)[number];
export const DEFAULT_EXPIRY: ExpiryChoice = "30";
const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/**
 * The expiry a choice means, as an ISO time, or the sentence that says why not.
 * A date is the end of that day in the browser's zone.
 */
export function expiryFor(
  choice: ExpiryChoice,
  date: string,
  now: Date = new Date(),
): { at: string } | { error: string } {
  const refused = { error: "Choose a date between 1 hour and 365 days from now." };
  let at: Date;
  if (choice === "date") {
    if (!/^\d{4}-\d{2}-\d{2}$/.test(date)) return refused;
    at = new Date(`${date}T23:59:59`);
  } else {
    at = new Date(now.getTime() + Number(choice) * DAY);
  }
  const ms = at.getTime() - now.getTime();
  if (Number.isNaN(ms) || ms < HOUR || ms > 365 * DAY) return refused;
  return { at: at.toISOString() };
}

/** The sentence a failed call carries, as the page shows it. */
function sentence(e: unknown): string {
  return readableErrorMessage(e instanceof Error ? e.message : "The change was not made.");
}

function endsOn(link: PublicLink): string {
  return new Date(link.expiresAt).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

/** Expiry choice: 1, 7, 30 or 90 days, or a date. */
function ExpiryPicker({
  choice, date, onChoice, onDate,
}: {
  readonly choice: ExpiryChoice;
  readonly date: string;
  readonly onChoice: (c: ExpiryChoice) => void;
  readonly onDate: (d: string) => void;
}) {
  return (
    <div className="flex flex-wrap items-center gap-2">
      <select
        aria-label="Link expires in"
        value={choice}
        onChange={(e) => onChoice(e.target.value as ExpiryChoice)}
        className="h-8 rounded-md border border-border bg-background px-2 text-xs"
      >
        <option value="1">1 day</option>
        <option value="7">7 days</option>
        <option value="30">30 days</option>
        <option value="90">90 days</option>
        <option value="date">A date…</option>
      </select>
      {choice === "date" ? (
        <Input type="date" aria-label="Link expires on" value={date} onChange={(e) => onDate(e.target.value)} className="h-8 w-40 text-xs" />
      ) : null}
    </div>
  );
}

/**
 * The public link of one dashboard (`BI-19` part A): make it with an expiry and
 * an optional password, see when it ends, change the expiry or password, make a
 * new one, switch it off. The password is typed here and never shown back; the
 * server keeps only a hash.
 */
export function PublicLinkPanel({ board }: { readonly board: string }) {
  const [state, setState] = React.useState<PublicLinkState | null>(null);
  const [failed, setFailed] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState(false);
  const [choice, setChoice] = React.useState<ExpiryChoice>(DEFAULT_EXPIRY);
  const [date, setDate] = React.useState("");
  const [password, setPassword] = React.useState("");
  const [message, setMessage] = React.useState<string | null>(null);
  const [copied, setCopied] = React.useState<string | false>(false);
  const [copyFailed, setCopyFailed] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    setState(null);
    setFailed(null);
    dashboardService
      .getPublicLink(board)
      .then((s) => { if (!cancelled) setState(s); })
      .catch((e) => { if (!cancelled) setFailed(sentence(e)); });
    return () => { cancelled = true; };
  }, [board]);

  const link = state?.link ?? null;
  const live = link !== null && !link.expired;
  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const shareUrl = live ? `${origin}/public/dashboard/${link.token}` : "";
  const embedUrl = live ? `${origin}/embed/dashboard/${link.token}` : "";
  const iframe = live
    ? `<iframe src="${embedUrl}" width="100%" height="600" frameborder="0" style="border:1px solid #e5e7eb;border-radius:12px" title="Rantai Lake dashboard"></iframe>`
    : "";

  async function run(work: () => Promise<PublicLink | null>) {
    setBusy(true);
    setMessage(null);
    try {
      const saved = await work();
      setState((cur) => (cur ? { ...cur, link: saved } : cur));
      setPassword("");
    } catch (e) {
      setMessage(sentence(e));
    } finally { setBusy(false); }
  }

  function save(extra: { password?: string | null; newLink?: boolean }) {
    const when = expiryFor(choice, date);
    if ("error" in when) { setMessage(when.error); return; }
    void run(() => dashboardService.savePublicLink({ board, expiresAt: when.at, ...extra }));
  }

  function keepExpiry(extra: { password?: string | null }) {
    // Changing only the password keeps the end the link already has.
    if (!link) return;
    void run(() => dashboardService.savePublicLink({ board, expiresAt: link.expiresAt, ...extra }));
  }

  async function copy(text: string, key: string) {
    const ok = await copyToClipboard(text);
    setCopyFailed(!ok);
    setCopied(ok ? key : false);
    setTimeout(() => { setCopied(false); setCopyFailed(false); }, ok ? 1800 : 4000);
  }

  const copyLabel = (k: string, label: string) => (
    <>
      {copied === k ? <Check className="size-4 text-emerald-500" /> : <Copy className="size-4" />}
      {copied === k ? "Copied" : label}
    </>
  );

  return (
    <div className="space-y-3 rounded-lg border border-border bg-muted/30 p-3">
      <div className="flex items-start gap-3">
        <Globe className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium">Public link</p>
          <p className="text-xs text-muted-foreground">Anyone with the link can view this dashboard read-only, no sign-in, until it ends.</p>
        </div>
      </div>

      {failed ? <p role="alert" className="text-xs text-destructive">{failed}</p> : null}
      {!state && !failed ? <p className="text-xs text-muted-foreground">Loading…</p> : null}
      {state && !state.enabled ? (
        <p role="status" className="text-xs text-amber-600 dark:text-amber-400">
          An administrator has switched public links off (Admin &gt; Settings). No link works and none can be made.
        </p>
      ) : null}

      {state && link && link.expired ? (
        <p role="status" className="text-xs text-amber-600 dark:text-amber-400">
          The link ended on {endsOn(link)}. It no longer works. Make a new one below.
        </p>
      ) : null}

      {state && live ? (
        <div className="space-y-3">
          <div className="space-y-1.5">
            <div className="flex items-center gap-2">
              <div className="flex min-w-0 flex-1 items-center gap-2 rounded-md border border-border bg-background px-2.5 py-2">
                <Link2 className="size-3.5 shrink-0 text-muted-foreground" />
                <span className="truncate text-xs text-foreground">{shareUrl}</span>
              </div>
              <Button size="sm" variant="outline" onClick={() => void copy(shareUrl, "link")}>{copyLabel("link", "Copy")}</Button>
            </div>
            <p className="text-xs text-foreground">
              Ends on {endsOn(link)}. {link.hasPassword ? "A password is needed." : "No password."}
            </p>
            {!state.enabled ? <p className="text-[11px] text-muted-foreground">It does not work while links are switched off.</p> : null}
            <a href={shareUrl} target="_blank" rel="noopener noreferrer" className="inline-block text-xs font-medium text-primary hover:underline">Open preview ↗</a>
          </div>

          <div className="space-y-1.5">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Change when it ends</p>
            <ExpiryPicker choice={choice} date={date} onChoice={setChoice} onDate={setDate} />
            <Button size="sm" variant="outline" disabled={busy || !state.enabled} onClick={() => save({})}>Change expiry</Button>
          </div>

          <div className="space-y-1.5">
            <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">{link.hasPassword ? "Replace the password" : "Add a password"}</p>
            <div className="flex items-center gap-2">
              <Input type="password" autoComplete="new-password" aria-label="New password" value={password}
                onChange={(e) => setPassword(e.target.value)} placeholder="8 to 128 characters" className="h-8 text-xs" />
              <Button size="sm" variant="outline" disabled={busy || !state.enabled || password.length === 0}
                onClick={() => keepExpiry({ password })}>{link.hasPassword ? "Replace" : "Set"}</Button>
            </div>
            <p className="text-[11px] text-muted-foreground">Visitors type it once per browser session. You cannot read it back, only replace it.</p>
            {link.hasPassword ? (
              <button className="text-xs text-destructive hover:underline disabled:opacity-50" disabled={busy}
                onClick={() => keepExpiry({ password: null })}>Remove the password</button>
            ) : null}
          </div>

          <div className="space-y-1.5">
            <p className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground"><Code2 className="size-3.5" /> Embed in a website</p>
            <div className="rounded-md border border-border bg-muted/30 p-2">
              <code className="block max-h-20 overflow-auto whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-foreground">{iframe}</code>
            </div>
            <Button size="sm" variant="outline" onClick={() => void copy(iframe, "embed")}>{copyLabel("embed", "Copy iframe")}</Button>
          </div>

          <div className="flex items-center justify-between gap-2 border-t border-border pt-2">
            <button className="text-xs text-destructive hover:underline disabled:opacity-50" disabled={busy}
              onClick={() => void run(async () => { await dashboardService.endPublicLink(board); return null; })}>Switch off now</button>
            <button className="text-xs text-muted-foreground hover:underline disabled:opacity-50" disabled={busy || !state.enabled}
              onClick={() => save({ newLink: true })}>Make a new link (ends this one)</button>
          </div>
          <p className="text-[11px] text-muted-foreground">
            Links made before this version was installed end 30 days after the upgrade. Check the date above.
          </p>
        </div>
      ) : null}

      {state && state.enabled && !live ? (
        <div className="space-y-2">
          <p className="text-xs text-muted-foreground">No link yet. Choose when it ends{link ? "" : ", and add a password if you want one"}.</p>
          <ExpiryPicker choice={choice} date={date} onChoice={setChoice} onDate={setDate} />
          <Input type="password" autoComplete="new-password" aria-label="Password (optional)" value={password}
            onChange={(e) => setPassword(e.target.value)} placeholder="Password (optional)" className="h-8 text-xs" />
          <Button size="sm" disabled={busy} onClick={() => save(password ? { password } : {})}>Make a public link</Button>
        </div>
      ) : null}

      {message ? <p role="alert" className="text-xs text-destructive">{message}</p> : null}
      {copyFailed ? <p role="alert" className="text-xs text-destructive">Could not copy — select the text and copy it</p> : null}
    </div>
  );
}
