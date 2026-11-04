import { strict as assert } from "node:assert";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { NAMESPACES } from "../i18n/namespaces";
import { routing } from "../i18n/routing";

/**
 * WEB-5 D3: every language has the same message files and the same keys,
 * and no value is empty, so a page never shows a missing-translation key
 * or a blank in one language. A port of the RantAI site's
 * `__tests__/i18n/messages.test.ts` to `bun test`.
 */

const dir = import.meta.dir;

/** Every leaf path, with array items as `[i]`. */
export function keys(value: unknown, prefix = ""): string[] {
  if (Array.isArray(value)) return value.flatMap((v, i) => keys(v, `${prefix}[${i}]`));
  if (value && typeof value === "object") {
    return Object.entries(value).flatMap(([k, v]) => keys(v, prefix ? `${prefix}.${k}` : k));
  }
  return [prefix];
}

/** Leaf paths whose string is empty or only spaces. */
export function emptyValues(value: unknown, prefix = ""): string[] {
  if (typeof value === "string") return value.trim() ? [] : [prefix];
  if (Array.isArray(value)) return value.flatMap((v, i) => emptyValues(v, `${prefix}[${i}]`));
  if (value && typeof value === "object") {
    return Object.entries(value).flatMap(([k, v]) => emptyValues(v, prefix ? `${prefix}.${k}` : k));
  }
  return [];
}

const read = (locale: string, file: string): unknown => JSON.parse(readFileSync(join(dir, locale, file), "utf8"));
const files = readdirSync(join(dir, routing.defaultLocale)).filter((f) => f.endsWith(".json")).sort();

test("every language has the same message files", () => {
  for (const locale of routing.locales) {
    const got = readdirSync(join(dir, locale)).filter((f) => f.endsWith(".json")).sort();
    assert.deepEqual(got, files, `${locale} has different message files`);
  }
});

test("every namespace the request config loads has a file, and every file is loaded", () => {
  assert.deepEqual([...NAMESPACES].map((n) => `${n}.json`).sort(), files);
});

for (const file of files) {
  test(`${file} has the same keys in every language`, () => {
    const base = keys(read(routing.defaultLocale, file)).sort();
    for (const locale of routing.locales) {
      const got = keys(read(locale, file)).sort();
      const missing = base.filter((k) => !got.includes(k));
      const extra = got.filter((k) => !base.includes(k));
      assert.deepEqual({ missing, extra }, { missing: [], extra: [] }, `${locale}/${file}`);
    }
  });

  test(`${file} has no empty values`, () => {
    for (const locale of routing.locales) {
      assert.deepEqual(emptyValues(read(locale, file)), [], `${locale}/${file}`);
    }
  });
}

test("keys and emptyValues see a deleted, an extra and an empty key", () => {
  const en = { a: "x", b: { c: "y", d: ["p", "q"] } };
  const id = { a: "x", b: { c: " ", e: "z", d: ["p"] } };
  const base = keys(en);
  const got = keys(id);
  assert.deepEqual(base.filter((k) => !got.includes(k)).sort(), ["b.d[1]"]);
  assert.deepEqual(got.filter((k) => !base.includes(k)), ["b.e"]);
  assert.deepEqual(emptyValues(id), ["b.c"]);
});
