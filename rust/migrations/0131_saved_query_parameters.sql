-- Query Studio: a saved query keeps the definitions of its `{{name}}`
-- parameters (backlog `QS-5` part 1, feature page
-- `docs/core/features/query-parameters.md`, decision 3 and D6).
--
-- WHY: a statement can now hold `{{region}}`, and the SQL alone cannot say
-- what type the value has, what it defaults to or which options a dropdown
-- offers. Those definitions are part of what the author saved, so they sit
-- beside the SQL on the query, and beside the SQL on every version, so
-- going back to an earlier version brings back the definitions that went
-- with that SQL and not the current ones.
--
-- `parameters` is a JSON array of definitions, `[{name, type, default,
-- options}]`. The API validates it against the SQL before it writes it
-- (`lakehouse-api` `query_params.rs`); the database only holds it, so a
-- change to the shape of a definition never needs a migration. `NOT NULL`
-- with an empty-array default means "no parameters", never "unknown": every
-- row written before this migration, and every insert that names its
-- columns and leaves this one out (the seed, the tests, an older
-- deployment's writer), reads as a query with none.
--
-- No backfill beyond the default. A statement saved before this migration
-- that happens to hold a `{{...}}` has no definition for it and keeps none:
-- inventing one would be a guess about what the author meant. A run reads
-- an undefined placeholder as text (feature decision 11), and the next save
-- of that query is checked against its definitions like any other.

ALTER TABLE saved_query
    ADD COLUMN parameters JSONB NOT NULL DEFAULT '[]'::jsonb;

ALTER TABLE saved_query_version
    ADD COLUMN parameters JSONB NOT NULL DEFAULT '[]'::jsonb;
