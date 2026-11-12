// WEB-5 review fix R6: writes the two static dashboard tiles to public/tiles/
// from src/lib/tile-svg.ts. Run `bun run tiles` after changing that drawing;
// src/lib/tile-svg.test.ts fails while the committed files differ from it.
import { mkdirSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { TILE_FILES } from "../src/lib/tile-svg";

const publicDir = join(import.meta.dir, "../public");
for (const { path, render } of Object.values(TILE_FILES)) {
  const file = join(publicDir, path);
  mkdirSync(dirname(file), { recursive: true });
  writeFileSync(file, render());
  console.log(`wrote public${path}`);
}
