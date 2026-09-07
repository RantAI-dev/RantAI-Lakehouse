# Feature Coverage

Canonical coverage matrix lives in:

**[`docs/FEATURE_COVERAGE.md`](./docs/FEATURE_COVERAGE.md)**

Also see:

- [`docs/RANTAI_LAKE_REPOSITORY_VALIDATION.md`](./docs/RANTAI_LAKE_REPOSITORY_VALIDATION.md)
- [`docs/UX_FLOWS.md`](./docs/UX_FLOWS.md)
- [`AI_PROJECT_INSIGHTS.md`](./AI_PROJECT_INSIGHTS.md)

**As of P6, most product data paths are live**, not mock adapters. Ten of
eleven `src/services/index.ts` domains (overview, assets/catalog, pipelines,
query studio, agents, governance, ops, identity, connectors, storage) are
backed by the real Rust API over ClickHouse/Postgres/Dagster — including the
new lakehouse layer (Bronze Iceberg via Lakekeeper/RustFS, CDC via Debezium,
Dagster-run maintenance) surfaced through the Catalog, Storage, and
Governance (Maintenance/Replication) domains. Only `knowledge.search`
remains mocked, deliberately (no vector store/embedding index exists). The
`streaming` domain — previously mocked because no Kafka/Flink/streaming
engine exists in this stack — has been removed outright, not kept as a
mock. See [`docs/FEATURE_COVERAGE.md`](./docs/FEATURE_COVERAGE.md) for the
per-domain breakdown and `AI_PROJECT_INSIGHTS.md` for what's real vs. not.
