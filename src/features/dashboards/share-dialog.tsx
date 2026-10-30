"use client";

import * as React from "react";
import { Check, Copy, KeyRound, Share2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog, DialogClose, DialogContent, DialogFooter, DialogHeader, DialogTitle,
} from "@/components/ui/dialog";
import { Switch } from "@/components/ui/switch";
import { apiFetch } from "@/services/http";
// BI-19 part A review fix (SHOULD-FIX) R4: features import `@/services` only.
import { dashboardService, type EmbedInfo } from "@/services";
import { copyText as copyToClipboard } from "@/lib/copy-text";
import { PublicLinkPanel } from "./public-link-panel";
import { PeoplePanel } from "./people-panel";
import { AllowedSitesEditor, WithdrawTokens } from "./embed-access-panel";
import { Input } from "@/components/ui/input";
import { tokenRules } from "./share-embed";
import { EmbedSettingsPanel } from "./embed-settings-panel";

async function putBoard(body: Record<string, unknown>): Promise<Record<string, unknown>> {
  const res = await apiFetch("/api/dashboard/boards", {
    method: "PUT", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body),
  });
  return (await res.json()) as Record<string, unknown>;
}

// SEC-12: until the first answer arrives the dialog offers no sample token or
// withdrawal it cannot back. Whether "not configured" is shown is decided by
// `infoState`, not by these placeholder values.
const EMPTY_INFO: EmbedInfo = {
  enabled: false, supported: false, maxLifetimeSeconds: 86_400, revokedBefore: null, allowedOrigins: [],
};

/** `null` when the state could not be loaded (not the same as "not configured"). */
async function fetchEmbedInfo(board: string): Promise<EmbedInfo | null> {
  try {
    return await dashboardService.getEmbedInfo(board);
  } catch {
    return null;
  }
}

/** Public link, iframe embed and signed embedding for one user dashboard. */
export function ShareDialog({
  board, dashName, open, onOpenChange, canWrite = true,
}: {
  readonly board: string;
  readonly dashName: string;
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
  /** Whether the caller may save filter modes; the server checks it too (`dashboard:write`). */
  readonly canWrite?: boolean;
}) {
  const [info, setInfo] = React.useState<EmbedInfo>(EMPTY_INFO);
  const [infoState, setInfoState] = React.useState<"loading" | "ready" | "failed">("loading");
  const embedEnabled = info.enabled;
  const sampleToken = info.sampleToken ?? "";
  const [busy, setBusy] = React.useState(false);
  const [copied, setCopied] = React.useState<string | false>(false);
  const [copyFailed, setCopyFailed] = React.useState(false);

  // Read the board's current sharing state each time the dialog opens.
  React.useEffect(() => {
    if (!open) return;
    let cancelled = false;
    setCopied(false);
    setInfoState("loading");
    void (async () => {
      const loaded = await fetchEmbedInfo(board);
      if (cancelled) return;
      setInfo(loaded ?? EMPTY_INFO);
      setInfoState(loaded ? "ready" : "failed");
    })();
    return () => { cancelled = true; };
  }, [open, board]);

  async function setEmbed(enable: boolean) {
    setBusy(true);
    try {
      await putBoard({ id: board, embed: enable });
      const loaded = await fetchEmbedInfo(board);
      setInfo(loaded ?? EMPTY_INFO);
      setInfoState(loaded ? "ready" : "failed");
    } finally { setBusy(false); }
  }

  async function copyText(text: string, key: string) {
    if (!text) return;
    // `copyToClipboard` works on an http origin too; "Copied" only on success.
    const ok = await copyToClipboard(text);
    setCopyFailed(!ok);
    setCopied(ok ? key : false);
    setTimeout(() => { setCopied(false); setCopyFailed(false); }, ok ? 1800 : 4000);
  }

  const origin = typeof window !== "undefined" ? window.location.origin : "";
  const signedPreviewUrl = sampleToken ? `${origin}/embed/signed/${sampleToken}` : "";

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
          <PeoplePanel kind="board" id={board} />
          <PublicLinkPanel board={board} />

          <div className="space-y-2 rounded-lg border border-border bg-muted/20 p-3">
            <div className="flex items-start gap-3">
              <KeyRound className="mt-0.5 size-4 shrink-0 text-muted-foreground" />
              <div className="min-w-0 flex-1">
                <p className="text-sm font-medium">Signed embedding</p>
                <p className="text-xs text-muted-foreground">Your app signs a token per viewer, with locked filters.</p>
              </div>
              <Switch checked={embedEnabled} disabled={busy || (infoState === "ready" && !info.supported)}
                onCheckedChange={(v) => void setEmbed(v)} aria-label="Signed embedding" />
            </div>

            {infoState === "failed" ? (
              <p role="alert" className="text-[11px] text-destructive">The signed-embedding state could not be loaded. Close and reopen this dialog to try again.</p>
            ) : null}
            {infoState === "ready" && !info.supported ? (
              <p role="status" className="text-[11px] text-amber-600 dark:text-amber-400">
                Signed embedding is not configured on this server (<code className="rounded bg-muted px-1 font-mono">EMBED_SECRET</code> is not set).
              </p>
            ) : null}

            {embedEnabled && info.supported ? (
              <div className="space-y-2 pt-1">
                <p className="text-[11px] text-muted-foreground">{tokenRules(info.maxLifetimeSeconds)}</p>

                <div className="space-y-1">
                  <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Sample token (valid 1 hour)</p>
                  <div className="flex items-center gap-2">
                    <Input readOnly value={sampleToken} aria-label="Sample token"
                      className="truncate font-mono text-[11px]" onFocus={(e) => e.currentTarget.select()} />
                    <Button size="sm" variant="outline" disabled={!sampleToken} onClick={() => void copyText(sampleToken, "token")}>
                      {copyLabel("token", "Copy token")}
                    </Button>
                    {signedPreviewUrl ? (
                      <a href={signedPreviewUrl} target="_blank" rel="noopener noreferrer" className="shrink-0 text-xs font-medium text-primary hover:underline">Preview ↗</a>
                    ) : null}
                  </div>
                </div>

                <EmbedSettingsPanel board={board} info={info} origin={origin} canWrite={canWrite} onInfo={setInfo} />

                <WithdrawTokens board={board} revokedBefore={info.revokedBefore}
                  onRevokedAll={(at) => setInfo((cur) => ({ ...cur, revokedBefore: at }))} />
              </div>
            ) : null}
          </div>

          <AllowedSitesEditor board={board} saved={info.allowedOrigins}
            onSaved={(origins) => setInfo((cur) => ({ ...cur, allowedOrigins: origins }))} />
        </div>
        {copyFailed ? <p role="alert" className="text-xs text-destructive">Could not copy — select the text and copy it</p> : null}
        <DialogFooter>
          <DialogClose render={<Button variant="ghost" size="sm" />}>Close</DialogClose>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
