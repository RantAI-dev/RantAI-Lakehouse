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
  const railRef = React.useRef<HTMLDivElement>(null);
  const dotsRef = React.useRef<HTMLOListElement>(null);
  const progress = useSectionProgress(ref, ["start start", "end end"]);
  const travel = useMotionValue(0);
  // WEB-5 M2 (phone pass): below 1024 px the rail steps one tile at a time (see `stepped`).
  const phone = useMotionValue(0);
  const count = React.Children.count(children);
  const x = useTransform([progress, travel, phone], ([p, t, ph]: number[]) =>
    ph ? -(stepped(p, count) / Math.max(1, count - 1)) * t : -p * t,
  );

  React.useEffect(() => {
    const section = ref.current;
    const track = railRef.current?.firstElementChild;
    if (!section || !track) return;
    const mq = window.matchMedia("(max-width: 1023.98px)");
    const measure = () => {
      phone.set(mq.matches ? 1 : 0);
      if (mq.matches && track.children.length > 1) {
        // Phone: exactly the distance that brings the last tile to the centre, and a little more
        // scroll per tile than it travels (x1.25), so each tile holds still for a moment.
        const a = track.children[0] as HTMLElement;
        const b = track.children[1] as HTMLElement;
        const t = (b.offsetLeft - a.offsetLeft) * (track.children.length - 1);
        travel.set(t);
        section.style.height = `calc(100vh + ${Math.round(t * 1.25)}px)`;
        return;
      }
      // How far the row sticks out past the viewport, with a little air at the end.
      const t = Math.max(0, track.scrollWidth - window.innerWidth + 32);
      travel.set(t);
      section.style.height = `calc(100vh + ${Math.round(t)}px)`;
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(track);
    window.addEventListener("resize", measure);
    mq.addEventListener("change", measure);
    // The phone's position dots follow the tile in view, as one CSS number (`--lk-f`).
    const dots = dotsRef.current;
    const paint = () => dots?.style.setProperty("--lk-f", String(stepped(progress.get(), count)));
    paint();
    const off = progress.on("change", paint);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", measure);
      mq.removeEventListener("change", measure);
      off();
    };
  }, [travel, phone, progress, count]);

  return (
    <section
      ref={ref}
      id="dashboards"
      className="relative h-[240vh] motion-reduce:h-auto!"
      style={{ height: "calc(100vh + 0px)" }}
    >
      {/* WEB-5 M2 (phone pass): below 1024 px the stage sits under the 64 px bar and its content
          (heading, one tile, the dots) is one block centred in the screen that is left, a little
          above the middle, so there is no band of empty water above or below the rail. */}
      <div className="lk-fallback-bg sticky top-0 flex h-[100svh] flex-col justify-center overflow-hidden bg-[var(--lk-n-bg)] max-lg:pt-16 max-lg:pb-[8svh] motion-reduce:static motion-reduce:h-auto motion-reduce:py-28 max-lg:motion-reduce:py-20">
        {before}

        {/* The rail. Pinned: transformed by scroll. Reduced motion: a native horizontal scroller.
            Phone: one tile wide (`--lk-tw`), centred, the next one stepping in; under reduced
            motion it is a swipe row that snaps each tile to the centre. */}
        <div
          ref={railRef}
          className="relative z-10 mt-8 md:mt-12 motion-reduce:overflow-x-auto motion-reduce:pb-4 max-lg:mt-7 max-lg:[scrollbar-width:none] max-lg:motion-reduce:snap-x max-lg:motion-reduce:snap-mandatory max-lg:[&::-webkit-scrollbar]:hidden"
        >
          <m.ul
            className="flex w-max gap-4 motion-reduce:transform-none! pr-4 pl-[max(1rem,calc((100vw-1120px)/2+1.5rem))] max-lg:gap-3 max-lg:px-[calc((100vw-var(--lk-tw))/2)] max-lg:[--lk-tw:min(calc(100vw-2rem),30rem)] max-lg:motion-reduce:[--lk-tw:min(calc(100vw-3.5rem),30rem)] md:gap-5"
            style={{ x }}
          >
            {children}
          </m.ul>
        </div>
        {/* Phone: where you are in the row, one dot per tile; the one in view is long. */}
        <ol ref={dotsRef} aria-hidden="true" className="lk-dash-dots mt-6 flex justify-center gap-1.5 lg:hidden motion-reduce:hidden">
          {Array.from({ length: count }, (_, i) => (
            <li key={i} style={{ "--i": i } as React.CSSProperties} />
          ))}
        </ol>
        {after}
      </div>
    </section>
  );
}

/**
 * WEB-5 M2 (phone pass): the phone rail's position, in tiles (0 to `n - 1`), for a scroll progress
 * `p`. Each tile holds still, centred, for the first and last quarter of its share of the scroll and
 * eases to the next one in between, so one tile is in view at a time rather than two half tiles.
 */
function stepped(p: number, n: number): number {
  if (n < 2) return 0;
  const f = Math.min(n - 1, Math.max(0, p) * (n - 1));
  const i = Math.min(n - 2, Math.floor(f));
  const k = Math.min(1, Math.max(0, (f - i - 0.25) / 0.5));
  return i + k * k * (3 - 2 * k);
}
