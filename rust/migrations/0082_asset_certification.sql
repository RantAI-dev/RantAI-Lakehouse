-- DATA-12 part 1 (docs/core/specs/data-12.md, F1): a mark on a table, so a
-- person can say "this is the one to trust" (`certified`) or "stop using
-- this" (`deprecated`, with a note and optionally the table to use instead).
--
-- The mark lives on `asset_annotation` (`0032`), keyed by the same full
-- catalog id, but it is written by its own statement
-- (`annotation::set_certification`) and its own permission
-- (`governance:write`, decision D1), never by `upsert_annotation`: that
-- statement names its columns, so editing the details cannot touch a mark,
-- and a `catalog:write` holder cannot set one.
--
-- Columns, all NULL when the asset has no mark:
--   * `certification`: `certified` or `deprecated`;
--   * `certification_note`: why, at most 1000 characters (deprecated only);
--   * `replacement_asset_id`: the catalog id to use instead, at most 200
--     characters (deprecated only). No foreign key: assets do not live in
--     Postgres, so the handler checks the id against the catalog;
--   * `certified_by`: the display name of the person who set the mark, at
--     most 128 characters;
--   * `certified_at`: when.
--
-- One table-level CHECK ties them: a note or a replacement is allowed only
-- with `deprecated`, and `certified_by` and `certified_at` are set exactly
-- when `certification` is. Like the bounds of `0032` these are defense in
-- depth; the handler validates first and answers 400 with a fixed sentence.
ALTER TABLE asset_annotation
    ADD COLUMN certification TEXT,
    ADD COLUMN certification_note TEXT,
    ADD COLUMN replacement_asset_id TEXT,
    ADD COLUMN certified_by TEXT,
    ADD COLUMN certified_at TIMESTAMPTZ,
    ADD CONSTRAINT asset_annotation_certification_check
        CHECK (certification IS NULL OR certification IN ('certified', 'deprecated')),
    ADD CONSTRAINT asset_annotation_certification_note_length_check
        CHECK (certification_note IS NULL OR char_length(certification_note) <= 1000),
    ADD CONSTRAINT asset_annotation_replacement_length_check
        CHECK (replacement_asset_id IS NULL OR char_length(replacement_asset_id) <= 200),
    ADD CONSTRAINT asset_annotation_certified_by_length_check
        CHECK (certified_by IS NULL OR char_length(certified_by) <= 128),
    ADD CONSTRAINT asset_annotation_certification_shape_check
        CHECK (
            ((certification IS NULL) = (certified_by IS NULL))
            AND ((certification IS NULL) = (certified_at IS NULL))
            AND (certification = 'deprecated'
                 OR (certification_note IS NULL AND replacement_asset_id IS NULL))
        );
