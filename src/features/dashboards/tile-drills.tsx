"use client";

import * as React from "react";
import { ChevronRight } from "lucide-react";
import { normalizeFilters } from "@/lib/dashboard-filter-state";
import {
  canDrillFurther, climbTo, drillBlockReason, drillRequest, fixedFilters, levelAfter, nextLevel, trailOf, type Drilled,
} from "@/lib/hierarchies";
import { hierarchyService } from "@/services";
import type { Hierarchy } from "@/services/contracts/hierarchies";
import type { FilterDef } from "@/services/clients/bi-store";
import type { TileFailure } from "@/services/contracts/dashboards";
import { apiFetch } from "@/services/http";
import type { Cell } from "./tile-dialogs";
import type { ChartRenderSpec } from "@/lib/dashboard-specs";

/**
 * Drill down a hierarchy (`BI-10`, feature page decision 5): a viewer's own,
 * temporary view of a chart one level down for the value clicked, with a trail
 * that climbs back. Nothing is saved and the address does not change.
 *
 * The console never sends a definition: it names the saved chart, the level
 * and the values clicked so far, and the server builds the statement from the
 * saved chart (`routes::drill`), so a drill cannot read another table or drop
 * the chart's own filters.
 */

type ChartWithDef = ChartRenderSpec & { def?: { dimension?: string; kind?: string; grain?: string } & Record<string, unknown> };

/** A drilled tile: the hierarchy it goes down, the values clicked so far, and the rows of the level it shows. */
export type TileDrill = { hierarchy: Hierarchy; steps: Drilled[]; cell: Cell | undefined; loading: boolean };

/** What the click menu shows for a chart: the next level to go to, or the plain reason it cannot. */
export type DrillOffer = { to: string; hierarchy: Hierarchy } | { reason: string };

/** `drills` without the tile `id`. */
function without(drills: Record<string, TileDrill>, id: string): Record<string, TileDrill> {
  return Object.fromEntries(Object.entries(drills).filter(([key]) => key !== id));
}

const sourceKey = (c: ChartWithDef) => (c.sqlSource ? `s:${c.sqlSource}` : `m:${c.mart}`);

async function previewDrill(body: unknown, signal: AbortSignal): Promise<Cell> {
  const res = await apiFetch("/api/dashboard/specs/preview", {
    method: "POST", headers: { "Content-Type": "application/json" }, body: JSON.stringify(body), signal,
  });
  const json = (await res.json().catch(() => null)) as { result?: Cell; error?: string; errorId?: string } | null;
  if (!res.ok || !json?.result) {
    return { error: typeof json?.error === "string" ? json.error : "This level could not be loaded." } as TileFailure;
  }
  return json.result;
}

/**
 * The drills of a dashboard. `charts` are the board's tiles; `filters` the
 * dashboard's active filters, passed to every drilled level so the numbers
 * agree with the rest of the board.
 */
export function useTileDrills(charts: readonly ChartWithDef[], filters: FilterDef[], enabled: boolean) {
  const [hierarchies, setHierarchies] = React.useState<Record<string, Hierarchy[]>>({});
  const [drills, setDrills] = React.useState<Record<string, TileDrill>>({});
  const active = React.useMemo(() => normalizeFilters(filters), [filters]);
  const filtersKey = JSON.stringify(active);

  // The hierarchies of every source the board's own charts read, once per source. A failed read
  // is not an error on the board: the tiles just offer no drill.
  const sources = React.useMemo(() => {
    const seen = new Map<string, ChartWithDef>();
    for (const c of charts) if (c.source !== "builtin" && (c.mart || c.sqlSource)) seen.set(sourceKey(c), c);
    return [...seen.entries()];
  }, [charts]);
  const sourcesKey = sources.map(([k]) => k).join("|");
  React.useEffect(() => {
    if (!enabled) return;
    let live = true;
    for (const [key, c] of sources) {
      void hierarchyService.list(c.sqlSource ? { source: c.sqlSource } : { mart: c.mart })
        .then((found) => { if (live) setHierarchies((h) => ({ ...h, [key]: found })); })
        .catch(() => { if (live) setHierarchies((h) => ({ ...h, [key]: [] })); });
    }
    return () => { live = false; };
    // eslint-disable-next-line react-hooks/exhaustive-deps -- `sources` is rebuilt with `charts`; its keys are what matter
  }, [enabled, sourcesKey]);

  /** What the menu offers for a click on `chart`'s current level. */
  function offer(chart: ChartWithDef): DrillOffer | null {
    if (chart.source === "builtin") return null;
    const dimension = chart.def?.dimension ?? "";
    const list = hierarchies[sourceKey(chart)] ?? [];
    // Nothing is said about a chart whose column is in no hierarchy: there is nothing to drill to.
    if (!list.some((h) => h.levels.includes(dimension))) return null;
    const blocked = drillBlockReason(chart.def);
    if (blocked) return { reason: blocked };
    const drill = drills[chart.id];
    if (drill) {
      const to = canDrillFurther(drill.hierarchy, dimension, drill.steps.length) ? levelAfter(drill.hierarchy, dimension, drill.steps.length + 1) : null;
      return to ? { to, hierarchy: drill.hierarchy } : { reason: "this is the last level of its hierarchy" };
    }
    const next = nextLevel(list, dimension);
    return "reason" in next ? next : { to: next.to, hierarchy: next.hierarchy };
  }

  /** The level a tile shows now: its own dimension, or the drilled one. */
  function currentLevel(chart: ChartWithDef): string {
    const drill = drills[chart.id];
    const dimension = chart.def?.dimension ?? "";
    return drill ? levelAfter(drill.hierarchy, dimension, drill.steps.length) ?? dimension : dimension;
  }

  const controllers = React.useRef<Record<string, AbortController>>({});
  const load = React.useCallback(async (chart: ChartWithDef, hierarchy: Hierarchy, steps: Drilled[]) => {
    const dimension = chart.def?.dimension ?? "";
    const body = drillRequest(chart.id, hierarchy, dimension, steps, active);
    controllers.current[chart.id]?.abort();
    if (!body) { setDrills((d) => without(d, chart.id)); return; }
    const ctl = new AbortController();
    controllers.current[chart.id] = ctl;
    setDrills((d) => ({ ...d, [chart.id]: { hierarchy, steps, cell: d[chart.id]?.cell, loading: true } }));
    try {
      const cell = await previewDrill(body, ctl.signal);
      if (!ctl.signal.aborted) setDrills((d) => (d[chart.id] ? { ...d, [chart.id]: { hierarchy, steps, cell, loading: false } } : d));
    } catch {
      if (!ctl.signal.aborted) setDrills((d) => (d[chart.id] ? { ...d, [chart.id]: { hierarchy, steps, cell: { error: "This level could not be loaded." }, loading: false } } : d));
    }
  }, [active]);

  /** Go one level down for the value clicked (`label` is how the person read it). */
  function down(chart: ChartWithDef, hierarchy: Hierarchy, value: string, label?: string) {
    const steps = [...(drills[chart.id]?.steps ?? []), { value, label }];
    void load(chart, hierarchy, steps);
  }

  /** Climb back to trail entry `index`; 0 is the whole chart. */
  function climb(chart: ChartWithDef, index: number) {
    const drill = drills[chart.id];
    if (!drill) return;
    const steps = climbTo(drill.steps, index);
    controllers.current[chart.id]?.abort();
    if (!steps.length) setDrills((d) => without(d, chart.id));
    else void load(chart, drill.hierarchy, steps);
  }

  // Every drilled level follows the dashboard's filters.
  const drilledKey = Object.entries(drills).map(([id, d]) => `${id}:${d.steps.length}`).join("|");
  React.useEffect(() => {
    if (!drilledKey) return;
    for (const chart of charts) {
      const drill = drills[chart.id];
      if (drill) void load(chart, drill.hierarchy, drill.steps);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps -- re-run when the filters change; `drills` is read, not watched
  }, [filtersKey]);

  /** The filters that fix the values clicked so far, for a records list that must agree with the drilled tile. */
  function fixed(chart: ChartWithDef): FilterDef[] {
    const drill = drills[chart.id];
    if (!drill) return [];
    return fixedFilters(drill.hierarchy, chart.def?.dimension ?? "", drill.steps).map((f) => ({ ...f, op: "in" as const }));
  }

  return { drills, offer, currentLevel, down, climb, fixed };
}

/** The trail above a drilled tile: All > Jawa Barat > Kota Bandung, each entry climbing back to it. */
export function DrillTrail({ steps, onClimb }: { readonly steps: readonly Drilled[]; readonly onClimb: (index: number) => void }) {
  const trail = trailOf(steps);
  return (
    <nav aria-label="Drill trail" className="flex flex-wrap items-center gap-0.5 px-1 pb-1 text-xs">
      {trail.map((entry, i) => (
        <React.Fragment key={i}>
          {i > 0 ? <ChevronRight className="size-3 text-muted-foreground" aria-hidden /> : null}
          {i === trail.length - 1 ? (
            <span aria-current="page" className="max-w-40 truncate font-medium">{entry}</span>
          ) : (
            <button type="button" className="max-w-40 truncate text-muted-foreground underline-offset-2 hover:text-foreground hover:underline" onClick={() => onClimb(i)}>
              {entry}
            </button>
          )}
        </React.Fragment>
      ))}
    </nav>
  );
}
