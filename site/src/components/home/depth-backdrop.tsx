"use client";

import * as React from "react";
import { JOURNEY, SURFACE_AT_FINAL } from "@/lib/home-content";
import { depthAt, steppedDepthAt, waterAt, type Anchor } from "@/lib/depth-journey";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/**
 * WEB-5 D14 S2: one water behind the second half of the page. From the
 * assistant (the deepest point) to the final call (the surface), the
 * sections have no background of their own; this fixed layer is the water
 * they sit in, and its colour follows the depth as the page scrolls. The
 * colour lightens on the way up and turns into the night sky at the
 * surface, and section edges no longer show as bands.
 *
 * - Before the assistant the 3D canvas or the scene zone's own gradient
 *   covers it, and its colour there is already the deep water those end on,
 *   so there is no seam where they stop.
 * - The colour is written to two CSS variables through a ref, from a
 *   passive scroll listener, once per frame and only when it changes. There
 *   is no React state per scroll event (D5).
 * - Reduced motion: the colour steps from chapter to chapter instead of
 *   blending.
 * - Without JavaScript the sections keep their own backgrounds
 *   (`.lk-fallback-bg` in app/globals.css), so the page reads as before.
 */
export function DepthBackdrop() {
  const ref = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();

  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let anchors: Anchor[] = [];
    let frame = 0;
    let last = "";

    // Each chapter's depth applies when its top reaches the middle of the
    // screen; the surface is reached halfway into the final section.
    const measure = () => {
      const half = window.innerHeight / 2;
      const next: Anchor[] = [];
      for (const c of JOURNEY) {
        const node = document.getElementById(c.id);
        if (!node) continue;
        next.push({ at: node.getBoundingClientRect().top + window.scrollY - half, depth: c.depth });
      }
      const final = document.getElementById("final");
      if (final) {
        const r = final.getBoundingClientRect();
        next.push({ at: r.top + window.scrollY + r.height * SURFACE_AT_FINAL - half, depth: 0 });
      }
      anchors = next.sort((a, b) => a.at - b.at);
      paint();
    };
    const paint = () => {
      frame = 0;
      const depth = (reduced ? steppedDepthAt : depthAt)(window.scrollY, anchors);
      const water = waterAt(depth);
      const key = `${water.top}${water.bottom}`;
      if (key === last) return;
      last = key;
      el.style.setProperty("--lk-water-top", water.top);
      el.style.setProperty("--lk-water-bottom", water.bottom);
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(paint);
    };

    measure();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", measure);
    // Fonts, the live hero's longer height and the dashboards rail all move
    // the chapters after load.
    const ro = new ResizeObserver(measure);
    ro.observe(document.body);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", measure);
      ro.disconnect();
    };
  }, [reduced]);

  return <div ref={ref} aria-hidden="true" className="lk-water pointer-events-none fixed inset-0" />;
}
