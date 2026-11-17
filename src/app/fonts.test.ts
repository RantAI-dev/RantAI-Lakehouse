/**
 * OPS-16 decision D3 (widened by D5): no source file in `src/` or
 * `design-system/` may import the Google font loader again. That import
 * fetches a font while the console builds, so a build behind a firewall or on
 * an air-gapped host fails (CI runs 41446159405, 41449722905, 41480029356).
 * The banned module id is assembled at runtime on purpose: a `git grep` for it
 * must stay empty in both trees, and the check's own source cannot be the one
 * exception.
 */
import { strict as assert } from "node:assert";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const ROOT_DIR = fileURLToPath(new URL("../..", import.meta.url));
const SCANNED_DIRS = [
  join(ROOT_DIR, "src"),
  join(ROOT_DIR, "design-system"),
];
const BANNED_MODULE = ["next/font", "google"].join("/");

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) {
      return sourceFiles(path);
    }
    return /\.(ts|tsx)$/.test(entry.name) ? [path] : [];
  });
}

test("no console or design-system source file imports the Google font loader", () => {
  const files = SCANNED_DIRS.flatMap((dir) => {
    // A renamed tree would otherwise make this check pass by scanning nothing.
    assert.ok(existsSync(dir), `font-guard root is missing: ${dir}`);
    return sourceFiles(dir);
  });
  const offenders = files.filter((file) =>
    readFileSync(file, "utf8").includes(BANNED_MODULE),
  );

  assert.deepEqual(
    offenders,
    [],
    `these files fetch fonts at build time: ${offenders.join(", ")}`,
  );
});
