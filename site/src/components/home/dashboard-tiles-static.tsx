import * as React from "react";
import { TILE_FILES, TILE_SIZE } from "@/lib/tile-svg";
import type { TileText } from "./dashboard-tiles";

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

/** A dot-matrix map: a landmass-like blob of dots whose size and brightness stand for a value per region. */
export function DotMapTile({ tile }: { tile: TileText }) {
  return <TileImage src={TILE_FILES.map.path} alt={tile.aria} />;
}

/** Twenty weeks of days; weekends are quieter, and a few busy weeks stand out. */
export function CalendarTile({ tile }: { tile: TileText }) {
  return <TileImage src={TILE_FILES.calendar.path} alt={tile.aria} />;
}
