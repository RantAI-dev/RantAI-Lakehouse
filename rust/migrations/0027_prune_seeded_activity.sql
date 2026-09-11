-- WS1 finding J17: drop the SEEDED FIXTURE *activity* rows from
-- `0008_seed_pipelines.sql` and `0012_seed_overview_alerts.sql`.
--
-- WHY: both seeds insert fabricated activity, not definitions, onto
-- console surfaces WS1 keeps.
--
-- `0008_seed_pipelines.sql`'s four `pipeline_definition` rows carry
-- invented statuses (`ready`/`failed`/`running`/`scheduled`), run times
-- computed relative to whenever this migration happens to apply
-- (`now() - interval '...'`), `sla_ok`/`freshness_lag_seconds` numbers
-- nobody measured, and `connector_id`s (`conn-pg-core`, `conn-s3-docs`)
-- that `0022_prune_connector_seed.sql` already removed as fixture — a
-- fresh Pipelines list would show a mix of real pipelines and these four,
-- with nothing to tell a viewer which is which.
--
-- `0012_seed_overview_alerts.sql`'s five `alert_instance` rows narrate
-- incidents that never happened on this deployment, down to invented
-- specifics (`al-02` claims a dead-letter target "captured 1,204
-- records"). The Overview alerts list is a kept surface; these rows would
-- read as real incident history.
--
-- Following `0026_drop_seeded_agent_runs.sql`'s precedent: this prunes
-- ACTIVITY, not DEFINITIONS. Seeded policies, classification rules, saved
-- queries, agent definitions, and identity rows are deliberately left in
-- place — they are examples of configuration, not claims about what has
-- happened. Quality-rule *results* are likewise untouched here; those are
-- handled at read time, not by deleting seed rows.
--
-- PRECISION: each delete matches on the seeded id AND an original column
-- value (`name` for pipelines, `title` for alerts) from the seed files
-- verbatim, never a blanket `DELETE FROM` — a deployment that happened to
-- reuse one of these ids for its own row keeps that row, because the
-- second column would not match.
DELETE FROM pipeline_definition
WHERE (id, name) IN (
    ('pl-orders-rollup', 'orders_hourly_rollup'),
    ('pl-erp-inventory', 'erp_inventory_sync'),
    ('pl-policy-docs',   'supplier_docs_ingest'),
    ('pl-embed-faq',     'faq_embedding_refresh')
);

DELETE FROM alert_instance
WHERE (id, title) IN (
    ('al-01', 'Streaming lag above threshold'),
    ('al-02', 'Pipeline failed after retries'),
    ('al-03', 'Residency policy blocked a query'),
    ('al-04', 'Agent budget nearing limit'),
    ('al-05', 'Quality check failing on curated table')
);
