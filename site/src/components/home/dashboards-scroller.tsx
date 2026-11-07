"use client";

import * as React from "react";
import { m, useMotionValue, useTransform } from "framer-motion";
import { useSectionProgress } from "@/hooks/use-section-progress";

/**
 * The dashboards section's scroll machinery: the section whose height is
 * the rail's measured overflow plus one screen, the pinned stage, and the
 * rail that vertical scroll slides sideways. The client leaf of
 * `dashboards-section.tsx` (WEB-5 review fix R5): the headline, lead, tiles,
 * shore and caption arrive as server-rendered slots.
 */
export function DashboardsScroller({
  before,
  after,
  children,
}: {
  /** Inside the stage, before the rail: the shore and the heading block. */
  before: React.ReactNode;
  /** Inside the stage, after the rail (none since O32 removed the caption). */
  after: React.ReactNode;
  /** The rail's `<li>` items. */
  children: React.ReactNode;
}) {
  // Refs sit on plain elements, never on `m.*`: since the animation
  // features load after hydration (review fix R1), an `m` element did not
  // hand its node to a ref in time, so the track was never measured, the
  // travel stayed 0 and the rail stopped sliding (found in R5, measured:
  // translateX(-972px) at the end of the pin on 5cf9dfa0, none on
  // d05fde85). The section's height is written straight to its style.
  const ref = React.useRef<HTMLElement>(null);
  const panelRef = React.useRef<HTMLDivElement>(null);
  const railRef = React.useRef<HTMLDivElement>(null);
  const progress = useSectionProgress(ref, ["start start", "end end"]);
  const travel = useMotionValue(0);
  const x = useTransform([progress, travel], ([p, t]: number[]) => -p * t);

  React.useEffect(() => {
    const section = ref.current;
    const track = railRef.current?.firstElementChild;
    if (!section || !track) return;
    // Task B: the light panel is an inset rounded card that grows to the full width as the
    // section scrolls to the top of the screen, and settles back into a card as it leaves
    // (rantai.dev's ExpandingPanel, platform/expanding-panel.tsx: a clip-path on a full-width
    // panel, so the content keeps one width and nothing re-wraps). `--k` is 0 for the card,
    // 1 for full bleed; it is written straight to the style from scroll, with no React state.
    const panel = panelRef.current;
    const still = window.matchMedia("(prefers-reduced-motion: reduce)");
    let frame = 0;
    const clamp = (v: number) => Math.min(1, Math.max(0, v));
    const expand = () => {
      frame = 0;
      if (!panel) return;
      if (still.matches) {
        panel.style.setProperty("--k", "1");
        return;
      }
      const r = section.getBoundingClientRect();
      const vh = window.innerHeight;
      const kIn = clamp((clamp((vh - r.top) / vh) - 0.5) / 0.5);
      const kOut = 1 - clamp((1 - r.bottom / vh) / 0.5);
      const k = Math.min(kIn, kOut);
      panel.style.setProperty("--k", (k * k * (3 - 2 * k)).toFixed(4));
    };
    const onScroll = () => {
      if (!frame) frame = requestAnimationFrame(expand);
    };
    expand();
    window.addEventListener("scroll", onScroll, { passive: true });
    still.addEventListener("change", expand);
    const measure = () => {
      // How far the row sticks out past the viewport, with a little air at the end.
      const t = Math.max(0, track.scrollWidth - window.innerWidth + 32);
      travel.set(t);
      section.style.height = `calc(100vh + ${Math.round(t)}px)`;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(track);
    window.addEventListener("resize", measure);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", measure);
      window.removeEventListener("scroll", onScroll);
      still.removeEventListener("change", expand);
      if (frame) cancelAnimationFrame(frame);
    };
  }, [travel]);

  return (
    <section
      ref={ref}
      id="dashboards"
      className="relative h-[240vh] motion-reduce:h-auto!"
      style={{ height: "calc(100vh + 0px)" }}
    >
      <div
        ref={panelRef}
        className="lk-light h-full overflow-clip bg-[var(--lk-l-bg)] text-[var(--lk-l-text)] [--g:12px] [--k:0] [clip-path:inset(0_calc((1-var(--k))*var(--g))_round_calc((1-var(--k))*28px))] sm:[--g:16px] motion-reduce:h-auto motion-reduce:[clip-path:none]"
      >
        <div className="sticky top-0 flex h-[100svh] flex-col justify-center overflow-hidden motion-reduce:static motion-reduce:h-auto motion-reduce:py-28">
          {before}

          {/* The rail. Pinned: transformed by scroll. Reduced motion: a native horizontal scroller. */}
          <div ref={railRef} className="relative z-10 mt-8 md:mt-12 motion-reduce:overflow-x-auto motion-reduce:pb-4">
            <m.ul
              className="flex w-max gap-4 motion-reduce:transform-none! pr-4 pl-[max(1rem,calc((100vw-1120px)/2+1.5rem))] md:gap-5"
              style={{ x }}
            >
              {children}
            </m.ul>
          </div>
          {after}
        </div>
      </div>
    </section>
  );
}
