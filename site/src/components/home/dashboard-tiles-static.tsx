import * as React from "react";
import type { HomeCopy } from "@/lib/copy";
import { mapZones, TILE_FILES, TILE_SIZE } from "@/lib/tile-svg";
import { CalendarHoverHost } from "./calendar-hover-host";

type Tiles = HomeCopy["dashboards"]["tiles"];

/**
 * The two dashboard tiles that do not animate, the dot map and the calendar
 * heatmap, as one `<img>` each (WEB-5 review fix R6). Inline they were
 * about 420 SVG elements that React walks during hydration; the drawing is
 * now `public/tiles/*.svg`, generated from `lib/tile-svg.ts`, the same
 * shapes as before (D6). The tile's accessible name is the image's `alt`,
 * and the explicit width and height keep the box the size the inline SVG's
 * viewBox gave it, so nothing shifts as the image arrives.
 *
 * A plain `<img>`, not `next/image`: the export serves images unoptimized
 * anyway, and `next/image` is a client component, which would bring back
 * the hydration this change removes.
 */
function TileImage({ src, alt }: { src: string; alt: string }) {
  // eslint-disable-next-line @next/next/no-img-element -- see above: next/image would be hydrated, and the export does not optimize images
  return <img src={src} alt={alt} width={TILE_SIZE.width} height={TILE_SIZE.height} className="h-auto w-full" />;
}

/**
 * WEB-5 owner QA O10: the two pictures move without becoming DOM again. The
 * entrance is a mask over the image (a wave across the map, a diagonal
 * ripple over the calendar), and the idle light is a layer masked by the
 * same SVG file, so it only ever lights the dots or the cells
 * (app/globals.css, "the living tiles").
 */

/** Region letters, in reading order of the 4 × 3 grid. */
const REGION_NAMES = "ABCDEFGHIJKL";

/**
 * A dot-matrix map: a landmass-like blob of dots whose size and brightness
 * stand for a value per region. WEB-5 O18: a 4 × 3 grid of hover regions
 * over the picture (only those that hold dots, `mapZones`); the region
 * under the pointer shows its dots lit, the rest dim, and a tooltip names
 * it with its value. CSS only (".lk-zone" in app/globals.css).
 */
export function DotMapTile({ tile }: { tile: Tiles["map"] }) {
  return (
    <div className="lk-map relative">
      <TileImage src={TILE_FILES.map.path} alt={tile.aria} />
      {/* The highlighted region (the right lobe) settling, then a light sweep now and then. */}
      <span aria-hidden="true" className="lk-map-glow absolute inset-0" />
      <span aria-hidden="true" className="lk-map-sweep absolute inset-0" />
      {mapZones().map((z) => (
        <span
          key={`${z.col}${z.row}`}
          aria-hidden="true"
          className="lk-zone absolute"
          style={{
            left: `${z.col * 25}%`,
            top: `${(z.row * 100) / 3}%`,
            width: "25%",
            height: `${100 / 3}%`,
            backgroundPosition: `${(z.col * 100) / 3}% ${z.row * 50}%`,
          }}
        >
          <span className="lk-tip">
            {tile.region} {REGION_NAMES[z.row * 4 + z.col]} · {z.value.toFixed(1)}k
          </span>
        </span>
      ))}
    </div>
  );
}

/**
 * Twenty weeks of days; weekends are quieter, and a few busy weeks stand
 * out. WEB-5 O18: its 140 cells take their hover from a small handler that
 * loads on the first pointer over the tile (`calendar-hover-host.tsx`), so
 * neither 140 hover elements nor the handler weigh on the first load.
 */
export function CalendarTile({ tile }: { tile: Tiles["calendar"] }) {
  return (
    <div className="lk-cal relative">
      <TileImage src={TILE_FILES.calendar.path} alt={tile.aria} />
      {/* Now and then one day glows, as if it just filled up. */}
      <span aria-hidden="true" className="lk-cal-glow absolute inset-0" />
      <CalendarHoverHost days={tile.days} week={tile.week} unit={tile.unit} />
    </div>
  );
}
