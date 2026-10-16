-- SRC-12 part C (decisions 6 and 12): the pending sign-ins of REST connectors
-- that use the OAuth authorization-code flow.
--
-- One row is one sign-in the console has started and the provider has not yet
-- returned from. It holds what the callback must know and the browser must
-- never see:
--
--   * `state_hash`    the SHA-256 (hex) of the random `state` the provider
--                     echoes back. Only the hash is stored, so a read of this
--                     table cannot be turned into a callback request.
--   * `connector_id`  the connector the sign-in is for.
--   * `principal_id`  the user who started it; the callback refuses any other
--                     signed-in user.
--   * `code_verifier` the PKCE verifier. It stays on the server: the provider
--                     only ever sees its S256 challenge.
--   * `expires_at`    10 minutes after the start, set by the database clock
--                     (the API never supplies a time).
--
-- A row is consumed by `DELETE ... RETURNING`, so a `state` works once: a
-- replay, an unknown or an expired `state` finds nothing. Rows of a deleted
-- connector go with it. Expired rows are removed whenever a new sign-in
-- starts, and a sign-in started again by the same user for the same connector
-- replaces the earlier one, so the table cannot grow without bound.
--
-- No token of any kind is stored here: the refresh token the callback obtains
-- is written to the sealed credential store (SRC-12 part A), and an access
-- token is never stored.

CREATE TABLE connector_oauth_state (
    state_hash    TEXT PRIMARY KEY
        CHECK (state_hash ~ '^[0-9a-f]{64}$'),
    connector_id  TEXT        NOT NULL REFERENCES connector (id) ON DELETE CASCADE,
    principal_id  TEXT        NOT NULL,
    code_verifier TEXT        NOT NULL
        CHECK (char_length(code_verifier) BETWEEN 43 AND 128),
    created_at    TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at    TIMESTAMPTZ NOT NULL
);

CREATE INDEX connector_oauth_state_expiry_idx
    ON connector_oauth_state (expires_at);
CREATE INDEX connector_oauth_state_owner_idx
    ON connector_oauth_state (connector_id, principal_id);
