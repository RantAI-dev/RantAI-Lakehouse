-- Login-throttle storage (SEC-2, login-protection-and-session-cleanup plan).
-- `key_hash` is a SHA-256 of the submitted email (trimmed, lower-cased) --
-- no email address is ever stored for accounts that do not exist, and the
-- hash is non-reversible so a database dump does not reveal which addresses
-- were tried. Same reason no `email TEXT` column exists here.
-- `failures` is the count within the current window, CHECK (>= 0).
-- `window_started_at` resets when the window passes without reaching the
-- limit, or when `clear` is called on a successful login.
-- `locked_until` is `NULL` when not locked, otherwise a time after which
-- the key may be tried again.
CREATE TABLE IF NOT EXISTS login_throttle (
    key_hash TEXT PRIMARY KEY,
    failures INT NOT NULL CHECK (failures >= 0),
    window_started_at TIMESTAMPTZ NOT NULL,
    locked_until TIMESTAMPTZ
);