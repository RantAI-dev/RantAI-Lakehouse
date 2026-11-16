-- OPS-8 part B: the rates behind the usage view's money column, and the
-- permission that opens the view.
--
-- WHY A TABLE OF RATES: the usage view (`GET /api/usage/queries`) shows an
-- amount in money only from rates an administrator entered, and labels it an
-- estimate (spec `docs/core/specs/ops-8.md`). The earlier `/api/ops/usage`
-- invented its figures from a bytes-per-row constant and was removed
-- (`routes/ops.rs` module doc); no price is ever built into the code.
--
-- ONE SET FOR THE DEPLOYMENT, like `reporting_settings` (`0070`), but not a
-- singleton: a rate has an effective date, and a day is priced with the rate
-- in force on that day. `kind` is what the rate is per: `scan_tb` (one
-- terabyte scanned), `engine_hour` (one hour of engine wall-clock time) and
-- `storage_gb_month` (one gigabyte stored for a month, stored now and used by
-- the storage part of the view). The rate for a day is, per kind, the row
-- with the latest `effective_from` on or before that day; two rows with the
-- same `effective_from` are a correction, and the one created last wins.
--
-- APPEND ONLY: a new price is a new row, and nothing updates or deletes one,
-- so the amounts that priced an earlier period can be read back later. The
-- store module offers no UPDATE or DELETE, and the triggers below refuse them
-- in the database too, the way `0142_audit_event_append_only.sql` does for
-- `audit_event`. The same LIMIT holds: the owner of the database can disable
-- a trigger.
--
-- ONE CURRENCY: `currency` is a three-letter upper-case code (ISO 4217 shape;
-- the list of codes is not checked here). The API refuses a rate whose
-- currency differs from the ones already stored, so one estimate never adds
-- two currencies. The check is made by `lakehouse_store::usage::add_rate`, not
-- by the table, under an advisory lock that makes writers go one after another
-- (a lone `INSERT ... WHERE NOT EXISTS` lets two writers of different
-- currencies both pass under READ COMMITTED); a writer that bypasses the store
-- is held only by the `CHECK`s below. Changing the deployment's currency is
-- not offered.
-- `amount` is NUMERIC(18,6), never a float: it is money.
--
-- `created_by` is the principal id of whoever entered the rate.
--
-- THE PERMISSION: `usage:read` opens the usage routes. There is no built-in
-- tenant administrator role (`0002_seed_identity.sql`); the grant goes to
-- Governance Admin, the role that already holds `audit:read`, and any other
-- role gets it from an administrator editing the role. `*:*` already holds
-- everything. The UPDATE follows `0031_governance_admin_catalog_read.sql` and
-- `0133_pages_per_role.sql`: guarded by a regex on the token, so running the
-- file again appends nothing, and it leaves a role that holds `*:*` alone.
--
-- IDEMPOTENT: every statement can run again (`IF NOT EXISTS`,
-- `CREATE OR REPLACE`, the guarded UPDATE), which the migration's test relies on.
CREATE TABLE IF NOT EXISTS usage_rate (
    id             UUID          PRIMARY KEY DEFAULT gen_random_uuid(),
    kind           TEXT          NOT NULL
        CHECK (kind IN ('scan_tb', 'engine_hour', 'storage_gb_month')),
    currency       TEXT          NOT NULL CHECK (currency ~ '^[A-Z]{3}$'),
    amount         NUMERIC(18,6) NOT NULL CHECK (amount >= 0),
    effective_from DATE          NOT NULL,
    created_by     TEXT          NOT NULL DEFAULT '',
    created_at     TIMESTAMPTZ   NOT NULL DEFAULT now()
);

-- The lookup of "the rate of this kind in force on this day".
CREATE INDEX IF NOT EXISTS usage_rate_kind_effective_idx
    ON usage_rate (kind, effective_from DESC, created_at DESC);

CREATE OR REPLACE FUNCTION usage_rate_refuse_change()
RETURNS TRIGGER AS $$
BEGIN
    RAISE EXCEPTION 'usage_rate is append-only: % is refused', TG_OP;
END;
$$ LANGUAGE plpgsql;

CREATE OR REPLACE TRIGGER usage_rate_refuse_update
    BEFORE UPDATE ON usage_rate
    FOR EACH ROW EXECUTE FUNCTION usage_rate_refuse_change();

CREATE OR REPLACE TRIGGER usage_rate_refuse_delete
    BEFORE DELETE ON usage_rate
    FOR EACH ROW EXECUTE FUNCTION usage_rate_refuse_change();

CREATE OR REPLACE TRIGGER usage_rate_refuse_truncate
    BEFORE TRUNCATE ON usage_rate
    FOR EACH STATEMENT EXECUTE FUNCTION usage_rate_refuse_change();

UPDATE role
SET permissions = CASE
    WHEN trim(permissions) = '' THEN 'usage:read'
    ELSE permissions || ',usage:read'
END
WHERE name = 'Governance Admin'
  AND permissions !~ '(^|,)\s*\*:\*\s*(,|$)'
  AND permissions !~ '(^|,)\s*usage:read\s*(,|$)';
