"use client";

import * as React from "react";

/**
 * The bento's pointer controller (WEB-5 owner QA O37), loaded by
 * `bento-fx-host.tsx`. Built here from generic techniques, not taken from
 * any component library:
 * - a soft spotlight that follows the pointer across the whole grid, and a
 *   border glow on every tile that is brightest near the pointer and fades
 *   with distance, so a neighbour catches a little of it (CSS variables,
 *   written once a frame from one `pointermove` on the grid; no React
 *   state);
 * - a few foam dots that drift inside the hovered tile, from one pool of
 *   ten that moves with the pointer and stops when it leaves;
 * - a soft ring from the press point, on any pointer, touch included.
 * No tilt and no magnetism. Pointer effects only where a mouse hovers
 * (`(hover: hover) and (pointer: fine)`); under reduced motion none of the
 * motion runs, and the borders stay as they are.
 */
const DOTS = 10;

export default function BentoFx() {
  const ref = React.useRef<HTMLSpanElement>(null);

  React.useEffect(() => {
    const grid = ref.current?.parentElement?.parentElement;
    if (!grid) return;
    const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    const hover = window.matchMedia("(hover: hover) and (pointer: fine)").matches;
    const tiles = [...grid.querySelectorAll<HTMLElement>(".lk-mb-tile")];
    const cleanups: (() => void)[] = [];

    // The ripple: any pointer, touch included; not under reduced motion.
    if (!reduce) {
      const press = (e: PointerEvent) => {
        const tile = (e.target as Element).closest<HTMLElement>(".lk-mb-tile");
        if (!tile) return;
        const r = tile.getBoundingClientRect();
        const ring = document.createElement("span");
        ring.className = "lk-mb-ripple";
        ring.style.left = `${e.clientX - r.left}px`;
        ring.style.top = `${e.clientY - r.top}px`;
        tile.appendChild(ring);
        ring.addEventListener("animationend", () => ring.remove(), { once: true });
      };
      grid.addEventListener("pointerdown", press);
      cleanups.push(() => grid.removeEventListener("pointerdown", press));
    }

    if (hover && !reduce) {
      const pool = document.createElement("span");
      pool.className = "lk-mb-dots";
      pool.setAttribute("aria-hidden", "true");
      for (let i = 0; i < DOTS; i++) {
        const d = document.createElement("span");
        d.className = "lk-mb-dot";
        d.style.left = `${8 + ((i * 37) % 84)}%`;
        d.style.top = `${10 + ((i * 53) % 80)}%`;
        d.style.setProperty("--dx", `${((i * 29) % 21) - 10}px`);
        d.style.setProperty("--dy", `${-12 - ((i * 17) % 18)}px`);
        d.style.animationDelay = `${(i * 0.37) % 2.4}s`;
        pool.appendChild(d);
      }
      let frame = 0;
      let px = 0;
      let py = 0;
      const paint = () => {
        frame = 0;
        const g = grid.getBoundingClientRect();
        grid.style.setProperty("--mx", `${px - g.left}px`);
        grid.style.setProperty("--my", `${py - g.top}px`);
        for (const t of tiles) {
          const r = t.getBoundingClientRect();
          t.style.setProperty("--x", `${px - r.left}px`);
          t.style.setProperty("--y", `${py - r.top}px`);
        }
      };
      const move = (e: PointerEvent) => {
        if (e.pointerType !== "mouse") return;
        px = e.clientX;
        py = e.clientY;
        if (!frame) frame = requestAnimationFrame(paint);
        const tile = (e.target as Element).closest<HTMLElement>(".lk-mb-tile");
        if (tile && pool.parentElement !== tile) tile.appendChild(pool);
      };
      const enter = () => grid.setAttribute("data-fx", "on");
      const leave = () => {
        grid.removeAttribute("data-fx");
        pool.remove();
      };
      grid.addEventListener("pointermove", move);
      grid.addEventListener("pointerenter", enter);
      grid.addEventListener("pointerleave", leave);
      enter();
      cleanups.push(() => {
        if (frame) cancelAnimationFrame(frame);
        grid.removeEventListener("pointermove", move);
        grid.removeEventListener("pointerenter", enter);
        grid.removeEventListener("pointerleave", leave);
        leave();
      });
    }
    return () => cleanups.forEach((c) => c());
  }, []);

  return <span ref={ref} hidden />;
}
