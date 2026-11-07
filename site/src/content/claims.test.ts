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

test("every FAQ answer is a claim with a backlog trace (D15)", () => {
  for (const locale of routing.locales) {
    const items = resolve(load(locale), "contact.faq.items") as { q: string; a: string }[];
    assert.ok(Array.isArray(items) && items.length >= 6 && items.length <= 8, `${locale}: 6 to 8 questions`);
    items.forEach((_, i) => {
      const key = `contact.faq.items.${i}.a`;
      assert.ok(CLAIMS.some((c) => c.key === key), `${key} has no entry in claims.ts`);
    });
  }
});

test("a claim's backlog rows are ones the page may claim: Released, Done or In Acceptance (D7)", () => {
  for (const c of CLAIMS) {
    // BI-16 is Building; its merged part C2 alone is claimed, and the row says so.
    for (const b of c.backlog.filter((x) => !x.startsWith("BI-16 "))) {
      assert.match(b, /^[A-Z]+-\d+ (Released|Done|In Acceptance)\b/, `${c.key}: ${b}`);
    }
  }
});

test("every claimed backlog row still has a status the page may claim in docs/core/BACKLOG.md", () => {
  // The repository's backlog is the source (D7). Read straight from it, so a
  // row that is reopened or killed later fails here instead of staying claimed.
  const backlog = readFileSync(join(import.meta.dir, "../../../docs/core/BACKLOG.md"), "utf8");
  const status = new Map<string, string>();
  for (const line of backlog.split("\n")) {
    const cells = line.split("|").map((c) => c.trim());
    const id = /^`([A-Z]+-\d+)`$/.exec(cells[1] ?? "")?.[1];
    // The item table's rows: ID, kind, item, module, priority, status, PR, source.
    if (id && cells.length >= 9 && /^(Feature|Task|Decision)$/.test(cells[2]) && !status.has(id)) status.set(id, cells[6]);
  }
  for (const c of CLAIMS) {
    for (const b of c.backlog.filter((x) => !x.startsWith("BI-16 "))) {
      const id = b.split(" ")[0];
      assert.match(status.get(id) ?? "missing", /^(Released|Done|In Acceptance)$/, `${c.key}: ${id} in BACKLOG.md`);
    }
  }
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
