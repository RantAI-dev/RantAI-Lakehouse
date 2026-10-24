"use client";

import * as React from "react";
import { Check, Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { copyText as copyToClipboard } from "@/lib/copy-text";
import { readableErrorMessage } from "@/lib/notify-message";
// Features import `@/services` only.
import { dashboardService, type BoardChart, type EmbedInfo, type FilterMode } from "@/services";
import { Disclosure } from "./embed-access-panel";
import { embedLookQuery, iframeSnippet, signSnippet } from "./share-embed";

type Theme = "light" | "dark" | "dashboard";
const MODES: { value: FilterMode; label: string; hint: string }[] = [
  { value: "locked", label: "Locked", hint: "the token must set it; the viewer cannot change it" },
  { value: "editable", label: "Editable", hint: "the viewer sees and changes it" },
  { value: "hidden", label: "Hidden", hint: "not shown; the dashboard's saved value applies" },
];

function sentence(e: unknown): string {
  return readableErrorMessage(e instanceof Error ? e.message : "The change was not made.");
}

/**
 * What a signed embed shows and does (`BI-26` plan D9): the whole dashboard or
 * one chart, the look in the address, each filter's mode, a live preview, and
 * the snippets a customer copies. Everything the snippet says about the token
 * comes from what is saved: the locked filters it must set, the chart it names.
 */
export function EmbedSettingsPanel({
  board, info, origin, canWrite = true, onInfo,
}: {
  readonly board: string;
  readonly info: EmbedInfo;
  readonly origin: string;
  readonly canWrite?: boolean;
  /** The dialog's copy of the state, after the modes are saved. */
  readonly onInfo: (next: EmbedInfo) => void;
}) {
  const [target, setTarget] = React.useState<"dashboard" | "chart">("dashboard");
  const [charts, setCharts] = React.useState<BoardChart[]>([]);
  const [chart, setChart] = React.useState("");
  const [sample, setSample] = React.useState(info.sampleToken ?? "");
  const [theme, setTheme] = React.useState<Theme>("dashboard");
  const [hideName, setHideName] = React.useState(false);
  const [transparent, setTransparent] = React.useState(false);
  const saved = info.filterModes ?? {};
  const columns = info.filterColumns ?? [];
  const [draft, setDraft] = React.useState<Record<string, FilterMode>>(saved);
  const [busy, setBusy] = React.useState(false);
  const [message, setMessage] = React.useState<string | null>(null);
  const [copied, setCopied] = React.useState<string | false>(false);
  const [previewOn, setPreviewOn] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    dashboardService.listBoardCharts(board).then((c) => { if (!cancelled) setCharts(c); }).catch(() => { if (!cancelled) setCharts([]); });
    return () => { cancelled = true; };
  }, [board]);

  // The sample token names what the target names, so the preview shows exactly that.
  React.useEffect(() => {
    if (target === "chart" && !chart) return;
    let cancelled = false;
    dashboardService
      .getEmbedInfo(board, undefined, target === "chart" ? chart : undefined)
      .then((i) => { if (!cancelled) setSample(i.sampleToken ?? ""); })
      .catch(() => { if (!cancelled) setSample(""); });
    return () => { cancelled = true; };
  }, [board, target, chart]);

  const modeOf = (c: string): FilterMode => draft[c] ?? "hidden";
  const changed = columns.some((c) => modeOf(c) !== (saved[c] ?? "hidden"));
  const lockedSaved = columns.filter((c) => (saved[c] ?? "hidden") === "locked");
  const query = embedLookQuery({ theme, hideName, transparent });
  const chartId = target === "chart" ? chart : undefined;
  const sign = signSnippet({
    board, origin, maxLifetimeSeconds: info.maxLifetimeSeconds, chart: chartId, lockedFilters: lockedSaved, query,
  });
  const frame = iframeSnippet({ url: `${origin}/embed/signed/<TOKEN FROM YOUR SERVER>${query ? `?${query}` : ""}`, origin });
  const previewUrl = sample && (target === "dashboard" || chart) ? `${origin}/embed/signed/${sample}${query ? `?${query}` : ""}` : "";

  async function saveModes() {
    setBusy(true);
    setMessage(null);
    try {
      const next: Record<string, FilterMode> = Object.fromEntries(columns.map((c) => [c, modeOf(c)]));
      await dashboardService.setEmbedFilterModes(board, next);
      onInfo({ ...info, filterModes: next });
    } catch (e) {
      setMessage(sentence(e));
    } finally { setBusy(false); }
  }

  async function copy(text: string, key: string) {
    const ok = await copyToClipboard(text);
    setCopied(ok ? key : false);
    setTimeout(() => setCopied(false), 1800);
  }
  const copyLabel = (k: string, label: string) => (
    <>{copied === k ? <Check className="size-4 text-emerald-500" /> : <Copy className="size-4" />}{copied === k ? "Copied" : label}</>
  );

  return (
    <div className="space-y-3">
      <fieldset className="space-y-1.5">
        <legend className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">What the token shows</legend>
        <label className="flex items-center gap-2 text-xs">
          <input type="radio" name="embed-target" checked={target === "dashboard"} onChange={() => setTarget("dashboard")} />
          Dashboard
        </label>
        <label className="flex items-center gap-2 text-xs">
          <input type="radio" name="embed-target" checked={target === "chart"} onChange={() => setTarget("chart")} />
          One chart
        </label>
        {target === "chart" ? (
          <select aria-label="Chart" value={chart} onChange={(e) => setChart(e.target.value)}
            className="h-8 w-full rounded-md border border-border bg-background px-2 text-xs">
            <option value="">Choose a chart…</option>
            {charts.map((c) => <option key={c.id} value={c.id}>{c.title}</option>)}
          </select>
        ) : null}
        {target === "chart" ? (
          <p className="text-[11px] text-muted-foreground">A chart token reads that chart and nothing else; the address cannot turn it into the dashboard.</p>
        ) : null}
      </fieldset>

      <fieldset className="space-y-1.5">
        <legend className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Look (in the address, not the token)</legend>
        <select aria-label="Theme" value={theme} onChange={(e) => setTheme(e.target.value as Theme)}
          className="h-8 rounded-md border border-border bg-background px-2 text-xs">
          <option value="dashboard">The dashboard&apos;s own look</option>
          <option value="light">Light</option>
          <option value="dark">Dark</option>
        </select>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={hideName} onChange={(e) => setHideName(e.target.checked)} />
          Hide the Rantai Lake name
        </label>
        <label className="flex items-center gap-2 text-xs">
          <input type="checkbox" checked={transparent} onChange={(e) => setTransparent(e.target.checked)} />
          Let the page behind show through
        </label>
      </fieldset>

      <div className="space-y-1.5">
        <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Filters</p>
        {columns.length === 0 ? (
          <p className="text-[11px] text-muted-foreground">This dashboard has no saved filters. A filter you save on the dashboard appears here, hidden until you choose.</p>
        ) : (
          <ul className="space-y-1">
            {columns.map((c) => (
              <li key={c} className="flex items-center gap-2 text-xs">
                <span className="min-w-0 flex-1 truncate font-mono">{c}</span>
                <select aria-label={`Mode of ${c}`} value={modeOf(c)} disabled={!canWrite}
                  onChange={(e) => setDraft((cur) => ({ ...cur, [c]: e.target.value as FilterMode }))}
                  title={MODES.find((m) => m.value === modeOf(c))?.hint}
                  className="h-7 rounded-md border border-border bg-background px-2 text-xs">
                  {MODES.map((m) => <option key={m.value} value={m.value}>{m.label}</option>)}
                </select>
              </li>
            ))}
          </ul>
        )}
        {columns.length > 0 ? (
          <div className="flex items-center gap-2">
            <Button size="sm" variant="outline" disabled={!changed || busy || !canWrite} onClick={() => void saveModes()}>Save filter modes</Button>
            {changed ? <span role="status" className="text-[11px] text-amber-600 dark:text-amber-400">Unsaved changes</span> : null}
          </div>
        ) : null}
        <p className="text-[11px] text-muted-foreground">A locked filter must have a value in the token, or the token is refused.</p>
        {message ? <p role="alert" className="text-[11px] text-destructive">{message}</p> : null}
      </div>

      <div className="space-y-1.5">
        <p className="text-[11px] font-medium uppercase tracking-wide text-muted-foreground">Try the look</p>
        {previewUrl && previewOn ? (
          <iframe title="Embed preview" src={previewUrl} className="h-56 w-full rounded-md border border-border" />
        ) : previewUrl ? (
          // Loaded on request: the frame opens a page with a live token, and a
          // dialog that only reads its settings should not fetch it.
          <Button size="sm" variant="outline" onClick={() => setPreviewOn(true)}>Show preview</Button>
        ) : (
          <p className="text-[11px] text-muted-foreground">{target === "chart" ? "Choose a chart to preview it." : "No sample token to preview."}</p>
        )}
      </div>

      <Disclosure title="How to sign a token">
        <p className="text-[11px] text-muted-foreground">The signing secret stays on your server and is never sent to the console. Set <code className="rounded bg-muted px-1 font-mono">EMBED_SECRET</code> there to the same value the backend uses.</p>
        <div className="rounded-md border border-border bg-background p-2">
          <code className="block max-h-40 overflow-auto whitespace-pre font-mono text-[11px] leading-relaxed text-foreground">{sign}</code>
        </div>
        <Button size="sm" variant="outline" onClick={() => void copy(sign, "sign")}>{copyLabel("sign", "Copy code")}</Button>
      </Disclosure>

      <Disclosure title="Embed on your page (the frame follows its content)">
        <div className="rounded-md border border-border bg-background p-2">
          <code className="block max-h-40 overflow-auto whitespace-pre font-mono text-[11px] leading-relaxed text-foreground">{frame}</code>
        </div>
        <Button size="sm" variant="outline" onClick={() => void copy(frame, "frame")}>{copyLabel("frame", "Copy snippet")}</Button>
        <p className="text-[11px] text-muted-foreground">Without the listener the frame keeps the height you give it. Only the sites listed under Allowed sites get the height. A single chart fills its frame and sends no height.</p>
      </Disclosure>
    </div>
  );
}
