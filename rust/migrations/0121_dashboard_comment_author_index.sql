-- The index the comment post limit reads (`BI-38` D2,
-- `docs/superpowers/plans/2026-11-02-bi-38-comment-limits.md`).
--
-- `dashboard_comment::insert` counts one author's comments inside a five-minute
-- window before writing the next one. The count deliberately includes deleted
-- rows (deleting and posting again does not free a place), so the index is not
-- partial: it covers every row of an author, live or deleted, and both the
-- window predicate and the `min(created_at)` the retry delay is built from read
-- it. Without it the count is a scan of the whole table on every post.
CREATE INDEX IF NOT EXISTS dashboard_comment_author_created_idx
    ON dashboard_comment (author_id, created_at);
