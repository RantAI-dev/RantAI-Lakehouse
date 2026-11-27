import { strict as assert } from "node:assert";
import { test } from "node:test";
import en from "../messages/en/home.json";
import id from "../messages/id/home.json";
import { BRAND_MARKS, isBrand } from "../components/shared/brand-icons";
import { SOURCE_ICONS } from "./home-content";

test("every source pill has a mark, in both languages (O20)", () => {
  assert.equal(SOURCE_ICONS.length, en.sources.items.length);
  assert.equal(SOURCE_ICONS.length, id.sources.items.length);
});

test("brand marks are Simple Icons paths with a readable hover colour", () => {
  for (const icon of SOURCE_ICONS) {
    if (!isBrand(icon)) continue;
    const m = BRAND_MARKS[icon];
    assert.match(m.path, /^M/, icon);
    assert.match(m.hover, /^#[0-9A-F]{6}$/i, icon);
  }
});
