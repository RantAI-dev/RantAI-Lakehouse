"use client";

import * as React from "react";
import { motion, useMotionValue, useTransform } from "motion/react";
import { DASHBOARDS } from "./landing-copy";
import { CalendarTile, DotMapTile, KpiTile, LineChartTile, SankeyTile } from "./dashboard-tiles";
import { Container, DisplayHeading, LeadText } from "./landing-ui";
import { MascotLogo } from "./mascot-svg";
import { SplitWords } from "./reveal";
import { TiltCard } from "./tilt-card";
import { useSectionProgress } from "./use-section-progress";

/**
 * S5 — "Dashboards live on the shore": answers come back up as dashboards.
 *
 * A pinned stage whose row of five tiles slides sideways as the page scrolls
 * down: vertical scroll drives `x` one for one, so the section is exactly as
 * tall as the row is wide, plus one screen. The row's overflow is measured,
 * not guessed, so it follows the viewport and the font loading. The tiles are
 * SVG with made-up numbers and the section says so. With reduced motion the
 * stage is not pinned and the row is an ordinary, keyboard-scrollable strip.
 */

const TILE_VISUALS = {
  line: LineChartTile,
  map: DotMapTile,
  sankey: SankeyTile,
  kpi: KpiTile,
  calendar: CalendarTile,
} as const;

export function DashboardsSection() {
  const ref = React.useRef<HTMLElement>(null);
  const trackRef = React.useRef<HTMLUListElement>(null);
  const progress = useSectionProgress(ref, ["start start", "end end"]);
  const travel = useMotionValue(0);
  const x = useTransform([progress, travel], ([p, t]: number[]) => -p * t);
  const height = useTransform(travel, (t) => `calc(100vh + ${Math.round(t)}px)`);

  React.useEffect(() => {
    const track = trackRef.current;
    if (!track) return;
    const measure = () => {
      // How far the row sticks out past the viewport, with a little air at the end.
      travel.set(Math.max(0, track.scrollWidth - window.innerWidth + 32));
    };
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(track);
    window.addEventListener("resize", measure);
    return () => {
      ro.disconnect();
      window.removeEventListener("resize", measure);
    };
  }, [travel]);

  return (
    <motion.section
      ref={ref}
      id="dashboards"
      className="relative h-[240vh] motion-reduce:h-auto!"
      style={{ height }}
    >
      <div className="sticky top-0 flex h-[100svh] flex-col justify-center overflow-hidden bg-[linear-gradient(180deg,#050A30_0%,#09154a_45%,#0B1A4A_75%,#12307a_100%)] motion-reduce:static motion-reduce:h-auto motion-reduce:py-28">
        <Shore />
        <Container className="relative z-10">
          <DisplayHeading size="lg" className="max-w-[16ch] max-md:text-[clamp(2.4rem,11vw,3.2rem)]">
            <SplitWords text={DASHBOARDS.headline} />
          </DisplayHeading>
          <LeadText className="mt-4 max-w-[38rem] max-md:text-[15px] max-md:leading-[1.5] md:mt-5">
            {DASHBOARDS.sub}
          </LeadText>
        </Container>

        {/* The rail. Pinned: transformed by scroll. Reduced motion: a native horizontal scroller. */}
        <div className="relative z-10 mt-8 md:mt-12 motion-reduce:overflow-x-auto motion-reduce:pb-4">
          <motion.ul
            ref={trackRef}
            className="flex w-max gap-4 motion-reduce:transform-none! pr-4 pl-[max(1rem,calc((100vw-1120px)/2+1.5rem))] md:gap-5"
            style={{ x }}
          >
            {DASHBOARDS.tiles.map((tile) => {
              const Visual = TILE_VISUALS[tile.id];
              return (
                <li key={tile.id} className="w-[78vw] shrink-0 sm:w-[380px] md:w-[420px]">
                  <TiltCard className="rounded-3xl border border-[#E8F4FF]/14 bg-[#E8F4FF]/[0.05] p-5 shadow-[0_30px_80px_-40px_rgba(2,5,20,0.9)] backdrop-blur-md md:p-6">
                    <p className="lk-mono text-[11px] tracking-[0.2em] text-[#44A6FA] uppercase">{tile.kind}</p>
                    <p className="mt-1.5 text-[15px] text-[#E8F4FF]">{tile.title}</p>
                    <div className="mt-4">
                      <Visual />
                    </div>
                  </TiltCard>
                </li>
              );
            })}
          </motion.ul>
        </div>
        <Container className="relative z-10 mt-5">
          <p className="lk-mono text-[12px] text-[#E8F4FF]/65">{DASHBOARDS.caption}</p>
        </Container>
      </div>
    </motion.section>
  );
}

/**
 * The shore: a thin lake-blue horizon along the bottom, and the creature,
 * tiny, sitting on it at the far right.
 */
function Shore() {
  return (
    <div aria-hidden="true" className="pointer-events-none absolute inset-x-0 bottom-0 h-[22%]">
      <div className="absolute inset-x-0 bottom-[34%] h-px bg-gradient-to-r from-transparent via-[#44A6FA]/70 to-[#44A6FA]/70" />
      <div className="absolute inset-x-0 bottom-[34%] h-24 bg-gradient-to-t from-transparent to-[#44A6FA]/10" />
      <div className="absolute inset-x-0 bottom-0 h-[34%] bg-gradient-to-b from-[#44A6FA]/10 to-[#050A30]/40" />
      <MascotLogo className="absolute right-[5%] bottom-[34%] w-9 translate-y-[2px] opacity-90 md:w-12" />
    </div>
  );
}
