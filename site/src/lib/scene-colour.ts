/**
 * WEB-5 O5: the colour the 3D water shows at the top and the bottom of the
 * screen, handed from the lake engine to the page-wide water behind it
 * (`components/home/depth-backdrop.tsx`).
 *
 * Where the scene zone ends, the canvas fades out and the DOM water takes
 * over. If the two colours differ, the fade reads as the water changing
 * colour, and any edge between them as a band. So the engine publishes the
 * exact colours it puts on screen (its fog, through the same ACES tone
 * mapping and sRGB encoding the lens pass applies), and the backdrop starts
 * from them and blends to its own over a long stretch of scroll.
 *
 * Plain TypeScript with no three.js, so the page's own bundle can read it
 * without pulling in the scene chunk.
 */

export type SceneColours = { top: string; bottom: string };

let current: SceneColours | null = null;
const listeners = new Set<() => void>();

/** The engine calls this when the colours change, and with `null` when it stops. */
export function publishSceneColours(next: SceneColours | null): void {
  if (next === current || (next && current && next.top === current.top && next.bottom === current.bottom)) return;
  current = next;
  for (const l of listeners) l();
}

export function sceneColours(): SceneColours | null {
  return current;
}

export function onSceneColours(listener: () => void): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

const clamp01 = (x: number): number => Math.min(1, Math.max(0, x));

/** sRGB encoding of one linear channel. */
function encode(c: number): number {
  return c <= 0.0031308 ? c * 12.92 : 1.055 * Math.pow(c, 1 / 2.4) - 0.055;
}

/**
 * three.js's `ACESFilmicToneMapping` (r187, `tonemapping_pars_fragment`),
 * then sRGB encoding, as a `#rrggbb` string: what a linear colour becomes
 * on screen through the lens pass.
 */
export function displayedHex(r: number, g: number, b: number, exposure = 1): string {
  const k = exposure / 0.6;
  const x = [r * k, g * k, b * k];
  // ACESInputMat (column-major in GLSL: these are its rows).
  const i = [
    0.59719 * x[0] + 0.35458 * x[1] + 0.04823 * x[2],
    0.076 * x[0] + 0.90834 * x[1] + 0.01566 * x[2],
    0.0284 * x[0] + 0.13383 * x[1] + 0.83777 * x[2],
  ];
  const fit = i.map((v) => (v * (v + 0.0245786) - 0.000090537) / (v * (0.983729 * v + 0.432951) + 0.238081));
  const o = [
    1.60475 * fit[0] - 0.53108 * fit[1] - 0.07367 * fit[2],
    -0.10208 * fit[0] + 1.10813 * fit[1] - 0.00605 * fit[2],
    -0.00327 * fit[0] - 0.07276 * fit[1] + 1.07602 * fit[2],
  ];
  return `#${o
    .map((v) =>
      Math.round(encode(clamp01(v)) * 255)
        .toString(16)
        .padStart(2, "0"),
    )
    .join("")}`;
}
