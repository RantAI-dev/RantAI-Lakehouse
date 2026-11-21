/**
 * OPS-40 (Next.js 16.4.3 → 16.4.5): `next dev` on 16.4.5 writes a managed
 * agent-rules block into the project root's `AGENTS.md` whenever it detects an
 * AI coding agent (`node_modules/next/dist/server/lib/generate-agent-files.js`,
 * reached from `syncAgentRulesForDev(dir, config.agentRules)`). In this
 * repository that root file IS the agent-rules file every coding session reads
 * first, so the dev server would rewrite the project's own rules and leave the
 * tree dirty on every `bun run dev` — and a commit of that block would put
 * Next's text inside the repository's rules.
 *
 * `agentRules: false` in `next.config.ts` turns the write off (and takes the
 * removal branch, so a block that was committed is deleted again on the next
 * dev run). This guard fails if a later edit puts the writer back on, or if
 * the managed block ends up in the tracked rules file.
 */
import { strict as assert } from "node:assert";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

import nextConfig from "../../next.config";

const AGENTS_MD = fileURLToPath(new URL("../../AGENTS.md", import.meta.url));
const MANAGED_BLOCK = "<!-- BEGIN:nextjs-agent-rules -->";

test("the console's Next.js config keeps `next dev` from writing the repository's AGENTS.md", () => {
  assert.equal(
    nextConfig.agentRules,
    false,
    "`next dev` appends a managed agent-rules block to the project root's AGENTS.md when it detects an AI coding agent; this repository's AGENTS.md is the agent-rules file itself, so `agentRules` must stay false (OPS-40).",
  );

  assert.ok(
    !readFileSync(AGENTS_MD, "utf8").includes(MANAGED_BLOCK),
    "the repository's AGENTS.md carries Next.js's managed agent-rules block; with `agentRules: false` the dev server removes that block, so the tracked rules file would change on the next `bun run dev`.",
  );
});
