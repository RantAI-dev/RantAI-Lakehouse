"use client";

import * as React from "react";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";
import { LineageView, PATHS } from "./lineage-view";

/**
 * The governance bento's lineage picture, "column lineage" (owner pick,
 * 2026-11-24, WEB-5; the three pills before it read "terlalu simple").
 *
 * Three small tables, Bronze → Silver → Gold, with their columns. One
 * column's path is traced across them and a light runs along it, then the
 * next path takes over. Pointing at any column traces that column instead.
 * The first path is `amount → revenue`, the one the tile's line describes.
 *
 * Table and column names are code (monospace) and illustrative; what the
 * product records is in `docs/core/features/column-lineage.md`. Needs no
 * copy: table, column and layer names are code or product names. Motion is
 * CSS (`.lk-lt-pulse` in app/globals.css) or a short timer, and runs only
 * while the tile is on screen. Reduced motion and the server markup show
 * the first path as a still, complete picture. `aria-hidden`: the tile's
 * title and line say it.
 */

/** On screen, by the same rule as `LiveTile` (45 % of the tile), so timers stop when it scrolls away. */
function useOnScreen(ref: React.RefObject<HTMLElement | null>): boolean {
  const [on, setOn] = React.useState(false);
  React.useEffect(() => {
    const el = ref.current?.closest(".lk-tile") ?? ref.current;
    if (!el) return;
    const io = new IntersectionObserver(([e]) => setOn(e.isIntersecting), { threshold: 0.45 });
    io.observe(el);
    return () => io.disconnect();
  }, [ref]);
  return on;
}

/** A mouse holds a choice while it is over the picture; a tap picks and lets the loop go on. */
const isMouse = (e: React.PointerEvent) => e.pointerType === "mouse";



export default function ColumnLineageLive() {
  const ref = React.useRef<HTMLDivElement>(null);
  const onScreen = useOnScreen(ref);
  const reduced = useReducedMotionSafe();
  const [path, setPath] = React.useState(0);
  const [held, setHeld] = React.useState(false);

  React.useEffect(() => {
    if (!onScreen || reduced || held) return;
    const t = setInterval(() => setPath((p) => (p + 1) % PATHS.length), 4400);
    return () => clearInterval(t);
  }, [onScreen, reduced, held]);

  const trace = (table: number, row: number) => {
    const p = PATHS.findIndex((x) => x[table] === row);
    if (p >= 0) setPath(p);
  };

  return (
    <LineageView
      path={path}
      rootRef={ref}
      onLeave={(e) => isMouse(e) && setHeld(false)}
      onCell={(kind, ti, ci, e) => {
        if (kind === "click") return trace(ti, ci);
        if (!isMouse(e as React.PointerEvent)) return;
        trace(ti, ci);
        setHeld(true);
      }}
    />
  );
}
