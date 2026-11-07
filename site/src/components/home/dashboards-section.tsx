import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { TILE_IDS } from "@/lib/home-content";
import { KpiTile, LineChartTile, SankeyTile } from "./dashboard-tiles";
import { CalendarTile, DotMapTile } from "./dashboard-tiles-static";
import { DashboardsScroller } from "./dashboards-scroller";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { MascotLogo } from "@/components/shared/mascot-svg";
import { SplitWords } from "@/components/shared/reveal";
import { TiltCard } from "@/components/shared/tilt-card";

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

export function DashboardsSection({ copy }: { copy: HomeCopy["dashboards"] }) {
  return (
    <DashboardsScroller
      before={
        <>
          <Shore />
          <Container className="relative z-10">
            <DisplayHeading size="lg" className="max-w-[16ch] max-md:text-[clamp(2.4rem,11vw,3.2rem)]">
              <SplitWords text={copy.headline} />
            </DisplayHeading>
            <LeadText className="mt-4 max-w-[38rem] max-md:text-[15px] max-md:leading-[1.5] md:mt-5">
              {copy.sub}
            </LeadText>
          </Container>
        </>
      }
      after={
        <Container className="relative z-10 mt-5">
          <p className="text-[12px] text-[#E8F4FF]/65">{copy.caption}</p>
        </Container>
      }
    >
      {TILE_IDS.map((id) => {
        const Visual = TILE_VISUALS[id];
        const tile = copy.tiles[id];
        return (
          <li key={id} className="w-[78vw] shrink-0 sm:w-[380px] md:w-[420px]">
            <TiltCard className="rounded-3xl border border-[#E8F4FF]/14 bg-[#E8F4FF]/[0.05] p-5 shadow-[0_30px_80px_-40px_rgba(2,5,20,0.9)] backdrop-blur-md md:p-6">
              <p className="text-[11px] tracking-[0.2em] text-[#44A6FA] uppercase">{tile.kind}</p>
              <p className="mt-1.5 text-[15px] text-[#E8F4FF]">{tile.title}</p>
              <div className="mt-4">
                <Visual tile={tile} />
              </div>
            </TiltCard>
          </li>
        );
      })}
    </DashboardsScroller>
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
      {/* WEB-5 D14 S2: fades to nothing rather than to the old section colour, so the shore ends without a band on the page-wide water. */}
      <div className="absolute inset-x-0 bottom-0 h-[34%] bg-gradient-to-b from-[#44A6FA]/10 to-transparent" />
      <MascotLogo className="absolute right-[5%] bottom-[34%] w-9 translate-y-[2px] opacity-90 md:w-12" />
    </div>
  );
}
