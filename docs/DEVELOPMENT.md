# Development

How to run the console and the API from source, test them, and the
frontend's conventions. Moved here from the README on 2026-11-05. The code
standard is [CODE-STANDARD.md](CODE-STANDARD.md); the contribution rules are
[CONTRIBUTING.md](../CONTRIBUTING.md).

## The whole stack from a checkout

Assumes Docker and nothing else.

```bash
git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git
cd RantAI-Lakehouse
sh ops/init-env.sh      # writes .env with generated secrets; refuses to overwrite one
$EDITOR .env            # set AUTH_BOOTSTRAP_EMAIL (and see CONFIGURATION.md)
scripts/compose.sh up --build
```

`ops/init-env.sh` generates every required secret (there are no default
credentials, `SEC-18`; Compose refuses to start while one is empty), including
`CONNECTOR_SECRETS_KEY`: keep it, or every stored connector credential has to be
entered again. `scripts/compose.sh` is `docker compose` with `GIT_SHA` set from
this checkout, which the pipeline page's source view needs. The API listens on
`:8080` once Postgres and ClickHouse are healthy (`curl -sf localhost:8080/health`)
and the console on `:3000` once the API is. Sign in with `AUTH_BOOTSTRAP_EMAIL`
and `AUTH_BOOTSTRAP_PASSWORD`.

### Test-only settings

A few settings the acceptance gates and the tests use are deliberately not
in `.env.example` any more (`REL-19`, decision D6). A normal run sets none of
them; each is defaulted where it is read, so nothing changes when the line
is absent:

| Setting | Read by | Default |
| --- | --- | --- |
| `GOLD_MART_NAME`, `GOLD_EXPORT_ROW_COUNT` | Gold export acceptance test (`ops/gold_export`); compose passes them to `dagster-code-location` | `gold_export_smoke`, `7` |
| `BRONZE_SOURCE_SCHEMA`, `BRONZE_SOURCE_TABLE`, `BRONZE_TABLE_NAME` | Bronze ingest (`dagster/dispar_orchestrate/dlt_pipeline.py`) and the G3/G7/G8 gates | `ingest_demo`, `orders`, `g3a_orders` |
| `BRONZE_CURATED_SLUGS` | The API's catalog | empty |
| `CONSOLE_ALLOWED_DEV_ORIGINS` | `next.config.ts`, `next dev` only (`OPS-11`) | empty (localhost only) |
| `APP_ENV` | The API's session-cookie posture; compose defaults it | `local` under compose |

No workflow or gate read these from `.env.example`: the CI jobs write their
own throwaway `.env`, and each gate defaults them in its own script or gets
the compose default. `docs/CONFIGURATION.md` documents every one.

### Developing: the console and the API on the host

Prefer running the console and the API on the host (e.g. for faster
iteration)? The old path still works — it needs **Bun** (`>= 1.3.0`) and
Node 20.9 or newer for the build, which the operator path above does not:

```bash
# Bring up just the data stores from compose, then run the API on the host
docker compose up postgres clickhouse
cd rust && cargo run -p lakehouse-api

# And the console, in another terminal
bun install
RUST_API_URL=http://localhost:8080 bun --bun next dev
```

### Runtime: Bun

This project runs on **Bun**, not Node.js. You need Bun `>= 1.3.0`
([install](https://bun.sh/docs/installation)):

```bash
curl -fsSL https://bun.sh/install | bash
```

Most scripts (`dev`, `start`, `lint`, `typecheck`) use the `--bun`
flag so Next.js executes under the Bun runtime, not Node. `bun run build`
runs `next build` under Node (Node 20.9 or newer must be installed).
It runs under Node because Bun's Turbopack runtime fails with "Expected
CommonJS module to have a function wrapper" (`REL-8`, PR #170 review round 1,
SHOULD-FIX 2). The lockfile is `bun.lock` — don't use `npm install`, it will
create an out-of-sync `package-lock.json`.

> Note: in `ps`, the server process shows up as `node` because Bun
> intentionally masquerades as Node for tooling compatibility. To confirm
> you're actually running Bun, check `readlink /proc/<pid>/exe` — it
> resolves to the `bun` binary.

```bash
# Development
bun run dev

# Production build, then run
bun run build
bun start
```

### Test, lint, typecheck

```bash
bun run test        # bun test src (every *.test.ts/tsx under src/)
bun run lint         # eslint
bun run typecheck    # tsc --noEmit
```

Rust backend, from `rust/`:

```bash
cargo test --all-features
cargo fmt --check
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo deny check licenses
```

### Rust toolchain / MSRV

`rust/Cargo.toml`'s `rust-version` is `1.99` and equals the
`rust/rust-toolchain.toml` pin (`channel = "1.99.1"`): since REL-9
(product-owner decision D1, 2026-10-17) the minimum the workspace promises
is the latest stable — `sqlx` 0.9, which `SEC-41` needs, requires rustc
1.94, so a held-over lower promise could not be kept — rather than a
dependency's floor. `rustup` fetches the pinned toolchain automatically on
the first `cargo` run inside `rust/`, so contributors and CI build with
one compiler. The CI `msrv` job proves the floor one patch below the pin
by running `cargo +1.99.0 check --workspace --all-targets --locked` in
`rust/`; the `+` is load-bearing because the `rust-toolchain.toml` pin
would otherwise override the toolchain that job installs (rustup
precedence) and rebuild the job on the pin instead of the floor. See
`docs/CI.md`.

## The lakehouse layer

See [docs/ARCHITECTURE.md](ARCHITECTURE.md) for the full module map (14
Rust crates), the request lifecycle (browser → rewrite → axum middleware →
policy check → handler → store/client), and the Postgres-vs-ClickHouse data
model.

The lakehouse layer (Lakekeeper, RustFS/SeaweedFS, Debezium, Trino) is real
infrastructure, verified end to end (see `docs/plans/G1-RESULT.md` through
`docs/plans/P5-RESULT.md`), and is compose-profile opt-in for the
Dagster/Trino pieces (`dagster`, `trino`). **`lakehouse-iceberg` now has a
real caller**: `POST`/`GET /api/gold/export/{mart}`
(`lakehouse-api::routes::gold`, ADR 0010) reads a Gold mart from
ClickHouse `MergeTree` and appends it to its own `gold` Iceberg namespace
through Lakekeeper, and reads it back through `iceberg-rust` — the console
still reads *Bronze* through ClickHouse's `DataLakeCatalog`/the
`bronze_meta.*` registry, not through `lakehouse-iceberg` directly, since
that read path works on both 26.3 and 26.8 (see "Status / Known
limitations" below for the write-path defects, and
`docs/plans/CLICKHOUSE-26.8-REMEASUREMENT.md` for which of them 26.8
fixes).

## Frontend implementation notes (Next.js scaffold)

The sections below are lower-level frontend conventions, kept from the
project's original scaffold notes.

### Tech stack

- **Bun** (runtime, package manager, test runner)
- **Next.js** (App Router)
- **TypeScript**
- **Tailwind CSS**
- **shadcn/ui** (UI components)
- **lucide-react** (icons)
- **next-themes** (dark mode)
- **clsx** (utility class names)

Styling follows the **Rantai Design System** (`design-system/`): OKLCH
blue/navy color tokens, Geist font, dark mode as the default theme.

### `src/` folder structure

```
src/
├── app/                 # App Router: thin page files that render a feature
├── features/            # One folder per console area (imports @/services only)
├── services/            # contracts/ → clients/ (apiFetch) → index.ts
├── components/
│   ├── ui/              # shadcn/ui components
│   ├── patterns/        # Repo patterns (page headers, page states, ...)
│   └── app-shell/       # Navigation and layout chrome
├── config/              # Shared configuration (data-table settings)
├── lib/                 # Pure logic and utilities (unit-tested)
├── hooks/               # Custom React hooks
├── types/               # TypeScript types
└── proxy.ts             # Next.js proxy (embed frame policy headers)
```

The layering rule is in `docs/CODE-STANDARD.md` section 4.1.

### Adding a shadcn component for the first time

1. Browse [shadcn/ui Components](https://ui.shadcn.com/docs/components).
2. Add a component via the CLI:
   ```bash
   bunx shadcn@latest add <component-name>
   ```
   Example:
   ```bash
   bunx shadcn@latest add card
   bunx shadcn@latest add dialog
   bunx shadcn@latest add input
   ```
3. Components land in `src/components/ui/` (per `components.json`).
4. Usage:
   ```tsx
   import { Button } from "@/components/ui/button"
   import { Card, CardContent, CardHeader } from "@/components/ui/card"

   export default function Page() {
     return (
       <Card>
         <CardHeader>Title</CardHeader>
         <CardContent>
           <Button>Click</Button>
         </CardContent>
       </Card>
     )
   }
   ```

### Path alias

- `@/*` → `./src/*` (configured in `tsconfig.json`)

### Dark mode

The project uses `next-themes` via the design system's `ThemeProvider`
(`@rantai/design-system/components/theme-provider`) in `src/app/layout.tsx`.
Dark is the default theme (`defaultTheme="dark"`, `enableSystem`); it is not
forced, and `src/components/theme-toggle.tsx` switches between light and
dark.
