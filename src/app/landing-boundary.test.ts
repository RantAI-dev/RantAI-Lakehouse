/**
 * WEB-15 review BLOCKER 1: the console image does not contain the landing
 * page. `.dockerignore` leaves `src/app/landing` and `src/features/landing`
 * out (WEB-2, product owner 2026-11-02: an install must not offer it), so a
 * console file that imports from either folder type-checks in the repository
 * and then breaks `bun run build` inside the image ("Module not found").
 * That is how the sign-in page's first import of the landing's reduced-motion
 * hook got through: `tsc` sees the whole repository, the image does not.
 *
 * This check reads every import specifier (`from "…"`, `import "…"`,
 * `import("…")`, `require("…")`) of every `.ts`/`.tsx` file outside those two
 * folders, resolves `@/…` and relative paths, and fails on any that lands in
 * them. It matches import syntax, not mentions: a comment naming the folder
 * (as `auth-provider.tsx` does) is fine.
 */
import { strict as assert } from "node:assert";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { dirname, join, relative, resolve, sep } from "node:path";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

const ROOT_DIR = fileURLToPath(new URL("../..", import.meta.url));
const SRC_DIR = join(ROOT_DIR, "src");
/** The folders the console image leaves out, as `.dockerignore` names them. */
const EXCLUDED = ["src/app/landing", "src/features/landing"];
const EXCLUDED_DIRS = EXCLUDED.map((dir) => join(ROOT_DIR, dir));

const SPECIFIER = [
  /\bfrom\s*["']([^"']+)["']/g,
  /\bimport\s*["']([^"']+)["']/g,
  /\bimport\s*\(\s*["']([^"']+)["']\s*\)/g,
  /\brequire\s*\(\s*["']([^"']+)["']\s*\)/g,
];

function sourceFiles(dir: string): string[] {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    if (entry.isDirectory()) return sourceFiles(path);
    return /\.(ts|tsx)$/.test(entry.name) ? [path] : [];
  });
}

function inExcluded(path: string): boolean {
  return EXCLUDED_DIRS.some((dir) => path === dir || path.startsWith(dir + sep));
}

/** The absolute path an in-repo specifier points at, or null for a package. */
function target(file: string, specifier: string): string | null {
  if (specifier.startsWith("@/")) return join(SRC_DIR, specifier.slice(2));
  if (specifier.startsWith(".")) return resolve(dirname(file), specifier);
  return null;
}

/** `file: specifier` for each import of `file` that resolves into an excluded folder. */
function landingImports(file: string): string[] {
  const text = readFileSync(file, "utf8");
  return SPECIFIER.flatMap((pattern) =>
    [...text.matchAll(pattern)]
      .map((m) => m[1])
      .filter((specifier) => {
        const to = target(file, specifier);
        return to !== null && inExcluded(to);
      })
      .map((specifier) => `${relative(ROOT_DIR, file)}: ${specifier}`),
  );
}

test("the folders this guard protects are the ones the console image excludes", () => {
  const ignored = readFileSync(join(ROOT_DIR, ".dockerignore"), "utf8").split("\n").map((l) => l.trim());
  for (const dir of EXCLUDED) {
    assert.ok(existsSync(join(ROOT_DIR, dir)), `landing folder is missing: ${dir}`);
    assert.ok(ignored.includes(dir), `.dockerignore no longer excludes ${dir}; update this guard`);
  }
  // The scanner must see imports at all: the landing's own files import each
  // other, so a broken pattern would show up here instead of passing silently.
  const inside = EXCLUDED_DIRS.flatMap(sourceFiles).flatMap(landingImports);
  assert.ok(inside.length > 0, "the import scanner found no imports inside the landing folders");
});

test("no console file imports from the landing folders, which the console image does not contain (.dockerignore, WEB-2)", () => {
  const offenders = sourceFiles(SRC_DIR)
    .filter((file) => !inExcluded(file))
    .flatMap(landingImports);
  assert.deepEqual(
    offenders,
    [],
    "These imports reach into src/app/landing or src/features/landing, which .dockerignore leaves out of the console image (WEB-2); the image's build fails on them. Move the shared code to a console folder (for example src/hooks) instead:\n" +
      offenders.join("\n"),
  );
});
