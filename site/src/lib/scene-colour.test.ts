import { strict as assert } from "node:assert";
import { test } from "node:test";
import { displayedHex, onSceneColours, publishSceneColours, sceneColours } from "./scene-colour";

test("tone mapping keeps black black and takes bright white to near white", () => {
  assert.equal(displayedHex(0, 0, 0), "#000000");
  // The creature's eyes are set to 6 in linear light for this reason (mascot.ts).
  const [r, g, b] = [1, 3, 5].map((i) => parseInt(displayedHex(6, 6, 6).slice(i, i + 2), 16));
  assert.ok(r >= 250 && g >= 250 && b >= 250, displayedHex(6, 6, 6));
});

test("tone mapping is monotone in brightness", () => {
  let prev = -1;
  for (let v = 0; v <= 2; v += 0.05) {
    const g = parseInt(displayedHex(v, v, v).slice(3, 5), 16);
    assert.ok(g >= prev, `${v}`);
    prev = g;
  }
});

test("listeners hear a change once, and not a repeat of the same colours", () => {
  let calls = 0;
  const off = onSceneColours(() => calls++);
  publishSceneColours({ top: "#010203", bottom: "#040506" });
  publishSceneColours({ top: "#010203", bottom: "#040506" });
  assert.equal(calls, 1);
  assert.deepEqual(sceneColours(), { top: "#010203", bottom: "#040506" });
  publishSceneColours(null);
  assert.equal(calls, 2);
  off();
  publishSceneColours({ top: "#000000", bottom: "#000000" });
  assert.equal(calls, 2);
  publishSceneColours(null);
});
