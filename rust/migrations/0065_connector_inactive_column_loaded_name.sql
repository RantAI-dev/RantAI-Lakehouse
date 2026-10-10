-- SRC-8 task 11 (review of the second pass): the Bronze table's Schema tab
-- lists the columns of the Bronze table, whose names the loader (`dlt`) has
-- normalised (`OrderDate` -> `order_date`). `connector_inactive_column`
-- (0064) holds the SOURCE name of a removed column, which cannot be matched
-- to the Schema tab: nothing in Rust reproduces `dlt`'s naming convention.
-- The orchestrator, where `dlt` runs, now sends the loaded name with every
-- before-load column; the API stores it with the observed shape and writes it
-- here when a removal is approved, so the catalog detail can join on it.
--
-- NULL means "not known" (a row approved before this migration, or a column
-- observed without a loaded name): such a row marks nothing, never a guess.
-- Additive; 0064 is not edited. Never edited once applied.

ALTER TABLE connector_inactive_column ADD COLUMN IF NOT EXISTS loaded_name TEXT;
