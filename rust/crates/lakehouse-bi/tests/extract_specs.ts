// Run by `specs_match_typescript.rs` via `bun run extract_specs.ts` — NOT
// part of the Next.js app. Prints `{ kpis, charts }` (id + sql only) from
// the TS source of truth (`src/lib/dashboard-specs.ts`) as JSON on stdout,
// so the Rust drift guard can compare it against `lakehouse_bi::specs`
// without re-implementing a TS parser.
//
// `dashboard-specs.ts` has no imports of its own (pure types + data), so a
// plain relative import works without any Next.js/bundler context.
import { CHARTS, KPIS } from "../../../../src/lib/dashboard-specs";

process.stdout.write(
  JSON.stringify({
    kpis: KPIS.map((k) => ({ id: k.id, sql: k.sql })),
    charts: CHARTS.map((c) => ({ id: c.id, sql: c.sql })),
  }),
);
