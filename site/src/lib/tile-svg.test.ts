import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { calendarSvg, dotMapSvg, TILE_FILES } from "./tile-svg";

const publicDir = join(import.meta.dir, "../../public");

test("the committed tile files are exactly what the drawing code produces (run `bun run tiles`)", () => {
  for (const { path, render } of Object.values(TILE_FILES)) {
    assert.equal(readFileSync(join(publicDir, path), "utf8"), render(), path);
  }
});

test("the tiles carry the same number of shapes as the inline tiles did (280 dots, 140 days)", () => {
  assert.equal(dotMapSvg().match(/<circle /g)?.length, 280);
  assert.equal(calendarSvg().match(/<rect /g)?.length, 140);
});

test("each tile is a standalone SVG document with the inline tiles' viewBox", () => {
  for (const svg of [dotMapSvg(), calendarSvg()]) {
    assert.ok(svg.startsWith('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 360 190"'), svg.slice(0, 80));
  }
});
