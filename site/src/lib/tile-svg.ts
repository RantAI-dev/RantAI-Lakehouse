/**
 * The two static dashboard tiles, the dot map and the calendar heatmap, as
 * SVG documents (WEB-5 review fix R6).
 *
 * Inline, they were about 420 of the page's 1,088 elements, and React
 * walks every one of them during hydration, server-rendered or not
 * (measured in R5). As two image files they are two elements. This module
 * is the one drawing: `scripts/build-tile-svgs.ts` writes its output to
 * `public/tiles/`, and `tile-svg.test.ts` fails if the committed files
 * differ from it, so the files cannot drift from the code. The shapes,
 * numbers and attributes are the ones the inline tiles rendered (D6).
 */

const LAKE = "#44A6FA";
const FOAM = "#E8F4FF";

/**
 * Stable pseudo-random in 0..1 from two integers. Integer maths only, so the
 * output is the same on every engine.
 */
function hash(a: number, b: number): number {
  let h = Math.imul(a | 0, 374761393) ^ Math.imul(b | 0, 668265263);
  h = Math.imul(h ^ (h >>> 13), 1274126177);
  h ^= h >>> 16;
  return (h >>> 0) / 4294967296;
}

/** Same width and height as the inline tiles' viewBox. */
export const TILE_SIZE = { width: 360, height: 190 } as const;

const open = `<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 ${TILE_SIZE.width} ${TILE_SIZE.height}" width="${TILE_SIZE.width}" height="${TILE_SIZE.height}">`;

/** A dot-matrix map: a landmass-like blob of dots whose size and brightness stand for a value per region. */
export function dotMapSvg(): string {
  const shapes: string[] = [];
  const cols = 34;
  const rows = 15;
  for (let r = 0; r < rows; r++) {
    for (let c = 0; c < cols; c++) {
      const nx = (c / (cols - 1)) * 2 - 1;
      const ny = (r / (rows - 1)) * 2 - 1;
      // Two overlapping lobes, a gap between them, and a ragged edge.
      const lobe = Math.min(
        (nx + 0.42) ** 2 / 0.32 + (ny + 0.05) ** 2 / 0.62,
        (nx - 0.4) ** 2 / 0.3 + (ny - 0.12) ** 2 / 0.5,
      );
      const edge = (hash(c, r) - 0.5) * 0.35;
      if (lobe + edge < 1) {
        const region = Math.floor(((nx + 1) / 2) * 4) + Math.floor(((ny + 1) / 2) * 3) * 4;
        const v = 0.25 + hash(region, 3) * 0.75;
        shapes.push(
          `<circle cx="${14 + c * 10.1}" cy="${16 + r * 11.5}" r="${1.6 + v * 2.2}" fill="${v > 0.7 ? FOAM : LAKE}" fill-opacity="${0.25 + v * 0.7}"/>`,
        );
      }
    }
  }
  return `${open}${shapes.join("")}</svg>\n`;
}

/** Twenty weeks of days; weekends are quieter, and a few busy weeks stand out. */
export function calendarSvg(): string {
  const shapes: string[] = [];
  const cols = 20;
  for (let c = 0; c < cols; c++) {
    for (let r = 0; r < 7; r++) {
      const v = hash(c + 1, r + 11);
      const quiet = r >= 5 ? 0.45 : 1;
      const level = Math.min(1, v * quiet * (0.55 + 0.6 * hash(c, 2)));
      shapes.push(
        `<rect x="${12 + c * 17}" y="${14 + r * 23}" width="13" height="17" rx="3.5" fill="${level > 0.55 ? FOAM : LAKE}" fill-opacity="${0.12 + level * 0.8}"/>`,
      );
    }
  }
  return `${open}${shapes.join("")}</svg>\n`;
}

/** Where each file is served from and written to (`public/` + path). */
export const TILE_FILES = {
  map: { path: "/tiles/dot-map.svg", render: dotMapSvg },
  calendar: { path: "/tiles/calendar-heatmap.svg", render: calendarSvg },
} as const;
