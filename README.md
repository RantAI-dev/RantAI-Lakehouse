<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="public/logo-dark.png">
    <source media="(prefers-color-scheme: light)" srcset="public/logo-light.png">
    <img alt="RantAI Lakehouse logo" src="public/logo-light.png" width="160">
  </picture>
</p>

<h1 align="center">RantAI Lakehouse</h1>

<p align="center">
  A data-lakehouse console for browsing a catalog, running pipelines,
  building dashboards, and chatting with an LLM over your data.
</p>

RantAI Lakehouse is a data-lakehouse console: a web UI for browsing a data
catalog, running and scheduling pipelines, building dashboards, managing
governance policies, and chatting with an LLM over your data. It's a
Next.js/React frontend backed by a Rust (axum) API that talks to Postgres,
ClickHouse, Dagster, and an OpenAI-compatible LLM.

The backend was originally written in TypeScript (as Next.js API routes)
and has since been fully ported to Rust; see "Status / Known limitations"
below and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for what that means
in practice.

## Architecture

```mermaid
flowchart LR
    Browser["Browser"]

    subgraph Frontend["Next.js (Bun runtime)"]
        UI["UI (App Router)"]
        Rewrite["/api/* rewrite\n(next.config.ts)"]
    end

    subgraph Backend["Rust backend"]
        API["lakehouse-api (axum)"]
    end

    Postgres[("Postgres\n(OLTP: identity, governance,\npipelines, connectors, ...)")]
    ClickHouse[("ClickHouse\n(analytics: serving.* marts,\ncatalog, lineage, BI)")]
    Dagster["Dagster\n(orchestration)"]
    LLM["LLM\n(OpenAI-compatible)"]

    Browser --> UI
    UI --> Rewrite
    Rewrite --> API
    API --> Postgres
    API --> ClickHouse
    API --> Dagster
    API --> LLM
```

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full module map (11
Rust crates), the request lifecycle (browser → rewrite → axum middleware →
policy check → handler → store/client), and the Postgres-vs-ClickHouse data
model.

## Quickstart

Assumes you have **Docker** and **Bun** (`>= 1.3.0`) installed and nothing
else.

```bash
# 1. Install Bun, if you don't have it
curl -fsSL https://bun.sh/install | bash

# 2. Clone and install frontend dependencies
git clone https://github.com/RantAI-dev/RantAI-Lakehouse.git
cd RantAI-Lakehouse
bun install

# 3. Configure the backend stack. Copy the example env file and set (at
#    minimum) AUTH_BOOTSTRAP_EMAIL / AUTH_BOOTSTRAP_PASSWORD — without
#    those there is deliberately no way to log in. Every other variable
#    has a safe local default; see .env.example and the Configuration
#    table below.
cp .env.example .env
$EDITOR .env

# 4. Bring up Postgres, ClickHouse, and the Rust API (built from
#    rust/Dockerfile). Dagster and a real LLM are NOT part of this stack —
#    see docs/OPERATIONS.md for what that means and how to add them.
docker compose up --build
# lakehouse-api listens on :8080 once postgres and clickhouse report
# healthy. Check with: curl -sf localhost:8080/health

# 5. In a separate terminal, point the frontend at the backend and run it
RUST_API_URL=http://localhost:8080 bun --bun next dev
```

Open [http://localhost:3000](http://localhost:3000). Sign in with the
bootstrap admin account you configured via `AUTH_BOOTSTRAP_EMAIL` /
`AUTH_BOOTSTRAP_PASSWORD` (see the Configuration table) — without those set,
no admin account is created and you'll need to seed one directly in
Postgres.

See [docs/OPERATIONS.md](docs/OPERATIONS.md) for what's in/out of the
compose stack, what `/health` actually checks, the Postgres backup/restore
procedure, and the biggest operational trap in this codebase (Postgres
being down fails quietly, not loudly).

Prefer running the Rust API directly instead of in a container (e.g. for
faster iteration)? The old path still works:

```bash
# Bring up just the data stores from compose, then run the API on the host
docker compose up postgres clickhouse
cd rust && cargo run -p lakehouse-api
```

### Runtime: Bun

This project runs on **Bun**, not Node.js. You need Bun `>= 1.3.0`
([install](https://bun.sh/docs/installation)):

```bash
curl -fsSL https://bun.sh/install | bash
```

All scripts (`dev`, `build`, `start`, `lint`, `typecheck`) use the `--bun`
flag so Next.js executes under the Bun runtime, not Node. The lockfile is
`bun.lock` — don't use `npm install`, it will create an out-of-sync
`package-lock.json`.

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
bun run test        # bun test (unit tests under src/lib)
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

The workspace declares `rust-version = "1.85"` in `rust/Cargo.toml` — that's
the minimum edition-2024-capable toolchain this code promises to compile
on. `rust/rust-toolchain.toml` pins the toolchain actually used for local
dev/CI (currently `1.96.1`, newer than the MSRV floor) so contributors and
CI build with the same compiler; `rustup` will fetch it automatically the
first time you run `cargo` inside `rust/`.

## Configuration

Every environment variable below is read in
[`rust/crates/lakehouse-api/src/config.rs`](rust/crates/lakehouse-api/src/config.rs),
which is the authoritative source — this table was generated from it, not
guessed.

| Variable | Purpose | Default | Required? |
| --- | --- | --- | --- |
| `CH_URL` | ClickHouse HTTP interface URL | `http://localhost:18123` | No |
| `CH_USER` | ClickHouse basic-auth user | `default` | No |
| `CH_PASSWORD` | ClickHouse basic-auth password | `""` | No |
| `DAGSTER_URL` | Dagster GraphQL endpoint | `http://localhost:13030/graphql` | No |
| `DAGSTER_REPO` | Dagster repository name | `__repository__` | No |
| `DAGSTER_LOCATION` | Dagster repository location | `dispar_orchestrate.definitions` | No |
| `LLM_URL` | LLM chat-completions base URL (OpenAI-compatible) | `https://api.minimax.io/v1` | No |
| `LLM_MODEL` | LLM model name | `MiniMax-M3` | No |
| `LLM_KEY` | LLM API key. Falls back to `MINIMAX_API_KEY` if unset **or empty** (`||` semantics, not `??`) | `""` | No (but AI features won't work without it) |
| `MINIMAX_API_KEY` | Fallback for `LLM_KEY` | — | No |
| `EMBED_SECRET` | HMAC signing secret for signed dashboard embeds | unset (embedding disabled) | No |
| `ALERTS_RUN_TOKEN` | Shared bearer token required to call `POST /api/alerts/run` | unset | No, but the endpoint fails closed (401) when unset — see Security notes below |
| `SMTP_HOST` | SMTP host for alert/digest email delivery | unset (email disabled) | No |
| `SMTP_PORT` | SMTP port. Invalid values log a warning and fall back to the default rather than failing boot | `587` | No |
| `SMTP_SECURE` | Force implicit TLS (`"true"`). Effective value is also `true` whenever `SMTP_PORT` is `465`, even if this is unset | `false` | No |
| `SMTP_USER` | SMTP auth username | unset (no SMTP auth) | No |
| `SMTP_PASS` | SMTP auth password | `""` | No |
| `SMTP_FROM` | `From` header for outgoing email. Falls back to `SMTP_USER`, then to `rantai-lake@localhost` | `rantai-lake@localhost` | No |
| `PORT` | Port `lakehouse-api` listens on | `8080` | No (but an invalid value fails to boot — the one field that does) |
| `APP_ENV` | `"development"` or `"local"` relaxes the session cookie's `Secure` attribute for local HTTP dev. Falls back to `NODE_ENV`. Never bypasses authentication | unset (defaults closed/`Secure`) | No |
| `NODE_ENV` | Fallback for `APP_ENV` | — | No |
| `AUTH_BOOTSTRAP_EMAIL` | Email for the idempotent bootstrap admin account created at startup | unset (no bootstrap admin) | No, but recommended for first run |
| `AUTH_BOOTSTRAP_PASSWORD` | Password for the bootstrap admin account | unset | No, but required alongside `AUTH_BOOTSTRAP_EMAIL` to actually create one |
| `OIDC_ISSUER` | OIDC provider issuer URL | unset | No — OIDC requires both this and `OIDC_CLIENT_ID` |
| `OIDC_CLIENT_ID` | This app's client id as registered with the OIDC provider | unset | No — see above |
| `OIDC_CLIENT_SECRET` | Reserved for a future authorization-code exchange; not currently read by `OidcAuthenticator` | unset | No |
| `OIDC_PROVIDER_NAME` | Short label combined with `oidc:` to form `Principal::provider` / `auth_identity.provider` | `default` | No |
| `OIDC_JWKS_URL` | Explicit JWKS endpoint override | derived: `{OIDC_ISSUER}/.well-known/jwks.json` | No |
| `OIDC_JIT_PROVISIONING` | `"true"` to auto-create an `app_user` for a validating token with no linked identity yet | `false` | No |
| `OIDC_ROLE_MAP` | `"group1=Role One,group2=Role Two"` — maps an IdP group/role claim to a local role name | empty | No |
| `OIDC_GROUPS_CLAIM` | Which token claim carries the caller's groups/roles | `groups` | No |
| `OIDC_CLOCK_SKEW_SECONDS` | Clock-skew tolerance for `exp`/`nbf` validation. Invalid values fall back to the default | `60` | No |
| `DATABASE_URL` | Postgres connection string for Phase 2 OLTP storage | `postgres://lakehouse:lakehouse@localhost:5432/lakehouse` | No (but a wrong/unreachable value means every Phase 2 route returns 503 — see below) |

Two additional variables live outside `config.rs`, on the frontend side —
listed here because you need them to run the console at all:

| Variable | Purpose | Default | Required? |
| --- | --- | --- | --- |
| `RUST_API_URL` | Target the Next.js `/api/*` rewrite proxies to (`next.config.ts`) | unset (rewrite disabled — no backend reachable) | Yes, to reach the Rust backend at all |
| `NEXT_PUBLIC_SSO_ENABLED` | Build-time flag that shows/hides SSO login UI | unset (SSO UI hidden) | No — see "SSO configuration is split across two processes" below |

See `rust/crates/lakehouse-auth/README.md` for detailed, per-provider OIDC
setup instructions (Okta, Entra, Google, Keycloak).

## Status / Known limitations

This is a young, honestly-scoped project. Please read this before filing an
issue about any of the following — they're known, not bugs:

- **`rust/Dockerfile` does not currently build from a clean clone**
  (wrong rustc pin + a `pub mod tenant;` in `lib.rs` whose source file
  isn't committed yet). See the "KNOWN BLOCKER" section at the top of
  [docs/OPERATIONS.md](docs/OPERATIONS.md) for the exact errors and status.
- **`streaming` is mocked.** There is no Kafka/Redpanda/Pulsar/Flink
  anywhere in this project. The streaming domain in the UI is backed by
  `src/services/mock/streaming.ts`.
- **`knowledge.search` is mocked.** There is no vector store or embeddings
  API wired up. Knowledge *sources* and *vector jobs* ARE real, backed by
  Postgres (`lakehouse-store::knowledge`) — only the search-query path
  itself is mocked.
- **`getWorkspaceSettings` returns a fixed response.** The contract has no
  setter; workspace settings are not actually persisted or configurable
  yet.
- **No login rate limiting beyond logging.** Failed login attempts are
  logged but not throttled or locked out.
- **SSO is gated by a build-time flag, not a runtime one.** The frontend
  can't read the Rust process's environment variables directly, so whether
  the SSO login UI shows up is controlled by `NEXT_PUBLIC_SSO_ENABLED` at
  *build* time — it cannot react to whether `OIDC_ISSUER`/`OIDC_CLIENT_ID`
  are actually set on the backend at runtime. A `GET /api/auth/providers`
  endpoint (letting the frontend ask the backend what's configured) has
  been proposed but is not built.
- **The service does not refuse to boot when Postgres is down.**
  `lakehouse-store::connect_lazy` never does network I/O at startup;
  Postgres connectivity is only discovered lazily, at first use. Dependent
  (Phase 2) routes return `503` instead of the process failing to start.
  This is deliberate (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)),
  but it means a misconfigured `DATABASE_URL` degrades quietly rather than
  loudly — watch logs and the 503 rate, not just process uptime.
- **Sessions and service tokens have no rotation/cleanup job.** Nothing
  today expires or garbage-collects them beyond whatever TTL logic exists
  at issuance/verification time.
- **A previously-internal API key and internal LAN hostnames are present
  in git history** (2 and ~10 commits reachable from `main`,
  respectively), predating this repo going public. The key must be, and
  has been treated as, compromised. See the "Known exposure" section of
  [SECURITY.md](SECURITY.md) and [docs/CI.md](docs/CI.md) (the
  `gitleaks (full git history)` job is intentionally red because of this).
- **The backend was ported from TypeScript to Rust by an AI agent
  workflow**, reviewed by AI reviewers, task by task, with a parity harness
  comparing responses against the original TypeScript backend along the
  way. The commit history has not had a full human security/architecture
  review end to end — treat it accordingly, especially before production
  use.

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for dev setup, test commands, commit
conventions, and PR expectations. See [SECURITY.md](SECURITY.md) to report
a vulnerability privately.

## License

AGPL-3.0-or-later — see [LICENSE](LICENSE) and [NOTICE](NOTICE) (including a
transitive LGPL-3.0 note for `sharp-libvips`, which is compatible with
AGPL-3.0). `v0.1.0` was released under Apache-2.0; the project relicensed to
AGPL-3.0-or-later afterward — see [CHANGELOG.md](CHANGELOG.md).

---

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
├── app/                 # App Router (layout, page, routes)
├── components/
│   ├── ui/              # shadcn/ui components
│   └── shared/          # Shared components (ThemeProvider, etc.)
├── lib/                 # Utilities (utils, config)
├── hooks/               # Custom React hooks
└── types/               # TypeScript types/interfaces
```

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
Per the design system, dark mode is **forced**. To enable a light/dark
toggle, remove the `forcedTheme` prop on that component.
