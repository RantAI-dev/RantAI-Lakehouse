-- ADM-11 (D1/D2, plan 2026-11-09-adm-11-usernames-and-first-password.md):
-- every user gets a unique username, and `app_user.email` becomes optional.
--
-- Why: sign-in worked by email only, and the email column was `NOT NULL`, so
-- an installation without a mail server had to invent addresses. From this
-- migration on, `app_user.username` is the required, case-insensitively
-- unique handle (sign-in accepts either; `lakehouse-store::identity` and
-- `lakehouse_auth::password::verify` compare through `lower(...)`).
--
-- Backfill (D2.2, ADM-11 review 1 B1): `username = lower(email)`, with any
-- character outside `[a-z0-9._@+-]` replaced by `-` and truncated to 250
-- characters. When two emails differ only in case or normalize to the same
-- base, the oldest row (`created_at`, then `id`) keeps the plain value
-- and the others get `-2`, `-3`, … in that order — the row_number IS
-- the suffix. No row is skipped: every `app_user` row at migration time has
-- an email (the column is still `NOT NULL` here), so every row gets a
-- username that passes the CHECK constraint.
--
-- `app_user_email_unique` is deliberately KEPT as it is (D2.4): rows may
-- already hold case-variant duplicates of each other, and dropping or
-- rewriting the constraint could fail an upgrade. New writes are checked
-- case-insensitively in the store (`lakehouse-store::identity`, ADM-11 D5),
-- and Postgres treats NULL emails as distinct in a UNIQUE constraint, so the
-- optional email needs no constraint change.

ALTER TABLE app_user ADD COLUMN username TEXT;

-- ADM-11 review 1 B1: backfill value replaces characters outside [a-z0-9._@+-]
-- with '-', truncated to 250 chars. Ranking and suffixing partition on that
-- cleaned base so collisions get -2, -3 suffixes.
WITH cleaned AS (
    SELECT id,
           created_at,
           left(regexp_replace(lower(email), '[^a-z0-9._@+-]', '-', 'g'), 250) AS base
    FROM app_user
),
ranked AS (
    SELECT id,
           base,
           row_number() OVER (
               PARTITION BY base
               ORDER BY created_at, id
           ) AS rn
    FROM cleaned
)
UPDATE app_user u
SET username = CASE
    WHEN r.rn = 1 THEN r.base
    ELSE r.base || '-' || r.rn::text
END
FROM ranked r
WHERE u.id = r.id;

ALTER TABLE app_user ALTER COLUMN username SET NOT NULL;

-- The sign-in/name-lookup comparisons run through `lower(username)`, so the
-- uniqueness that matters is on the lowered value, not the stored one.
CREATE UNIQUE INDEX app_user_username_lower_key ON app_user (lower(username));

-- ADM-11 D3 (proposed): 1 to 254 characters from `a-z 0-9 . _ @ + -`,
-- stored lowercase. 254 leaves room for an email used as a username. The
-- schema-level CHECK is the backstop; the one validator callers go through
-- is `lakehouse_store::identity::normalize_username`.
ALTER TABLE app_user ADD CONSTRAINT app_user_username_format
    CHECK (char_length(username) BETWEEN 1 AND 254
           AND username ~ '^[a-z0-9._@+-]+$');

ALTER TABLE app_user ALTER COLUMN email DROP NOT NULL;

-- Default username to lower(email) for legacy or idempotent seed scripts
-- (e.g. 0002_seed_identity.sql) that insert without specifying username.
CREATE OR REPLACE FUNCTION app_user_default_username()
RETURNS TRIGGER AS $$
BEGIN
    IF NEW.username IS NULL AND NEW.email IS NOT NULL THEN
        NEW.username := lower(NEW.email);
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

CREATE TRIGGER app_user_default_username_trigger
BEFORE INSERT ON app_user
FOR EACH ROW
EXECUTE FUNCTION app_user_default_username();
