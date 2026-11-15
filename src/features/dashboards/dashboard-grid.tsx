"use client";

import * as React from "react";
import { cn } from "@/lib/utils";
import { settleLayout } from "@/lib/grid-layout";
import { reflowNarrow } from "@/lib/grid-reflow";
import {
  PHONE_MAX_WIDTH, collapseLayout, foldedTiles, phoneOrder, placeUnboxed, tabOf,
} from "@/lib/dashboard-tabs";
import type { BoardTab, LayoutMap, PhoneTile, TileBox } from "@/services";
import { PhoneStack } from "./phone-stack";
import { SectionTile, TileCard, isSectionKind } from "./tile-card";
import { useFolded } from "./use-folded";

export type TileMenuItem = {
  label: string;
  icon: React.ReactNode;
  /** Ignored when `sub` is given: the item opens its submenu instead. */
  onSelect: () => void;
  destructive?: boolean;
  /** Start a new group: a separator is drawn above this item. */
  separatorBefore?: boolean;
  /** The item is a submenu of these (`BI-61` T4: "Download ›"). */
  sub?: TileMenuItem[];
};

/**
 * How a tile is drawn: `card`, the bordered card of the embed and the public
 * link; `soft`, the dashboard page's own (`BI-61` T4, layout A): a soft fill
 * with no border and no badge, whose ⋯ and "Data from" line show on hover.
 */
export type TileLook = "card" | "soft";

export type GridItem = {
  id: string;
  /** Chart width chosen in the builder; `2` = full width. Only sizes a tile with no stored box yet. */
  span?: 1 | 2;
  /** `heading`, `divider` and `section` are drawn without a card and fold (BI-18 part C); any other is a card. */
  kind?: string;
  title: string;
  subtitle?: string;
  badge?: React.ReactNode;
  /** Small affordance beside the title, e.g. that the chart is clickable. */
  hint?: React.ReactNode;
  body: React.ReactNode;
  /** Heading of the ⋯ menu (e.g. where the data comes from). */
  menuLabel?: string;
  /**
   * One quiet line at the top of the ⋯ menu (`soft` look): when the data is from
   * and what the tile reads ("12:47 · mart_wisman"), with the full sentence as
   * `detail` for the tooltip and screen readers (BI-61 review round 1, SHOULD-FIX 3).
   */
  menuNote?: { label: string; detail: string };
  /** "Data from 12:47", shown on hover in the `soft` look's header. */
  dataFrom?: string | null;
  menu?: TileMenuItem[];
};

const COLS = 12;
const MARGIN = 12;
const ROW_H = 44;

/**
 * Ensure every item has a box; items without one get placed below by the one
 * rule the server shares (`placeUnboxed`, BI-18 part C review fix (BLOCKER)
 * R13: a row wraps by its tallest tile). A stored layout that already overlaps
 * gets tidied up too. The page hands the grid the layout as shown, so on a
 * board that was saved after R13 this only tidies.
 */
function resolve(items: GridItem[], layout: LayoutMap): LayoutMap {
  return settleLayout(placeUnboxed(items, layout, []));
}

type Drag = { id: string; mode: "move" | "resize"; px: number; py: number; box: TileBox };

/**
 * A 12-column dashboard canvas with DRAG (move) & RESIZE (corner handle),
 * Tableau-style. Custom-built (pointer events) — safe for React 19. In
 * `editable` mode, tiles can be moved and resized; changes are sent via
 * onLayoutChange.
 *
 * BI-18 part C: with `tabs` only the tiles of `tab` are drawn (a layout change
 * is sent back whole, the other tabs' tiles untouched); a `section` tile folds
 * the tiles below it (the viewer's own state, `foldKey`); and a canvas narrower
 * than 640 px, or one in `phoneEdit`, is drawn as one full-width column
 * (`PhoneStack`) in the order of the phone layout, else the reading order.
 *
 * Between the two, a reading viewer whose canvas would draw a tile under
 * 240 px sees that row two tiles a row (`reflowNarrow`, owner QA 2026-11-15);
 * the saved layout is untouched and edit mode shows it as saved.
 */
export function DashboardGrid({
  items, layout, editable, onLayoutChange, tabs, tab, phone, phoneEdit, onPhoneChange, foldKey, look = "card",
}: {
  /** How the tiles are drawn; the dashboard page passes `soft` (`BI-61` T4). */
  look?: TileLook;
  items: GridItem[];
  layout: LayoutMap;
  editable: boolean;
  onLayoutChange: (next: LayoutMap) => void;
  tabs?: BoardTab[];
  tab?: string;
  phone?: PhoneTile[];
  phoneEdit?: boolean;
  onPhoneChange?: (next: PhoneTile[]) => void;
  foldKey?: string;
}) {
  const ref = React.useRef<HTMLDivElement>(null);
  const [width, setWidth] = React.useState(0);
  const [drag, setDrag] = React.useState<Drag | null>(null);
  const [preview, setPreview] = React.useState<LayoutMap | null>(null);
  const [folded, toggleFold] = useFolded(foldKey);

  // Measure width IMMEDIATELY (useLayoutEffect) so tiles don't collapse on the
  // first render, plus a ResizeObserver for width changes.
  React.useLayoutEffect(() => {
    if (ref.current) setWidth(ref.current.getBoundingClientRect().width);
  }, []);
  React.useEffect(() => {
    if (!ref.current) return;
    const ro = new ResizeObserver((e) => setWidth(e[0].contentRect.width));
    ro.observe(ref.current);
    return () => ro.disconnect();
  }, []);

  const tabbed = Boolean(tabs?.length && tab);
  const shown = React.useMemo(
    () => (tabbed ? items.filter((it) => tabOf(tabs ?? [], layout, it.id) === tab) : items),
    [tabbed, items, tabs, layout, tab],
  );
  const base = React.useMemo(() => resolve(shown, layout), [shown, layout]);
  const view = preview ?? base;

  // A layout change goes up whole: this tab's tiles stamped with the tab, and
  // every other tab's tiles as they were.
  const emit = (next: LayoutMap) => {
    if (!tabbed) { onLayoutChange(next); return; }
    const mine = new Set(shown.map((i) => i.id));
    const out: LayoutMap = {};
    for (const [id, box] of Object.entries(layout)) if (!mine.has(id)) out[id] = box;
    for (const [id, box] of Object.entries(next)) out[id] = { ...box, tab };
    onLayoutChange(out);
  };

  const stacked = phoneEdit || (width > 0 && width < PHONE_MAX_WIDTH);
  const kinds = React.useMemo(() => shown.map((i) => ({ id: i.id, kind: i.kind ?? "" })), [shown]);
  // Edit mode shows everything; folding is for reading.
  const folding = !editable && !phoneEdit;

  const hiddenIds = folding && !stacked ? foldedTiles(kinds, view, folded) : new Set<string>();
  const drawn = hiddenIds.size ? collapseLayout(view, kinds, hiddenIds) : view;
  // Owner QA 2026-11-15: reading, a row whose tiles would be drawn under 240 px goes two a row
  // (`reflowNarrow`); edit mode draws the saved grid, so its author edits what is saved.
  const laid = editable || stacked ? drawn : reflowNarrow(drawn, width, { cols: COLS, margin: MARGIN });
  const reflowKey = laid === drawn ? "" : Object.keys(laid).filter((id) => laid[id].w !== drawn[id]?.w || laid[id].x !== drawn[id]?.x).sort().join(" ");
  // A tile that changes row because of the reflow moves there at once: when a side panel opens,
  // the tiles do not all glide across the canvas. Their 200 ms glide is back a frame later.
  const [seenKey, setSeenKey] = React.useState(reflowKey);
  const [instant, setInstant] = React.useState(false);
  if (seenKey !== reflowKey) {
    setSeenKey(reflowKey);
    setInstant(true);
  }
  React.useEffect(() => {
    if (!instant) return;
    let inner = 0;
    const outer = requestAnimationFrame(() => { inner = requestAnimationFrame(() => setInstant(false)); });
    return () => { cancelAnimationFrame(outer); cancelAnimationFrame(inner); };
  }, [instant]);

  // Margin hanya menjadi gap ANTAR kolom/baris. Kanvas tidak menambahkan
  // inset luar agar tepi tile sejajar dengan KPI dan konten dashboard lain.
  const colW = width > 0 ? (width - MARGIN * (COLS - 1)) / COLS : 0;
  const unitX = colW + MARGIN;
  const unitY = ROW_H + MARGIN;
  const boxStyle = (b: TileBox): React.CSSProperties => ({
    position: "absolute",
    left: b.x * unitX,
    top: b.y * unitY,
    width: b.w * colW + (b.w - 1) * MARGIN,
    height: b.h * ROW_H + (b.h - 1) * MARGIN,
  });

  if (stacked) {
    const ordered = phoneOrder(shown.map((i) => i.id), base, phone);
    // Folding follows the column's own order: a section folds what comes after it there.
    const column: LayoutMap = {};
    ordered.forEach((p, i) => { column[p.id] = { x: 0, y: i, w: 12, h: 1 }; });
    const hidden = folding ? foldedTiles(kinds, column, folded) : new Set<string>();
    return (
      <div ref={ref} className="w-full">
        <PhoneStack
          items={shown}
          order={ordered.filter((p) => !hidden.has(p.id))}
          editing={Boolean(phoneEdit)}
          look={look}
          folded={folded}
          onToggleFold={toggleFold}
          onChange={onPhoneChange}
        />
      </div>
    );
  }

  const maxY = Math.max(0, ...Object.values(laid).map((b) => b.y + b.h));
  const canvasH = maxY > 0
    ? maxY * ROW_H + (maxY - 1) * MARGIN + (editable ? unitY : 0)
    : 0;

  // Attach move/up listeners DIRECTLY (imperatively) in startDrag so it does not
  // depend on a re-render/effect — even the first pointermove event is caught.
  const startDrag = (e: React.PointerEvent, id: string, mode: "move" | "resize") => {
    if (!editable || !colW) return;
    e.preventDefault();
    const startBox = base[id];
    if (!startBox) return;
    const minH = isSectionKind(shown.find((i) => i.id === id)?.kind) ? 1 : 3;
    const px = e.clientX, py = e.clientY;
    setDrag({ id, mode, px, py, box: startBox });
    let last: LayoutMap = base;
    const move = (ev: PointerEvent) => {
      const dCols = Math.round((ev.clientX - px) / unitX);
      const dRows = Math.round((ev.clientY - py) / unitY);
      const b = startBox;
      const nb: TileBox = mode === "move"
        ? { ...b, x: Math.min(Math.max(0, b.x + dCols), COLS - b.w), y: Math.max(0, b.y + dRows) }
        : { ...b, w: Math.min(Math.max(2, b.w + dCols), COLS - b.x), h: Math.max(minH, b.h + dRows) };
      // Tile yang ditabrak turun ke bawah, jadi tidak ada yang tertimpa.
      last = settleLayout({ ...base, [id]: nb }, id);
      setPreview(last);
    };
    const up = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", up);
      setPreview(null);
      setDrag(null);
      emit(last);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", up);
  };

  return (
    <div ref={ref} className="dash-canvas relative w-full" style={{ height: canvasH, minHeight: 200 }}>
      {editable ? (
        <div
          // Micro-interaction audit 2026-11-14: the column lines fade in with edit mode.
          className="pointer-events-none absolute inset-0 rounded-lg opacity-25 transition-opacity duration-200 ease-ui starting:opacity-0"
          style={{
            backgroundImage: `linear-gradient(90deg, var(--border) 1px, transparent 1px)`,
            backgroundSize: `${unitX}px 100%`,
            backgroundPosition: "0 0",
          }}
        />
      ) : null}
      {shown.map((it) => {
        const b = laid[it.id];
        if (!b) return null;
        const dragging = drag?.id === it.id;
        const onMove = editable ? (e: React.PointerEvent) => startDrag(e, it.id, "move") : undefined;
        return (
          <div key={it.id} style={boxStyle(b)} className={cn(
            // Micro-interaction audit 2026-11-14: the tiles a drag pushes aside glide on the console's
            // ease-out (was Tailwind's 150 ms default curve); the one under the pointer follows it at once.
            "dash-tile transition-[left,top,width,height] duration-200 ease-ui in-data-[sidebar-moving]:transition-none",
            dragging && "z-20 transition-none",
            instant && "transition-none",
          )}>
            {isSectionKind(it.kind) ? (
              <SectionTile item={it} editable={editable} folded={folded.has(it.id)} onToggle={() => toggleFold(it.id)} onHeaderPointerDown={onMove} />
            ) : (
              <TileCard item={it} editable={editable} dragging={dragging} onHeaderPointerDown={onMove} look={look} />
            )}
            {/* Resize handle (bottom-right corner, visible) */}
            {editable ? (
              <div
                onPointerDown={(e) => startDrag(e, it.id, "resize")}
                className={cn(
                  // Micro-interaction audit 2026-11-14: the corner grows in with edit mode and answers hover and press.
                  "absolute -bottom-1 -right-1 z-10 grid size-6 cursor-se-resize place-items-center rounded-md text-muted-foreground shadow-sm hover:bg-muted hover:text-foreground",
                  "transition-[opacity,scale,background-color,color] duration-200 ease-ui starting:scale-90 starting:opacity-0 active:scale-110",
                  look === "soft" ? "bg-background ring-1 ring-foreground/10" : "border border-border bg-card",
                )}
                style={{ touchAction: "none" }}
                title="Drag to resize"
              >
                <svg viewBox="0 0 10 10" className="size-3"><path d="M9 3v6H3" fill="none" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" /></svg>
              </div>
            ) : null}
          </div>
        );
      })}
    </div>
  );
}
