"use client";

import * as React from "react";
import { ascentAt, causticAt, depthAt, steppedDepthAt, waterAt, type Anchor } from "@/lib/depth-journey";
import { measureJourney } from "@/lib/measure-journey";
import { afterPaint } from "@/lib/after-paint";
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
 *
 * S5, the ascent: from the dashboards up to the surface, moonlight caustics
 * come back into the water, stronger the shallower it is, and bubbles rise
 * at the screen's edges. Both are scaled by two more CSS variables from the
 * same listener (`--lk-ascent`, `--lk-caustic`). The bubbles are mounted
 * only once the page is usable (`afterPaint`), and neither shows under
 * reduced motion or without JavaScript.
 */
export function DepthBackdrop() {
  const ref = React.useRef<HTMLDivElement>(null);
  const reduced = useReducedMotionSafe();

  React.useEffect(() => {
    const el = ref.current;
    if (!el) return;
    let anchors: Anchor[] = [];
    let climb = { from: 0, to: 0 };
    let frame = 0;
    let last = "";

    // R9: the same measurement as the gauge. No hold through the hero: the
    // water is covered there (see `journeyFromBoxes`).
    const measure = () => {
      ({ anchors, climb } = measureJourney({ holdSurfaceThroughHero: false }));
      paint();
    };
    const paint = () => {
      frame = 0;
      // Stepped (reduced motion): a chapter's water applies as soon as its top
      // enters the screen, half a screen before its blended anchor, so no
      // chapter shows in the water of the one above it.
      const depth = reduced
        ? steppedDepthAt(window.scrollY + window.innerHeight / 2, anchors)
        : depthAt(window.scrollY, anchors);
      const water = waterAt(depth);
      const ascent = reduced ? 0 : ascentAt(window.scrollY, climb.from, climb.to, window.innerHeight * 0.6);
      const caustic = ascent * causticAt(depth);
      const key = `${water.top}${water.bottom}|${ascent.toFixed(2)}|${caustic.toFixed(2)}`;
      if (key === last) return;
      last = key;
      el.style.setProperty("--lk-water-top", water.top);
      el.style.setProperty("--lk-water-bottom", water.bottom);
      el.style.setProperty("--lk-ascent", ascent.toFixed(2));
      el.style.setProperty("--lk-caustic", caustic.toFixed(2));
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

  const showBubbles = useAfterPaint() && !reduced;

  return (
    <div ref={ref} aria-hidden="true" className="lk-water pointer-events-none fixed inset-0 overflow-hidden">
      {/* Moonlight from above, stronger as the water gets shallower (S5). */}
      <div className="lk-water-caustics absolute inset-0">
        <div className="lk-caustic-a absolute -top-1/3 left-[10%] size-[70%] rounded-full bg-[radial-gradient(circle,rgba(120,190,255,0.22)_0%,transparent_60%)] blur-2xl" />
        <div className="lk-caustic-b absolute -top-1/4 right-[5%] size-[60%] rounded-full bg-[radial-gradient(circle,rgba(150,205,255,0.18)_0%,transparent_60%)] blur-3xl" />
      </div>
      {showBubbles && <Bubbles />}
    </div>
  );
}

/**
 * A dozen bubbles rising at the edges of the screen, a CSS animation each
 * (transform and opacity only). Positions, sizes and timings are fixed
 * numbers, not random, so every visit looks the same.
 */
const BUBBLES = Array.from({ length: 12 }, (_, i) => ({
  left: i % 2 === 0 ? 2 + ((i * 7) % 11) : 88 + ((i * 5) % 10),
  size: 3 + ((i * 3) % 6),
  duration: 9 + ((i * 7) % 8),
  delay: -((i * 13) % 17),
}));

function Bubbles() {
  return (
    <div className="lk-bubbles absolute inset-0">
      {BUBBLES.map((b, i) => (
        <span
          key={i}
          className="lk-bubble absolute bottom-0 rounded-full border border-[#E8F4FF]/45 bg-[#E8F4FF]/10"
          style={{
            left: `${b.left}%`,
            width: b.size,
            height: b.size,
            animationDuration: `${b.duration}s`,
            animationDelay: `${b.delay}s`,
          }}
        />
      ))}
    </div>
  );
}

/** True once the page is usable (`afterPaint`); false on the server and in the first render. */
let painted = false;
let paintScheduled = false;
const paintListeners = new Set<() => void>();
function subscribePainted(onChange: () => void): () => void {
  paintListeners.add(onChange);
  if (!paintScheduled) {
    paintScheduled = true;
    afterPaint(() => {
      painted = true;
      for (const l of paintListeners) l();
    });
  }
  return () => {
    paintListeners.delete(onChange);
  };
}
function useAfterPaint(): boolean {
  return React.useSyncExternalStore(subscribePainted, () => painted, () => false);
}
