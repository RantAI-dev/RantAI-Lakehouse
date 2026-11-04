import { strict as assert } from "node:assert";
import { readFileSync, readdirSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { routing } from "../i18n/routing";
import { CLAIMS, FORBIDDEN } from "./claims";

/** WEB-5 D7: every claim resolves in every language, and no forbidden claim is made. */

const messagesDir = join(import.meta.dir, "../messages");
const load = (locale: string): Record<string, unknown> =>
  Object.fromEntries(
    readdirSync(join(messagesDir, locale))
      .filter((f) => f.endsWith(".json"))
      .map((f) => [f.replace(/\.json$/, ""), JSON.parse(readFileSync(join(messagesDir, locale, f), "utf8"))]),
  );

/** The value at a dotted path, where a numeric segment indexes an array. */
export function resolve(root: unknown, key: string): unknown {
  return key.split(".").reduce<unknown>((node, part) => {
    if (node === undefined || node === null) return undefined;
    if (Array.isArray(node)) return /^\d+$/.test(part) ? node[Number(part)] : undefined;
    return typeof node === "object" ? (node as Record<string, unknown>)[part] : undefined;
  }, root);
}

function strings(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (Array.isArray(value)) return value.flatMap(strings);
  if (value && typeof value === "object") return Object.values(value).flatMap(strings);
  return [];
}

test("every claim key holds non-empty text in every language", () => {
  for (const locale of routing.locales) {
    const messages = load(locale);
    for (const c of CLAIMS) {
      const v = resolve(messages, c.key);
      assert.equal(typeof v, "string", `${locale}: ${c.key} is not a string`);
      assert.ok((v as string).trim(), `${locale}: ${c.key} is empty`);
    }
  }
});

test("every claim names at least one backlog row and no key is listed twice", () => {
  for (const c of CLAIMS) assert.ok(c.backlog.length > 0 && c.backlog.every((b) => /^[A-Z]+-\d+ /.test(b)), c.key);
  assert.equal(new Set(CLAIMS.map((c) => c.key)).size, CLAIMS.length);
});

test("no message in any language makes a claim from the do-not-add list", () => {
  for (const locale of routing.locales) {
    for (const s of strings(load(locale))) {
      for (const re of FORBIDDEN) assert.ok(!re.test(s), `${locale}: "${s}" matches ${re}`);
    }
  }
});

test("resolve follows objects and array indexes, and the forbidden patterns catch the phrases", () => {
  assert.equal(resolve({ a: { b: ["x", "y"] } }, "a.b.1"), "y");
  assert.equal(resolve({ a: {} }, "a.b.0"), undefined);
  assert.ok(FORBIDDEN.some((re) => re.test("Live change capture for MySQL and PostgreSQL")));
  assert.ok(FORBIDDEN.some((re) => re.test("Upload over SFTP")));
  assert.ok(FORBIDDEN.some((re) => re.test("every chart type you need")));
  assert.ok(FORBIDDEN.some((re) => re.test("Change capture langsung untuk MySQL")));
  assert.ok(FORBIDDEN.some((re) => re.test("semua jenis chart tersedia")));
  assert.ok(!FORBIDDEN.some((re) => re.test("Live change capture for PostgreSQL.")));
  assert.ok(!FORBIDDEN.some((re) => re.test("Change capture langsung untuk PostgreSQL.")));
});
