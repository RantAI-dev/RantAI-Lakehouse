import { strict as assert } from "node:assert";
import { test } from "node:test";
import { maskEmail, maskName, scrambleFrame } from "./scramble";

test("maskEmail keeps the first letter and the top-level domain only", () => {
  assert.equal(maskEmail("mira.okafor@example.com"), "m•••@•••.com");
  assert.equal(maskEmail("aiko.tanaka@example.org"), "a•••@•••.org");
});

test("maskEmail masks a value that is not an email entirely", () => {
  assert.equal(maskEmail("no-at-sign"), "•••");
});

test("maskName keeps one initial per word", () => {
  assert.equal(maskName("Mira Okafor"), "M••• O•••");
  assert.equal(maskName("Noor"), "N•••");
});

test("scrambleFrame is the old value at 0 and the new value at 1", () => {
  const rand = () => 0.5;
  assert.equal(scrambleFrame("Mira Okafor", "M••• O•••", 0, rand), "Mira Okafor");
  assert.equal(scrambleFrame("Mira Okafor", "M••• O•••", 1, rand), "M••• O•••");
});

test("scrambleFrame keeps separators in place while the rest cycles", () => {
  const frame = scrambleFrame("mira.okafor@example.com", "m•••@•••.com", 0.3, () => 0.1);
  assert.ok(frame.includes("@"), `lost the @ in ${frame}`);
  assert.ok(frame.includes("."), `lost the . in ${frame}`);
  assert.ok(/[a-z0-9#$%&*+=?]/.test(frame), "expected cycling glyphs");
});

test("scrambleFrame resolves from the left: early characters are already final", () => {
  const frame = scrambleFrame("abcdefghij", "0123456789", 0.5, () => 0.99);
  assert.equal(frame.slice(0, 5), "01234");
});

test("scrambleFrame length moves between the two lengths", () => {
  for (const p of [0.1, 0.5, 0.9]) {
    const n = scrambleFrame("aaaaaaaaaaaa", "bbbb", p, () => 0.2).length;
    assert.ok(n >= 4 && n <= 12, `length ${n} at ${p}`);
  }
});
