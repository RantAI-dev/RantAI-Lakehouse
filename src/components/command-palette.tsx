"use client";

import * as React from "react";
import { Command } from "cmdk";
import { useRouter } from "next/navigation";
import { usePathname } from "next/navigation";
import { useTheme } from "next-themes";
import {
  Search, Sparkles, BarChart3, LayoutGrid, Plus, Download, Moon, Sun, Clock, Database,
} from "lucide-react";
import { NAV_GROUPS, pageTitleFor } from "@/components/app-shell/nav-config";
import { assetService } from "@/services";
import { isServiceError } from "@/services/errors";
import type { Asset } from "@/services/contracts/assets";
import { capPaletteAssetResults, matchedOnLabel } from "@/lib/palette-search";

/** Debounce, in ms, before a typed search term reaches `assetService`
 * (WS2 §13). */
const CATALOG_SEARCH_DEBOUNCE_MS = 250;

const OPEN_EVENT = "rantai:open-command";
/** Call from anywhere (e.g. the navbar search box) to open the palette. */
export function openCommandPalette() {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

/** What the box says when a search did not return assets. Upstream error text
 * never reaches the user (principle 4): only a `403` and a `supported: false`
 * answer (thrown with status `200` by `loadCatalog`) have their own line. */
function failureLine(err: unknown): string {
  if (isServiceError(err)) {
    if (err.code === "permission_denied") return "You do not have access to the catalog";
    if (err.status === 200) return err.message;
  }
  return "Catalog search is unavailable";
}

type Recent = { href: string; title: string };
const RECENT_KEY = "rantai-recent-pages";

function readRecents(): Recent[] {
  try { return JSON.parse(localStorage.getItem(RECENT_KEY) || "[]"); } catch { return []; }
}

/**
 * Command Palette (⌘K) — Linear/Vercel-style navigation & quick actions.
 * Type to jump to any page, run an action (open Copilot, build a chart,
 * export YAML, change theme), or open a recent page. A modern console
 * pattern for a large menu: real navigation happens here, keeping the
 * sidebar compact.
 */
export function CommandPalette() {
  const [open, setOpen] = React.useState(false);
  const [recents, setRecents] = React.useState<Recent[]>([]);
  const [search, setSearch] = React.useState("");
  const [assetResults, setAssetResults] = React.useState<Asset[]>([]);
  // A failed search is said out loud (principle 2); an empty list would
  // read as "nothing matches".
  // The line to show for it: a refusal, a `supported: false` reason, or the
  // outage text (DATA-11 review SHOULD-FIX 4). Null when the search worked.
  const [searchFailed, setSearchFailed] = React.useState<string | null>(null);
  const router = useRouter();
  const pathname = usePathname();
  const { resolvedTheme, setTheme } = useTheme();

  const groups = NAV_GROUPS;

  // Server-side catalog search behind the "Catalog assets" group (WS2 §13).
  // Debounced 250ms and abortable on every keystroke so a slow
  // response for an earlier term can never clobber a later one's result —
  // `cancelled` guards state updates from a request whose signal already
  // aborted or whose debounce timer never fired. A failed request shows
  // "Catalog search is unavailable", not a spinner that never resolves and
  // not an empty list (DATA-11 T2).
  React.useEffect(() => {
    const term = search.trim();
    if (!term) {
      setAssetResults([]);
      setSearchFailed(null);
      return;
    }
    let cancelled = false;
    const controller = new AbortController();
    const timer = setTimeout(() => {
      assetService
        .listAssets({ search: term }, controller.signal)
        .then((assets) => {
          if (cancelled) return;
          setSearchFailed(null);
          setAssetResults(capPaletteAssetResults(assets));
        })
        .catch((err: unknown) => {
          if (cancelled) return;
          setAssetResults([]);
          setSearchFailed(failureLine(err));
        });
    }, CATALOG_SEARCH_DEBOUNCE_MS);
    return () => {
      cancelled = true;
      clearTimeout(timer);
      controller.abort();
    };
  }, [search]);

  // Open via ⌘K / Ctrl+K, and via an event from the navbar search box.
  React.useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.metaKey || e.ctrlKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((o) => !o);
      }
    };
    const onOpen = () => setOpen(true);
    document.addEventListener("keydown", onKey);
    window.addEventListener(OPEN_EVENT, onOpen);
    return () => {
      document.removeEventListener("keydown", onKey);
      window.removeEventListener(OPEN_EVENT, onOpen);
    };
  }, []);

  React.useEffect(() => {
    if (open) setRecents(readRecents());
    else setSearch("");
  }, [open]);

  // Record visited pages (for the Recent list).
  React.useEffect(() => {
    const title = pageTitleFor(pathname);
    const href = pathname;
    try {
      const prev = readRecents().filter((r) => r.href !== href);
      const next = [{ href, title }, ...prev].slice(0, 6);
      localStorage.setItem(RECENT_KEY, JSON.stringify(next));
    } catch { /* ignore */ }
  }, [pathname]);

  const go = (href: string) => { setOpen(false); router.push(href); };
  const run = (fn: () => void) => { setOpen(false); fn(); };

  return (
    <Command.Dialog
      open={open}
      onOpenChange={setOpen}
      label="Command Palette"
      contentClassName="fixed left-1/2 top-[18%] z-[100] w-[min(92vw,560px)] -translate-x-1/2 overflow-hidden rounded-xl border border-border bg-popover shadow-2xl"
      overlayClassName="fixed inset-0 z-[99] bg-black/40 backdrop-blur-sm"
    >
      <div className="flex items-center gap-2 border-b border-border px-3">
        <Search className="size-4 shrink-0 text-muted-foreground" />
        <Command.Input
          autoFocus
          placeholder="Search pages, actions, or catalog assets…"
          className="h-11 w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
          value={search}
          onValueChange={setSearch}
        />
        <kbd className="hidden rounded border border-border px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground sm:block">esc</kbd>
      </div>

      <Command.List className="max-h-[54vh] overflow-y-auto p-1.5">
        {/* Not while the server has results: they are force-mounted, so
            cmdk's own count would say "No results." under them. */}
        {assetResults.length === 0 ? (
          <Command.Empty className="px-3 py-6 text-center text-sm text-muted-foreground">
            No results.
          </Command.Empty>
        ) : null}
        {searchFailed ? (
          <div role="alert" className="px-3 py-2 text-sm text-muted-foreground">
            {searchFailed}
          </div>
        ) : null}

        {/* Quick actions */}
        <Command.Group heading="Quick actions" className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-muted-foreground">
          <PaletteItem icon={Sparkles} label="Ask / build via AI Copilot" value="ai copilot chat ask build" onSelect={() => go("/")} />
          {/* "Open Dashboards" sengaja menuju `/dashboards`: itu penerus yang
              membawa ke board terakhir, jadi label ini menepati janjinya.
              Mengelola daftarnya adalah tujuan lain, maka barisnya sendiri. */}
          <PaletteItem icon={BarChart3} label="Open Dashboards" value="dashboards visualization chart" onSelect={() => go("/dashboards")} />
          {/* Membuat chart dilakukan DI ATAS sebuah dashboard, bukan di
              ruang kosong — jadi barisan ini sengaja menuju penerus yang
              sama, lalu kanvasnya yang menyediakan tombol tambah tile. */}
          <PaletteItem icon={Plus} label="Add a chart to a dashboard" value="new chart add tile dashboard" onSelect={() => go("/dashboards")} />
          <PaletteItem icon={LayoutGrid} label="Browse all dashboards" value="dashboards list manage browse rename share" onSelect={() => go("/dashboards/browse")} />
          <PaletteItem icon={Download} label="Export dashboard (YAML)" value="export yaml dashboard" onSelect={() => run(() => window.open("/api/dashboard/export", "_blank"))} />
          <PaletteItem
            icon={resolvedTheme === "dark" ? Sun : Moon}
            label={`Change theme (${resolvedTheme === "dark" ? "light" : "dark"})`}
            value="theme dark light"
            onSelect={() => run(() => setTheme(resolvedTheme === "dark" ? "light" : "dark"))}
          />
        </Command.Group>

        {/* Recent */}
        {recents.length ? (
          <Command.Group heading="Recent" className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-muted-foreground">
            {recents.map((r) => (
              <PaletteItem key={r.href} icon={Clock} label={r.title} value={`recent ${r.title} ${r.href}`} onSelect={() => go(r.href)} />
            ))}
          </Command.Group>
        ) : null}

        {/* Catalog assets — server-side search, WS2 §13. forceMount: the
            server already decided these match (by description, tag or
            column, which are not in the item's value), so cmdk's own
            filter must not hide them (DATA-11 F6). */}
        {assetResults.length ? (
          <Command.Group forceMount heading="Catalog assets" className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-muted-foreground">
            {assetResults.map((a) => (
              <PaletteItem
                key={a.id}
                forceMount
                icon={Database}
                label={a.name}
                detail={matchedOnLabel(a.matchedOn)}
                value={`asset ${a.id} ${a.name}`}
                onSelect={() => go(`/data/assets/${a.id}`)}
              />
            ))}
            <PaletteItem
              forceMount
              icon={Search}
              label="See all results"
              value={`see all results ${search.trim()}`}
              onSelect={() => go(`/data?search=${encodeURIComponent(search.trim())}`)}
            />
          </Command.Group>
        ) : null}

        {/* All pages, per section */}
        {groups.map((g) => (
          <Command.Group
            key={g.label}
            heading={g.label}
            className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-muted-foreground"
          >
            {g.items.map((it) => (
              <PaletteItem key={it.href} icon={it.icon} label={it.title} value={`${g.label} ${it.title} ${it.href}`} onSelect={() => go(it.href)} />
            ))}
          </Command.Group>
        ))}
      </Command.List>
    </Command.Dialog>
  );
}

function PaletteItem({
  icon: Icon, label, detail, value, forceMount, onSelect,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  /** A second, smaller line under the label. */
  detail?: string | null;
  value: string;
  forceMount?: boolean;
  onSelect: () => void;
}) {
  return (
    <Command.Item
      value={value}
      forceMount={forceMount}
      onSelect={onSelect}
      className="flex cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-2 text-sm text-foreground data-[selected=true]:bg-accent data-[selected=true]:text-accent-foreground"
    >
      <Icon className="size-4 shrink-0 text-muted-foreground" />
      <span className="min-w-0 flex-1">
        <span className="block truncate">{label}</span>
        {detail ? <span className="block truncate text-xs text-muted-foreground">{detail}</span> : null}
      </span>
    </Command.Item>
  );
}
