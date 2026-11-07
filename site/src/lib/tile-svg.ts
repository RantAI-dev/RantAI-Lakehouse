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

// Task B: the tiles are drawn for the light panel: strong cells in the primary blue, quiet ones in grey.
const QUIET = "#6b7680";
const STRONG = "#1d6ccf";

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

/**
 * Audit #23: the dot map is Indonesia, so it reads as a map. The outline is
 * this project's own drawing, not traced from any map file: each island is a
 * rough polygon in longitude and latitude (about a degree of detail),
 * projected onto the tile (95°E to 141°E, 6°N to 11°S). Each island has an
 * illustrative value (visits, in thousands); its dots' size and brightness
 * follow it.
 */
type Island = { name: string; value: number; shape: readonly (readonly [number, number])[][] };
const ISLANDS: readonly Island[] = [
  {
    name: "Sumatra",
    value: 6.4,
    shape: [[[95.3, 5.6], [97.5, 5.2], [100.3, 2.5], [103.8, 1.0], [104.6, -1.5], [106.0, -3.2], [105.8, -5.8], [104.5, -5.9], [102.3, -4.0], [100.4, -1.2], [98.7, 1.7], [96.2, 3.7]]],
  },
  {
    name: "Java",
    value: 9.8,
    shape: [[[105.2, -6.8], [106.4, -5.9], [108.3, -6.2], [110.4, -6.9], [112.6, -6.9], [114.4, -7.7], [114.4, -8.6], [111.5, -8.3], [108.5, -7.8], [106.4, -7.4]]],
  },
  {
    name: "Kalimantan",
    value: 3.1,
    shape: [[[108.8, 1.5], [109.6, 2.0], [111.5, 1.0], [113.0, 1.4], [114.5, 1.5], [115.5, 3.0], [116.0, 4.1], [117.7, 4.2], [118.0, 2.0], [117.5, 0.5], [117.0, -1.0], [116.5, -2.5], [116.2, -4.0], [114.5, -3.6], [113.0, -3.2], [111.5, -3.0], [110.2, -2.9], [109.6, -1.0], [109.0, 0.5]]],
  },
  {
    name: "Sulawesi",
    value: 4.2,
    shape: [[[120.0, 0.8], [121.5, 1.2], [123.0, 0.9], [124.9, 1.6], [125.2, 1.4], [123.3, 0.3], [121.5, 0.5], [120.6, -0.3], [121.5, -1.2], [123.4, -0.9], [122.8, -1.6], [121.4, -1.9], [122.9, -4.6], [122.0, -5.4], [121.0, -4.0], [120.6, -2.8], [120.3, -5.6], [119.4, -5.4], [119.6, -3.5], [118.8, -2.7], [119.5, -0.8]]],
  },
  {
    name: "Bali & Nusa Tenggara",
    value: 2.6,
    shape: [
      [[114.5, -8.0], [115.7, -8.3], [115.2, -8.9], [114.5, -8.5]],
      [[116.0, -8.2], [119.0, -8.1], [119.0, -8.9], [116.1, -9.0]],
      [[119.8, -8.3], [122.9, -8.1], [122.8, -8.8], [119.9, -9.0]],
      [[119.0, -9.4], [120.8, -9.5], [120.5, -10.3], [119.0, -9.9]],
      [[123.5, -10.2], [125.2, -9.0], [127.0, -8.4], [125.0, -9.7], [123.8, -10.5]],
    ],
  },
  {
    name: "Maluku",
    value: 1.4,
    shape: [
      [[127.4, 1.9], [128.4, 1.5], [128.0, 0.5], [128.7, -0.4], [127.9, -0.8], [127.4, 0.3]],
      [[128.0, -2.9], [130.8, -2.9], [130.6, -3.7], [128.2, -3.7]],
      [[125.9, -3.1], [127.2, -3.1], [127.0, -3.9], [126.0, -3.8]],
    ],
  },
  {
    name: "Papua",
    value: 1.9,
    shape: [[[131.0, -1.0], [132.5, -0.4], [134.2, -0.9], [135.2, -3.3], [137.5, -1.5], [140.0, -2.3], [141.0, -2.6], [141.0, -9.1], [139.8, -8.1], [138.2, -8.4], [137.8, -7.3], [138.8, -6.9], [138.0, -5.3], [135.2, -4.4], [133.5, -4.0], [132.8, -3.0], [131.6, -2.6], [132.0, -2.0]]],
  },
];
const MAX_VALUE = Math.max(...ISLANDS.map((i) => i.value));
const project = ([lon, lat]: readonly [number, number]) => [10 + ((lon - 95) / 46) * 340, 10 + ((6 - lat) / 17) * 170] as const;
const PROJECTED = ISLANDS.map((i) => i.shape.map((ring) => ring.map(project)));

function inside(x: number, y: number, ring: readonly (readonly [number, number])[]): boolean {
  let hit = false;
  for (let i = 0, j = ring.length - 1; i < ring.length; j = i++) {
    const [xi, yi] = ring[i];
    const [xj, yj] = ring[j];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) hit = !hit;
  }
  return hit;
}

/** Every dot of the map: its place, and the island it belongs to. */
function mapDots(): { x: number; y: number; island: number }[] {
  const dots: { x: number; y: number; island: number }[] = [];
  for (let r = 0; r < 30; r++) {
    for (let c = 0; c < 58; c++) {
      const x = 8 + c * 6;
      const y = 8 + r * 6;
      const island = PROJECTED.findIndex((rings) => rings.some((ring) => inside(x, y, ring)));
      if (island >= 0) dots.push({ x, y, island });
    }
  }
  return dots;
}

/** A dot-matrix map of Indonesia: each island's dots sized and lit by its value. */
export function dotMapSvg(): string {
  const shapes = mapDots().map(({ x, y, island }) => {
    const v = ISLANDS[island].value / MAX_VALUE;
    return `<circle cx="${x}" cy="${y}" r="${(1.15 + v * 1.25).toFixed(2)}" fill="${v > 0.6 ? STRONG : QUIET}" fill-opacity="${(0.35 + v * 0.6).toFixed(2)}"/>`;
  });
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
        `<rect x="${12 + c * 17}" y="${14 + r * 23}" width="13" height="17" rx="3.5" fill="${level > 0.55 ? STRONG : QUIET}" fill-opacity="${0.16 + level * 0.8}"/>`,
      );
    }
  }
  return `${open}${shapes.join("")}</svg>\n`;
}

/**
 * WEB-5 O18: the map's hover regions, a 4 × 3 grid over the tile; only
 * cells that hold dots are hoverable. Audit #23: each cell is named after
 * the island most of its dots belong to (its index in `ISLANDS`, whose
 * names the messages hold as `tiles.map.islands`), with that island's value.
 */
export function mapZones(): { col: number; row: number; island: number; value: number }[] {
  const cells = new Map<string, number[]>();
  for (const d of mapDots()) {
    const key = `${Math.min(3, Math.floor(d.x / 90))},${Math.min(2, Math.floor(d.y / (TILE_SIZE.height / 3)))}`;
    const counts = cells.get(key) ?? ISLANDS.map(() => 0);
    counts[d.island] += 1;
    cells.set(key, counts);
  }
  return [...cells.entries()]
    .filter(([, counts]) => Math.max(...counts) >= 6)
    .map(([key, counts]) => {
      const [col, row] = key.split(",").map(Number);
      const island = counts.indexOf(Math.max(...counts));
      return { col, row, island, value: ISLANDS[island].value };
    })
    .sort((a, b) => a.row - b.row || a.col - b.col);
}

/** WEB-5 O18: the calendar's cell value (0..1) at a column and row, the same as the drawing's. */
export function calendarLevel(c: number, r: number): number {
  const v = hash(c + 1, r + 11);
  const quiet = r >= 5 ? 0.45 : 1;
  return Math.min(1, v * quiet * (0.55 + 0.6 * hash(c, 2)));
}

/** Where each file is served from and written to (`public/` + path). */
export const TILE_FILES = {
  map: { path: "/tiles/dot-map.svg", render: dotMapSvg },
  calendar: { path: "/tiles/calendar-heatmap.svg", render: calendarSvg },
} as const;
