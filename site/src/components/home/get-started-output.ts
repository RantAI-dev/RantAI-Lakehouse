/**
 * WEB-5 O23: what each Quickstart command prints, shown dimmed under it once
 * it has "run". Terminal output, like the commands themselves
 * (`GET_STARTED_COMMANDS`), so it is not translated (D3).
 *
 * Each line is a claim of real behaviour, so each is taken from the thing
 * that prints it, shortened but not invented:
 * - `git clone`: git's own "Cloning into '<dir>'..." and the "done." it ends
 *   its progress with (counts left out);
 * - `sh ops/init-env.sh`: the script's own last lines (ops/init-env.sh,
 *   `echo "init-env: wrote $target (mode 600). Filled:"` and the variables
 *   it generated, the first two of them here);
 * - `scripts/compose.sh up --build`: the README's Quickstart, "lakehouse-api
 *   listens on :8080 once postgres and clickhouse report healthy … and the
 *   console listens on :3000 once the API reports healthy" (REL-2).
 * `ok` marks the line that says the step worked; it gets the green check.
 * `comment` is a dim `#` line, not output: step 2's is the README's instruction for the first
 * account (README step 2: set AUTH_BOOTSTRAP_EMAIL and AUTH_BOOTSTRAP_PASSWORD in the .env it
 * wrote; SEC-18, REL-2), which used to be a paragraph under the step.
 */
export type OutputLine = { text: string; ok?: true; comment?: true };

export const GET_STARTED_OUTPUT: readonly (readonly OutputLine[])[] = [
  [{ text: "Cloning into 'RantAI-Lakehouse'..." }, { text: "Resolving deltas: 100%, done.", ok: true }],
  [
    { text: "init-env: wrote .env (mode 600). Filled:" },
    { text: "  POSTGRES_PASSWORD  RUSTFS_ACCESS_KEY  …", ok: true },
    { text: "# then set AUTH_BOOTSTRAP_EMAIL and AUTH_BOOTSTRAP_PASSWORD in .env", comment: true },
  ],
  [{ text: "lakehouse-api  healthy" }, { text: "console  on http://localhost:3000", ok: true }],
];
