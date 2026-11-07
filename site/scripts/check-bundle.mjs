// WEB-5 D5: the JavaScript a visitor downloads before the 3D scene, measured
// from the build output, against the spec's budget (docs/core/specs/web-5.md:
// "JS loaded before the 3D scene (all initial chunks, gzip) ≤ 200 KB").
//
// "Initial" means every script a page's HTML loads by itself: the
// `<script src>` tags and the script preloads in `out/index.html` and
// `out/id/index.html`. A `<script noModule>` is not counted: it is Next's
// legacy polyfill bundle, which every browser that runs modules (all the
// spec's targets, and Lighthouse's emulated Chrome) skips without
// downloading; it is listed so the number stays visible. The scene's
// chunks are not in the HTML (they are a dynamic import that starts after
// the page is usable), so they are reported separately and do not count. The check fails:
//   - when a page's initial JS is over the budget;
//   - when an initial chunk contains three.js, because then the scene is no
//     longer deferred and the number above would not mean what it says;
//   - when no scene chunk is found, because then the detection is broken and
//     the scene's size would silently go unreported.
//
// The budget is 200 KB as 200,000 bytes (not 204,800): the stricter reading.
// Run after `next build`: `bun run size`.

import { readFileSync, readdirSync, existsSync } from "node:fs";
import { join } from "node:path";
import { gzipSync } from "node:zlib";

const OUT = new URL("../out/", import.meta.url).pathname;
const BUDGET = 200_000;
const PAGES = ["index.html", "id/index.html"];
// Strings three.js keeps after minification, and one of the scene's own
// shader uniforms, so both the library chunk and the engine chunk are found.
const SCENE_MARKERS = ["WebGLRenderer", "uSkyHorizon"];

if (!existsSync(OUT)) {
  console.error("check-bundle: out/ does not exist; run `bun run build` first.");
  process.exit(1);
}

const gz = (file) => gzipSync(readFileSync(join(OUT, file))).length;
const kb = (n) => `${(n / 1000).toFixed(1)} KB`;
const isScene = (file) => {
  const text = readFileSync(join(OUT, file), "utf8");
  return SCENE_MARKERS.some((m) => text.includes(m));
};

function initialScripts(page) {
  const html = readFileSync(join(OUT, page), "utf8");
  const files = new Set();
  for (const m of html.matchAll(/<script[^>]*\ssrc="([^"]+\.js)[^"]*"[^>]*>/g)) {
    if (/\snoModule/i.test(m[0])) skipped.add(m[1].replace(/^\//, ""));
    else files.add(m[1]);
  }
  for (const m of html.matchAll(/<link[^>]*rel="(?:modulepreload|preload)"[^>]*>/g)) {
    const tag = m[0];
    if (/\sas="script"/.test(tag) || /modulepreload/.test(tag)) {
      const href = /href="([^"]+\.js)[^"]*"/.exec(tag);
      if (href) files.add(href[1]);
    }
  }
  return [...files].map((f) => f.replace(/^\//, "")).sort();
}

const skipped = new Set();
let failed = false;
const allInitial = new Set();
for (const page of PAGES) {
  const files = initialScripts(page);
  files.forEach((f) => allInitial.add(f));
  const total = files.reduce((sum, f) => sum + gz(f), 0);
  const ok = total <= BUDGET;
  console.log(`${page}: initial JS ${kb(total)} gzip in ${files.length} files (budget ${kb(BUDGET)}) ${ok ? "OK" : "OVER"}`);
  if (!ok) failed = true;
  for (const f of files) {
    if (isScene(f)) {
      console.error(`  ${f} is loaded by the HTML and contains the 3D scene: the scene is not deferred.`);
      failed = true;
    }
  }
}

for (const f of skipped) console.log(`not counted (noModule, legacy browsers only): ${f} ${kb(gz(f))}`);

const chunkDir = join(OUT, "_next/static/chunks");
const scene = readdirSync(chunkDir, { recursive: true })
  .filter((f) => String(f).endsWith(".js"))
  .map((f) => join("_next/static/chunks", String(f)))
  .filter((f) => !allInitial.has(f) && !skipped.has(f) && isScene(f));
if (scene.length === 0) {
  console.error("No scene chunk found: the markers no longer match the build, so its size is unknown.");
  failed = true;
} else {
  const total = scene.reduce((sum, f) => sum + gz(f), 0);
  console.log(`scene (deferred, not counted): ${kb(total)} gzip in ${scene.length} files`);
  for (const f of scene) console.log(`  ${f} ${kb(gz(f))}`);
}

// WEB-5 D14 S1: the test-only WebGL override must never reach the export.
const leaked = readdirSync(OUT, { recursive: true })
  .map(String)
  .filter((f) => /\.(js|html|txt)$/.test(f))
  .filter((f) => readFileSync(join(OUT, f), "utf8").includes("WEB5_FORCE_WEBGL"));
if (leaked.length) {
  console.error(`The test-only WebGL override is in the export: ${leaked.join(", ")}`);
  failed = true;
} else {
  console.log("test-only WebGL override: not in the export");
}

process.exit(failed ? 1 : 0);
