"use client";

import * as React from "react";
import { calendarLevel } from "@/lib/tile-svg";
import type { CalendarHoverText } from "./calendar-hover-host";

/**
 * The calendar's hover (WEB-5 O18), loaded by `calendar-hover-host.tsx`.
 * One pointer listener on the tile, throttled to a frame, writes the cell
 * it is over into CSS variables and the tooltip's text directly: no React
 * state per move. The cell grows with a ring, its row and column get a
 * faint guide, and the tooltip gives the day and an illustrative count
 * (the drawing's own value, `calendarLevel`). The grid is the drawing's:
 * 20 columns of 17, 7 rows of 23, from (12, 14) in the 360 × 190 tile.
 */
const COLS = 20;
const ROWS = 7;

export default function CalendarHover({ days, week, unit }: Readonly<CalendarHoverText>) {
  const ref = React.useRef<HTMLSpanElement>(null);
  React.useEffect(() => {
    const layer = ref.current;
    const tile = layer?.parentElement?.parentElement;
    const tip = layer?.querySelector<HTMLSpanElement>("[data-tip]");
    if (!layer || !tile || !tip) return;
    let frame = 0;
    let last = "";
    let px = 0;
    let py = 0;
    const paint = () => {
      frame = 0;
      const box = layer.getBoundingClientRect();
      const x = ((px - box.left) / box.width) * 360;
      const y = ((py - box.top) / box.height) * 190;
      const c = Math.floor((x - 12 + 2) / 17);
      const r = Math.floor((y - 14 + 3) / 23);
      const inside = c >= 0 && c < COLS && r >= 0 && r < ROWS;
      const key = inside ? `${c},${r}` : "";
      if (key === last) return;
      last = key;
      layer.toggleAttribute("data-on", inside);
      if (!inside) return;
      layer.style.setProperty("--c", String(c));
      layer.style.setProperty("--r", String(r));
      tip.textContent = `${week} ${c + 1} · ${days[r]} · ${Math.round(40 + calendarLevel(c, r) * 860)} ${unit}`;
      tip.toggleAttribute("data-flip", c > COLS - 6);
    };
    const move = (e: PointerEvent) => {
      px = e.clientX;
      py = e.clientY;
      if (!frame) frame = requestAnimationFrame(paint);
    };
    const leave = () => {
      last = "";
      layer.removeAttribute("data-on");
    };
    tile.addEventListener("pointermove", move);
    tile.addEventListener("pointerleave", leave);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      tile.removeEventListener("pointermove", move);
      tile.removeEventListener("pointerleave", leave);
    };
  }, [days, week, unit]);
  return (
    <span ref={ref} className="lk-calhov pointer-events-none absolute inset-0">
      <span className="lk-calhov-row" />
      <span className="lk-calhov-col" />
      <span className="lk-calhov-cell" />
      <span data-tip className="lk-tip lk-calhov-tip" />
    </span>
  );
}
