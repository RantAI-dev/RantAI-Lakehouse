"use client";

import * as React from "react";
import { Command } from "cmdk";
import { useRouter } from "next/navigation";
import { usePathname } from "next/navigation";
import { useTheme } from "next-themes";
import {
  Search, Sparkles, BarChart3, Plus, Download, Moon, Sun, Clock,
} from "lucide-react";
import { visibleNavGroups, pageTitleFor } from "@/components/app-shell/nav-config";

const OPEN_EVENT = "rantai:open-command";
/** Panggil dari mana saja (mis. box search navbar) untuk membuka palette. */
export function openCommandPalette() {
  window.dispatchEvent(new Event(OPEN_EVENT));
}

type Recent = { href: string; title: string };
const RECENT_KEY = "rantai-recent-pages";

function readRecents(): Recent[] {
  try { return JSON.parse(localStorage.getItem(RECENT_KEY) || "[]"); } catch { return []; }
}

/**
 * Command Palette (⌘K) — navigasi & aksi cepat ala Linear/Vercel. Ketik untuk
 * loncat ke halaman manapun, jalankan aksi (buka Copilot, bikin chart, ekspor
 * YAML, ganti tema), atau buka halaman baru-baru ini. Pola konsol modern untuk
 * menu yang banyak: navigasi sebenarnya lewat sini, sidebar tetap ringkas.
 */
export function CommandPalette() {
  const [open, setOpen] = React.useState(false);
  const [recents, setRecents] = React.useState<Recent[]>([]);
  const router = useRouter();
  const pathname = usePathname();
  const { resolvedTheme, setTheme } = useTheme();

  const groups = visibleNavGroups();

  // Buka via ⌘K / Ctrl+K, dan via event dari box search navbar.
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

  React.useEffect(() => { if (open) setRecents(readRecents()); }, [open]);

  // Catat halaman yang dikunjungi (untuk daftar Recent).
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
          placeholder="Cari halaman atau aksi…"
          className="h-11 w-full bg-transparent text-sm outline-none placeholder:text-muted-foreground"
        />
        <kbd className="hidden rounded border border-border px-1.5 py-0.5 font-mono text-[10px] text-muted-foreground sm:block">esc</kbd>
      </div>

      <Command.List className="max-h-[54vh] overflow-y-auto p-1.5">
        <Command.Empty className="px-3 py-6 text-center text-sm text-muted-foreground">
          Tak ada hasil.
        </Command.Empty>

        {/* Aksi cepat */}
        <Command.Group heading="Aksi cepat" className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-muted-foreground">
          <PaletteItem icon={Sparkles} label="Tanya / bikin lewat AI Copilot" value="ai copilot chat tanya bikin" onSelect={() => go("/copilot")} />
          <PaletteItem icon={BarChart3} label="Buka Dashboards" value="dashboards visualisasi chart" onSelect={() => go("/dashboards")} />
          <PaletteItem icon={Plus} label="Bikin chart baru" value="chart baru buat tambah dashboard" onSelect={() => go("/dashboards")} />
          <PaletteItem icon={Download} label="Ekspor dashboard (YAML)" value="ekspor export yaml dashboard" onSelect={() => run(() => window.open("/api/dashboard/export", "_blank"))} />
          <PaletteItem
            icon={resolvedTheme === "dark" ? Sun : Moon}
            label={`Ganti tema (${resolvedTheme === "dark" ? "terang" : "gelap"})`}
            value="tema theme dark light gelap terang"
            onSelect={() => run(() => setTheme(resolvedTheme === "dark" ? "light" : "dark"))}
          />
        </Command.Group>

        {/* Recent */}
        {recents.length ? (
          <Command.Group heading="Baru dibuka" className="[&_[cmdk-group-heading]]:px-2 [&_[cmdk-group-heading]]:py-1 [&_[cmdk-group-heading]]:text-[11px] [&_[cmdk-group-heading]]:font-medium [&_[cmdk-group-heading]]:uppercase [&_[cmdk-group-heading]]:tracking-wide [&_[cmdk-group-heading]]:text-muted-foreground">
            {recents.map((r) => (
              <PaletteItem key={r.href} icon={Clock} label={r.title} value={`recent ${r.title} ${r.href}`} onSelect={() => go(r.href)} />
            ))}
          </Command.Group>
        ) : null}

        {/* Semua halaman, per section */}
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
  icon: Icon, label, value, onSelect,
}: {
  icon: React.ComponentType<{ className?: string }>;
  label: string;
  value: string;
  onSelect: () => void;
}) {
  return (
    <Command.Item
      value={value}
      onSelect={onSelect}
      className="flex cursor-pointer items-center gap-2.5 rounded-md px-2.5 py-2 text-sm text-foreground data-[selected=true]:bg-accent data-[selected=true]:text-accent-foreground"
    >
      <Icon className="size-4 shrink-0 text-muted-foreground" />
      <span className="flex-1 truncate">{label}</span>
    </Command.Item>
  );
}
