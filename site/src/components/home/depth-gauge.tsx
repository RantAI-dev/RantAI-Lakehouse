"use client";

import * as React from "react";
import { JOURNEY } from "@/lib/home-content";
import { formatDepth, gaugeDepthAt, SLAB_DEPTHS, type Anchor, type DepthsRange } from "@/lib/depth-journey";
import { measureJourney } from "@/lib/measure-journey";
import { useReducedMotionSafe } from "@/hooks/use-reduced-motion";

/** Every depth a chapter starts at, plus the three slabs: one tick each. */
const TICKS = [...new Set([0, ...JOURNEY.map((c) => c.depth), ...SLAB_DEPTHS])].sort((a, b) => b - a);
/** The scale's full depth: the deepest point. */
const FLOOR = 100;

/**
 * WEB-5 D14 S3: where the visitor is in the dive, the whole way down and
 * back up.
 * - Wide screens (1360 px and up): a thin scale on the left edge, 0 m at the top and the deepest
 *   point at the bottom, with a tick for each chapter, a marker and the
 *   reading in metres. Inside the depths section it hides, because that
 *   section's own gauge stands in the same place with the slabs' labels,
 *   and the readings are the same.
 * - Narrower screens, phones included: a hairline under the top bar whose
 *   fill is the depth, with the reading at its right end. It does not crowd
 *   a 360 px bar, which is already full with the wordmark, the language
 *   switch and the button.
 * - WEB-5 review fix R8 (BLOCKER): the scale used to start at `md` (768 px)
 *   and covered the sections' text up to about 1280 px. Its reading ends at
 *   x 92 px, and the 1120 px column's text starts at x 97 at 1280 and x 137
 *   at 1360 (measured), so the scale starts at 1360 px. There its reading
 *   clears the text by 44 px, at least 37 px even for "−100 m"; the review
 *   asks for 24.
 *
 * The values are written through refs from a passive, rAF-throttled scroll
 * listener (no React state per scroll event, D5). Under reduced motion the
 * reading steps from chapter to chapter. It is decorative (`aria-hidden`):
 * the chapters it marks are the section headings. Without JavaScript it is
 * hidden (app/globals.css).
 */
export function DepthGauge({ surfaceLabel }: { surfaceLabel: string }) {
  const rootRef = React.useRef<HTMLDivElement>(null);
  const readingRef = React.useRef<HTMLSpanElement>(null);
  const phoneReadingRef = React.useRef<HTMLSpanElement>(null);
  const reduced = useReducedMotionSafe();

  React.useEffect(() => {
    const root = rootRef.current;
    if (!root) return;
    let anchors: Anchor[] = [];
    let depths: DepthsRange | null = null;
    let frame = 0;
    let last = "";

    const measure = () => {
      // R9: the same measurement as the water, holding 0 m through the
      // hero's pin because this reading is on screen over the hero.
      ({ anchors, depths } = measureJourney({ holdSurfaceThroughHero: true }));
      paint();
    };
    const paint = () => {
      frame = 0;
      const y = window.scrollY;
      const depth = gaugeDepthAt(y, anchors, depths, reduced);
      const inDepths = depths !== null && y >= depths.top - window.innerHeight * 0.25 && y <= depths.top + depths.span;
      const f = Math.min(1, Math.max(0, -depth / FLOOR));
      const text = formatDepth(depth);
      const key = `${f.toFixed(3)}|${text}|${inDepths}`;
      if (key === last) return;
      last = key;
      root.style.setProperty("--lk-gauge-f", f.toFixed(4));
      root.toggleAttribute("data-in-depths", inDepths);
      if (readingRef.current) readingRef.current.textContent = text;
      if (phoneReadingRef.current) phoneReadingRef.current.textContent = text;
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(paint);
    };

    measure();
    window.addEventListener("scroll", onScroll, { passive: true });
    window.addEventListener("resize", measure);
    const ro = new ResizeObserver(measure);
    ro.observe(document.body);
    return () => {
      if (frame) cancelAnimationFrame(frame);
      window.removeEventListener("scroll", onScroll);
      window.removeEventListener("resize", measure);
      ro.disconnect();
    };
  }, [reduced]);

  return (
    <div ref={rootRef} aria-hidden="true" className="lk-gauge pointer-events-none">
      {/* Wide screens: the scale on the left edge (R8: from 1360 px). */}
      <div className="lk-gauge-scale fixed top-[18%] bottom-[14%] left-8 z-40 hidden w-28 min-[1360px]:block">
        <span className="absolute top-0 left-[7px] h-full w-px bg-gradient-to-b from-[#E8F4FF]/45 via-[#E8F4FF]/25 to-[#E8F4FF]/10" />
        {TICKS.map((d) => (
          <span
            key={d}
            className="absolute left-[3px] block h-px w-[9px] bg-[#E8F4FF]/40"
            style={{ top: `${(-d / FLOOR) * 100}%` }}
          />
        ))}
        <span className="absolute -top-6 left-0 text-[10px] font-medium tracking-[0.18em] text-[#E8F4FF]/55 uppercase">
          {surfaceLabel}
        </span>
        <span className="lk-gauge-marker absolute left-[3px] flex -translate-y-1/2 items-center gap-2.5">
          <span className="block size-[9px] rounded-full bg-[#44A6FA] shadow-[0_0_12px_3px_rgba(68,166,250,0.6)]" />
          <span ref={readingRef} className="text-[11px] font-medium tracking-[0.12em] whitespace-nowrap text-[#E8F4FF]/85 tabular-nums">
            0 m
          </span>
        </span>
      </div>
      {/* Below 1360 px: a hairline under the top bar, with the reading at its end. */}
      <div className="lk-gauge-line fixed inset-x-0 top-16 z-40 h-4 min-[1360px]:hidden">
        <span className="absolute inset-x-0 top-0 h-px bg-[#E8F4FF]/10" />
        <span className="lk-gauge-fill absolute top-0 left-0 h-px w-full origin-left bg-[#44A6FA]/80" />
        <span ref={phoneReadingRef} className="absolute top-1 right-4 text-[10px] font-medium tracking-[0.12em] text-[#E8F4FF]/70 tabular-nums">
          0 m
        </span>
      </div>
    </div>
  );
}
