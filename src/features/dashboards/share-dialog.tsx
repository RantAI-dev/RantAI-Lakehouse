"use client";

import * as React from "react";
import { Check, Code2, Copy, Globe, KeyRound, Link2, Share2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { apiFetch } from "@/services/http";

async function putBoard(body: Record<string, unknown>): Promise<Record<string, unknown>> {
  const res = await apiFetch("/api/dashboard/boards", {
    method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
  });
  return (await res.json()) as Record<string, unknown>;
}

async function fetchEmbedInfo(board: string): Promise<{ enabled: boolean; sampleToken: string }> {
  try {
    const res = await apiFetch(`/api/dashboard/embed-info?board=${encodeURIComponent(board)}`, { cache: "no-store" });
    const json = await res.json();
    return { enabled: Boolean(json?.enabled), sampleToken: json?.sampleToken ?? "" };
  } catch {
    return { enabled: false, sampleToken: "" };
  }
}

/** Public link, iframe embed and signed embedding for one user dashboard. */
export function ShareDialog({
  board, dashName, open, onOpenChange,
}: {
  readonly board: string;
  readonly dashName: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  const [shareToken, setShareToken] = React.useState("");
  const [embedEnabled, setEmbedEnabled] = React.useState(false);
  const [sampleToken, setSampleToken] = React.useState("");
  const [busy, setBusy] = React.useState(false);
  const [copied, setCopied] = React.useState<string | false>(false);

  // Read the board's current sharing state each time the dialog opens.
  React.useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setCopied(false);
    void (async () => {
      try {
        const res = await apiFetch("/api/dashboard/boards", { cache: "no-store" });
        const json = await res.json();
        const b = (json?.boards ?? []).find((x: { id: string; publicToken?: string }) => x.id === board);
        if (!cancelled) setShareToken(b?.publicToken ?? "");
      } catch {
        if (!cancelled) setShareToken("");
      }
      const info = await fetchEmbedInfo(board);
      if (!cancelled) {
        setEmbedEnabled(info.enabled);
        setSampleToken(info.sampleToken);
      }
    })();
    return () => { cancelled = true; };
  }, [open, board]);

  async function setPublic(enable: boolean) {
    setBusy(true);
    try {
      const json = await putBoard({ id: board, public: enable });
      setShareToken(typeof json.publicToken === "string" ? json.publicToken : "");
      setCopied(false);
    } finally { setBusy(false); }
  }

  async function setEmbed(enable: boolean) {
    setBusy(true);
    try {
      await putBoard({ id: board, embed: enable });
      setEmbedEnabled(enable);
      setSampleToken((await fetchEmbedInfo(board)).sampleToken);
    } finally { setBusy(false); }
  }

  async function copyText(text: string, key: string) {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      setTimeout(() => setCopied(false), 1800);
    } catch { /* ignore */ }
  }

  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const shareUrl = shareToken ? `${origin}/public/dashboard/${shareToken}` : "";
  const embedDashUrl = shareToken ? `${origin}/embed/dashboard/${shareToken}` : "";
  const embedIframe = shareToken
    ? `<iframe src="${embedDashUrl}" width="100%" height="600" frameborder="0" style="border:1px solid #e5e7eb;border-radius:12px" title="Rantai Lake dashboard"></iframe>`
    : "";
  const signedPreviewUrl = sampleToken ? `${origin}/embed/signed/${sampleToken}` : "";
  const signSnippet = [
    `// Node — sign a per-viewer embed token (KEEP THE SECRET SERVER-SIDE)`,
    `import jwt from "jsonwebtoken";`,
    `const token = jwt.sign({`,
    `  resource: { dashboard: "${board}" },`,
    `  params: { /* locked filters, e.g. */ kawasan: "Jakarta Pusat" },`,
    `  exp: Math.floor(Date.now()/1000) + 60*10,`,
    `}, EMBED_SECRET);`,
    `const url = "${origin}/embed/signed/" + token;`,
  ].join("\n");

  const copyLabel = (k: string, label: string) => (
    <>
      {copied === k ? <Check className="size-4 text-emerald-500" /> : <Copy className="size-4" />}
      {copied === k ? "Copied" : label}
    </>
  );

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-h-[85vh] overflow-y-auto overflow-x-hidden sm:max-w-md">
        <DialogHeader>
          <DialogTitle className="flex items-center gap-2"><Share2 className="size-4" /> Share “{dashName}”</DialogTitle>
        </DialogHeader>
        <div className="space-y-3">
          <div className="flex items-start gap-3 rounded-lg border border-border bg-muted/30 p-3">
            <Globe className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
            <div className="min-w-0 flex-1">
              <p className="text-sm font-medium">Public link</p>
              <p className="text-xs text-muted-foreground">Anyone with the link can view this dashboard read-only — no sign-in. Charts stay live; they cannot edit anything.</p>
            </div>
            <Switch checked={Boolean(shareToken)} disabled={busy} onCheckedChange={(v) => void setPublic(v)} aria-label="Public link" />
          </div>

          {shareToken ? (
            <div className="space-y-3">
              <div className="space-y-1.5">
                <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Public link</p>
                <div className="flex items-center gap-2">
                  <div className="flex min-w-0 flex-1 items-center gap-2 rounded-md border border-border bg-background px-2.5 py-2">
                    <Link2 className="size-3.5 shrink-0 text-muted-foreground" />
                    <span className="truncate text-xs text-foreground">{shareUrl}</span>
                  </div>
                  <Button size="sm" variant="outline" onClick={() => void copyText(shareUrl, "link")}>
                    {copyLabel("link", "Copy")}
                  </Button>
                </div>
                <a href={shareUrl} target="_blank" rel="noopener noreferrer" className="inline-block text-xs font-medium text-primary hover:underline">Open preview ↗</a>
              </div>

              <div className="space-y-1.5">
                <p className="flex items-center gap-1.5 text-[11px] font-medium uppercase tracking-wide text-muted-foreground"><Code2 className="size-3.5" /> Embed in a website</p>
                <div className="rounded-md border border-border bg-muted/30 p-2">
                  <code className="block max-h-20 overflow-auto whitespace-pre-wrap break-all font-mono text-[11px] leading-relaxed text-foreground">{embedIframe}</code>
                </div>
                <div className="flex items-center gap-2">
                  <Button size="sm" variant="outline" onClick={() => void copyText(embedIframe, "embed")}>
                    {copyLabel("embed", "Copy iframe")}
                  </Button>
                  <a href={embedDashUrl} target="_blank" rel="noopener noreferrer" className="text-xs font-medium text-primary hover:underline">Preview embed ↗</a>
                </div>
                <p className="text-[11px] text-muted-foreground">One chart only? Append <code className="rounded bg-muted px-1 font-mono">?chart=&lt;id&gt;</code> to the embed URL.</p>
              </div>

              <div className="flex items-center justify-between gap-2 border-t border-border pt-2">
                <button onClick={() => void setPublic(false)} disabled={busy} className="text-xs text-destructive hover:underline disabled:opacity-50">Revoke link</button>
              </div>
              <p className="rounded-md bg-amber-500/5 px-2.5 py-1.5 text-[11px] text-amber-600 dark:text-amber-400">
                Works wherever the server is reachable. For the public internet, expose the server (e.g. Cloudflare Tunnel / reverse proxy).
              </p>
            </div>
          ) : (
            <p className="text-xs text-muted-foreground">Turn on the switch to generate a shareable link.</p>
          )}

          <div className="space-y-2 rounded-lg border border-border bg-muted/20 p-3">
            <div className="flex items-start gap-3">
              <KeyRound className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">Signed embedding</p>
                <p className="text-xs text-muted-foreground">Your app signs a JWT with the secret to embed this dashboard with <span className="font-medium text-foreground">locked filters per viewer</span>. Revocable instantly.</p>
              </div>
              <Switch checked={embedEnabled} disabled={busy} onCheckedChange={(v) => void setEmbed(v)} aria-label="Signed embedding" />
            </div>

            {embedEnabled ? (
              <div className="space-y-2 pt-1">
                {/* The signing secret itself is never sent to the browser — see
                    /api/dashboard/embed-info. */}
                <p className="text-[11px] text-muted-foreground">The signing secret is kept server-side and is never exposed to the console. Set <code className="rounded bg-muted px-1 font-mono">EMBED_SECRET</code> in your server environment to the same value the backend signs with.</p>
                <div className="space-y-1">
                  <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Sign a token (server-side)</p>
                  <div className="rounded-md border border-border bg-background p-2">
                    <code className="block max-h-40 overflow-auto whitespace-pre font-mono text-[11px] leading-relaxed text-foreground">{signSnippet}</code>
                  </div>
                  <Button size="sm" variant="outline" onClick={() => void copyText(signSnippet, "snippet")}>
                    {copyLabel("snippet", "Copy code")}
                  </Button>
                </div>
                {signedPreviewUrl ? (
                  <a href={signedPreviewUrl} target="_blank" rel="noopener noreferrer" className="inline-block text-xs font-medium text-primary hover:underline">Preview signed embed (sample token, 1h) ↗</a>
                ) : null}
              </div>
            ) : null}
          </div>
        </div>
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Close</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
