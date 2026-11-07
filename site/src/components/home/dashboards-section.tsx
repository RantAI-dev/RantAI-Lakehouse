import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { TILE_IDS } from "@/lib/home-content";
import { KpiTile, LineChartTile, SankeyTile } from "./dashboard-tiles";
import { CalendarTile, DotMapTile } from "./dashboard-tiles-static";
import { DashboardsScroller } from "./dashboards-scroller";
import { LiveTile } from "./live-tile";
import { Container, DisplayHeading, LeadText } from "@/components/shared/primitives";
import { MascotLogo } from "@/components/shared/mascot-svg";
import { SplitWords } from "@/components/shared/reveal";
import { TiltCard } from "@/components/shared/tilt-card";

/**
 * S5 — "Dashboards live on the shore": answers become dashboards (O33: one descent, so not "come back up").
 *
 * A pinned stage whose row of five tiles slides sideways as the page scrolls
 * down: vertical scroll drives `x` one for one, so the section is exactly as
 * tall as the row is wide, plus one screen. The row's overflow is measured,
 * not guessed, so it follows the viewport and the font loading. The tiles are
 * SVG with made-up numbers and the section says so. With reduced motion the
 * stage is not pinned and the row is an ordinary, keyboard-scrollable strip.
 */

/** Each tile's drawing, with its own words (O18: tooltips need each tile's labels). */
function TileVisual({ id, tiles }: { id: (typeof TILE_IDS)[number]; tiles: HomeCopy["dashboards"]["tiles"] }) {
  switch (id) {
    case "line":
      return <LineChartTile tile={tiles.line} />;
    case "map":
      return <DotMapTile tile={tiles.map} />;
    case "sankey":
      return <SankeyTile tile={tiles.sankey} />;
    case "kpi":
      return <KpiTile tile={tiles.kpi} />;
    case "calendar":
      return <CalendarTile tile={tiles.calendar} />;
  }
}

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
            {/* WEB-5 O10, "lots of visuals, little text": the features as chips, each one a claim. */}
            <ul className="mt-5 flex max-w-[52rem] flex-wrap gap-1.5 max-md:max-h-[4.6rem] max-md:overflow-hidden md:mt-6 md:gap-2">
              {copy.features.map((f) => (
                <li
                  key={f}
                  // O18: the site's chip states, a lighter border and a small lift.
                  className="rounded-full border border-[#E8F4FF]/14 bg-[#E8F4FF]/[0.05] px-3 py-1 text-[12px] leading-5 whitespace-nowrap text-[#E8F4FF]/80 transition-[border-color,background-color,color,translate] duration-200 ease-out hover:-translate-y-0.5 hover:border-[#E8F4FF]/35 hover:bg-[#E8F4FF]/[0.09] hover:text-[#E8F4FF] active:translate-y-0 motion-reduce:transition-none motion-reduce:hover:translate-y-0 md:text-[13px]"
                >
                  {f}
                </li>
              ))}
            </ul>
          </Container>
        </>
      }
      // O32: no visible "Illustrations, not customer data." line (the owner's decision); each
      // tile's accessible name still says it is illustrative.
      after={null}
    >
      {TILE_IDS.map((id) => {
        const tile = copy.tiles[id];
        return (
          <LiveTile
            key={id}
            label={`${tile.kind}: ${tile.title}`}
            // O18: a clear lift (6 px) with a deeper shadow on hover, focus or a tap; still under reduced motion.
            className="w-[78vw] shrink-0 rounded-3xl transition-transform duration-200 ease-out outline-none hover:-translate-y-1.5 focus-visible:-translate-y-1.5 active:-translate-y-1 motion-reduce:transition-none motion-reduce:hover:translate-y-0 motion-reduce:focus-visible:translate-y-0 sm:w-[380px] md:w-[420px]"
          >
            <TiltCard className="rounded-3xl border border-[#E8F4FF]/14 bg-[#E8F4FF]/[0.05] p-5 shadow-[0_30px_80px_-40px_rgba(2,5,20,0.9)] backdrop-blur-md transition-[border-color,box-shadow] duration-200 ease-out group-hover/tile:border-[#44A6FA]/60 group-hover/tile:shadow-[0_40px_90px_-36px_rgba(2,5,20,0.95),0_0_44px_-6px_rgba(68,166,250,0.55)] group-focus-visible/tile:border-[#44A6FA]/75 group-focus-visible/tile:shadow-[0_40px_90px_-36px_rgba(2,5,20,0.95),0_0_44px_-6px_rgba(68,166,250,0.6)] group-active/tile:border-[#44A6FA]/60 md:p-6">
              <p className="text-[11px] tracking-[0.2em] text-[#44A6FA] uppercase">{tile.kind}</p>
              <p className="mt-1.5 text-[15px] text-[#E8F4FF]/85 transition-colors duration-200 ease-out group-hover/tile:text-white group-focus-visible/tile:text-white">
                {tile.title}
              </p>
              <div className="mt-4">
                <TileVisual id={id} tiles={copy.tiles} />
              </div>
            </TiltCard>
          </LiveTile>
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
      <MascotLogo className="absolute right-[5%] bottom-[34%] h-auto w-9 translate-y-[2px] opacity-90 md:w-12" />
    </div>
  );
}
