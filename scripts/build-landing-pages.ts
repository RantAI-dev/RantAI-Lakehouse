#!/usr/bin/env bun
/**
 * Builds the published landing page as a static export (the `WEB-1` page,
 * which an installed Lakehouse does not serve, `WEB-2`).
 *
 * The console's Next build is a standalone server (`output: "standalone"`)
 * whose root layout reads the request's CSP nonce and is `force-dynamic`
 * (`SEC-47`), so `output: "export"` cannot be turned on for the whole
 * `src/app` tree. Instead this runs a minimal Next project,
 * `ops/landing-pages`, whose only route renders `LandingPage` at `/`, with a
 * static layout of its own.
 *
 * `next build` itself runs under Node, not Bun (`REL-8`: the Bun Turbopack
 * runtime fails on the production build), so the child process is the
 * `next` bin script, whose shebang is `node`.
 *
 * The export is copied to `out-landing/` at the repository root, together
 * with the two files GitHub Pages needs there:
 *
 * - `CNAME` — the custom domain (`https://lakehouse.rantai.dev`);
 * - `.nojekyll` — without it, GitHub Pages runs Jekyll on the artifact and
 *   drops `_next/` (a leading underscore is "special" to Jekyll), so every
 *   script and stylesheet 404s.
 *
 * `out-landing/` is a build artifact and is gitignored; nothing here is
 * committed.
 */
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, rmSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const REPO_ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const PROJECT_DIR = join(REPO_ROOT, "ops", "landing-pages");
const EXPORT_DIR = join(PROJECT_DIR, "out");
const OUT_DIR = join(REPO_ROOT, "out-landing");
const NEXT_BIN = join(REPO_ROOT, "node_modules", ".bin", "next");
const CNAME = "lakehouse.rantai.dev";

function fail(message: string): never {
  console.error(`build-landing-pages: ${message}`);
  process.exit(1);
}

if (!existsSync(NEXT_BIN)) {
  fail(`next is not installed at ${NEXT_BIN}; run "bun install" first`);
}

// A stale export would let a failed build silently publish an old page.
rmSync(EXPORT_DIR, { recursive: true, force: true });
rmSync(OUT_DIR, { recursive: true, force: true });

const build = spawnSync(NEXT_BIN, ["build", PROJECT_DIR], {
  cwd: REPO_ROOT,
  stdio: "inherit",
  env: process.env,
});
if (build.error !== undefined) fail(`could not start next build: ${build.error.message}`);
if (build.status !== 0) fail(`next build exited with status ${build.status ?? "null"}`);

if (!existsSync(join(EXPORT_DIR, "index.html"))) {
  fail(`the export produced no index.html in ${EXPORT_DIR}`);
}

cpSync(EXPORT_DIR, OUT_DIR, { recursive: true });
writeFileSync(join(OUT_DIR, "CNAME"), `${CNAME}\n`);
writeFileSync(join(OUT_DIR, ".nojekyll"), "");

if (!existsSync(join(OUT_DIR, "_next"))) {
  fail(`the export produced no _next/ directory in ${OUT_DIR}`);
}

console.log(`build-landing-pages: wrote ${OUT_DIR} (CNAME ${CNAME})`);
