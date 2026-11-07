import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { TILE_IDS } from "@/lib/home-content";
import { KpiTile, LineChartTile, SankeyTile } from "./dashboard-tiles";
import { CalendarTile, DotMapTile } from "./dashboard-tiles-static";
import { DashboardsScroller } from "./dashboards-scroller";
import { LiveTile } from "./live-tile";
import { Container, DisplayHeading } from "@/components/shared/primitives";
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
            <p className="lk-l-muted mt-4 max-w-[38rem] text-pretty text-[15px] leading-[1.5] md:mt-5 md:text-[19px] md:leading-[1.6]">
              {copy.sub}
            </p>
            {/* Audit #2 + #12: no feature pills here. The tiles already show the features, and the
                page has enough chip walls (sources, assistant, the footer card); on phones the pills'
                third row was also cut by the pinned track. */}
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
            <TiltCard
              tone="light"
              className="lk-l-card rounded-3xl border border-[#0f1419]/10 bg-white p-5 shadow-[0_1px_2px_rgba(15,20,25,0.04),0_14px_34px_-20px_rgba(15,20,25,0.2)] transition-[border-color,box-shadow] duration-200 ease-out group-hover/tile:border-[#1d6ccf]/45 group-hover/tile:shadow-[0_1px_2px_rgba(15,20,25,0.05),0_26px_50px_-22px_rgba(15,20,25,0.32)] group-focus-visible/tile:border-[#1d6ccf]/70 group-focus-visible/tile:shadow-[0_0_0_3px_rgba(29,108,207,0.18),0_26px_50px_-22px_rgba(15,20,25,0.3)] group-active/tile:border-[#1d6ccf]/45 md:p-6"
            >
              <p className="text-[11px] font-medium tracking-[0.2em] text-[#1d6ccf] uppercase">{tile.kind}</p>
              <p className="lk-l-muted mt-1.5 text-[15px] transition-colors duration-200 ease-out group-hover/tile:text-[#0b0d0e] group-focus-visible/tile:text-[#0b0d0e]">
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
      {/* O39: a solid hairline, with no glow above or below it. */}
      <div className="absolute inset-x-0 bottom-[34%] h-px bg-[#0f1419]/14" />
      <MascotLogo className="absolute right-[5%] bottom-[34%] h-auto w-9 translate-y-[2px] opacity-90 md:w-12" />
    </div>
  );
}
