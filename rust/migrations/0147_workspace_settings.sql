-- The workspace's own name, logo, dark-mode logo and browser-tab icon, saved
-- from Settings → General (`ADM-2`, slice A: the settings and their routes).
--
-- Why a table: the console's name and pictures were fixed in code and static
-- files, so a customer could not put its own name on the sign-in page.
-- `ADM-2` lets a holder of `settings:write` set them, and the console reads
-- them through `GET /api/branding` (slice B).
--
-- Why `bytea` and not a bucket object: each file is at most 512 KB (the two
-- logos) or 64 KB (the icon), the CHECKs below are the backstop for the API's
-- own cap, Postgres compresses large values on its own (TOAST), and one store
-- means one backup and no object to orphan (the reasoning of `0100_geo_map`).
--
-- One row at most: `singleton` is the primary key and can only be true, as in
-- `0129_sign_in_settings`. One name and one set of pictures per installation;
-- branding per organisation waits for `ADM-1` (ADR 0018). No row means
-- nothing was ever saved, and the console shows the product's own name and
-- pictures. Every column is nullable for the same reason: NULL is "not set".
--
-- Each picture is two columns, its bytes and its type, and the CHECK that
-- follows them says both are set or both are NULL, so a type without bytes
-- (or bytes without a type to serve them with) can never be stored. The type
-- list is closed here, too: the logos take PNG, JPEG or WebP and the icon
-- takes PNG only. The API decides the type from the file's first bytes and
-- never from the file name or the part's `Content-Type`; these CHECKs only
-- keep a value outside the list out of the row.
--
-- `updated_by` is the principal id of whoever saved last.
CREATE TABLE IF NOT EXISTS workspace_settings (
    singleton       BOOLEAN     PRIMARY KEY DEFAULT TRUE CHECK (singleton),
    workspace_name  TEXT        CHECK (workspace_name IS NULL OR char_length(workspace_name) BETWEEN 1 AND 60),
    logo_bytes      BYTEA       CHECK (logo_bytes IS NULL OR octet_length(logo_bytes) BETWEEN 1 AND 524288),
    logo_type       TEXT        CHECK (logo_type IS NULL OR logo_type IN ('image/png', 'image/jpeg', 'image/webp')),
    logo_dark_bytes BYTEA       CHECK (logo_dark_bytes IS NULL OR octet_length(logo_dark_bytes) BETWEEN 1 AND 524288),
    logo_dark_type  TEXT        CHECK (logo_dark_type IS NULL OR logo_dark_type IN ('image/png', 'image/jpeg', 'image/webp')),
    icon_bytes      BYTEA       CHECK (icon_bytes IS NULL OR octet_length(icon_bytes) BETWEEN 1 AND 65536),
    icon_type       TEXT        CHECK (icon_type IS NULL OR icon_type = 'image/png'),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_by      TEXT        NOT NULL DEFAULT '',
    CHECK ((logo_bytes IS NULL) = (logo_type IS NULL)),
    CHECK ((logo_dark_bytes IS NULL) = (logo_dark_type IS NULL)),
    CHECK ((icon_bytes IS NULL) = (icon_type IS NULL))
);
