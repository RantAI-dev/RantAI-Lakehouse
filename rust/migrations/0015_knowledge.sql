-- Phase 2, Task 2.8: knowledge sources and vector jobs — metadata only.
--
-- HONESTY NOTE -- read before touching this file.
--
-- There is no vector database, embedding engine, or search index anywhere
-- in this repository (see `AI_PROJECT_INSIGHTS.md`: "There is no real ...
-- vector database ... in this repository", and a live check against the
-- real ClickHouse instance this deployment points at (`CH_URL`) found no
-- `Array(Float32|Float64)` embedding column and no vector index in any
-- user table). This migration therefore stores only the metadata a console
-- genuinely can own: a catalog of *declared* knowledge sources and vector
-- jobs (their configuration and status), the same shape
-- `src/services/mock/knowledge.ts` already exposes. It does NOT store
-- chunks, embeddings, or any retrievable content, and `semanticSearch`
-- stays served by the mock (see `lakehouse_store::knowledge`'s module doc
-- comment) because fabricating similarity scores against a document store
-- that does not exist would be strictly worse than an honestly-labeled
-- mock.
CREATE TABLE knowledge_source (
    id                    TEXT PRIMARY KEY,
    name                  TEXT NOT NULL,
    kind                  TEXT NOT NULL,
    status                TEXT NOT NULL DEFAULT 'draft',
    owner                 TEXT NOT NULL DEFAULT 'Current user',
    version               TEXT NOT NULL DEFAULT 'v1',
    last_refresh          TIMESTAMPTZ NOT NULL DEFAULT now(),
    chunk_count           BIGINT NOT NULL DEFAULT 0,
    embedding_model       TEXT NOT NULL,
    index_status          TEXT NOT NULL DEFAULT 'indexing',
    freshness_lag_seconds BIGINT NOT NULL DEFAULT 0,
    classification        TEXT NOT NULL,
    dependent_agents      BIGINT NOT NULL DEFAULT 0,
    asset_id              TEXT,
    vector_job_id         TEXT,
    created_at            TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT knowledge_source_kind_check
        CHECK (kind IN ('file', 'object-storage', 'web', 'table', 'query', 'manual')),
    CONSTRAINT knowledge_source_index_status_check
        CHECK (index_status IN ('ready', 'indexing', 'degraded')),
    CONSTRAINT knowledge_source_classification_check
        CHECK (classification IN ('public', 'internal', 'confidential', 'restricted')),
    CONSTRAINT knowledge_source_name_unique UNIQUE (name)
);

CREATE TABLE vector_job (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    status            TEXT NOT NULL DEFAULT 'draft',
    source            TEXT NOT NULL,
    -- References `knowledge_source.id` when the job indexes a registered
    -- source; NULL is allowed (a job's `source` field can name an
    -- ad-hoc/unregistered source, mirroring `VectorJob.sourceId?`).
    source_id         TEXT REFERENCES knowledge_source (id) ON DELETE SET NULL,
    output_asset_id   TEXT,
    embedding_model   TEXT NOT NULL,
    index_type        TEXT NOT NULL,
    last_run_at       TIMESTAMPTZ NOT NULL DEFAULT now(),
    owner             TEXT NOT NULL DEFAULT 'Current user',
    created_at        TIMESTAMPTZ NOT NULL DEFAULT now(),
    CONSTRAINT vector_job_name_unique UNIQUE (name)
);
