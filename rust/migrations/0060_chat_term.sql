-- A user's own words for the chat: when the Copilot asks what a word means
-- and the user clicks an option, the click is stored here so the next chat
-- of that person does not ask again. One row per person and word.
--
-- `owner` is the same key Copilot sessions and `home_layout` (`0056`) are
-- scoped to (`routes::ai::session_owner`: the signed-in principal's id
-- rendered as a UUID string), as TEXT and not a foreign key to `app_user`.
-- Sessions live in `ClickHouse` and key on the same string, so one
-- ownership model covers per-person chat state, and a service identity or
-- a deleted user needs no cascade for what is only a remembered phrase.
-- A request with no principal never reaches this table (401), and every
-- store function binds `owner`, so no query reads or writes across owners.
--
-- Only a person's click writes a row; the model never does (the routes in
-- `routes::ai::terms`). A word typed in a chat answers that chat and is not
-- stored.
--
-- Every rule is a CHECK, so a row written around the store functions still
-- cannot crowd the prompt that later reads these rows: `term` is stored
-- trimmed and lower-cased (the store normalises it; the CHECK refuses a row
-- that is not) and is 1 to 60 characters, `meaning` is 1 to 200 characters
-- and not only whitespace, `question` (the message that led to the
-- question) is at most 500. The cap of 100 words per owner is a count, not
-- a row rule, so it is enforced by `chat_term::upsert` inside a
-- transaction that locks the owner, and not here.
CREATE TABLE chat_term (
    owner      TEXT        NOT NULL,
    term       TEXT        NOT NULL,
    meaning    TEXT        NOT NULL,
    question   TEXT        NOT NULL DEFAULT '',
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (owner, term),
    CONSTRAINT chat_term_term_normalised_check
        CHECK (term = lower(btrim(term))),
    CONSTRAINT chat_term_term_length_check
        CHECK (char_length(term) BETWEEN 1 AND 60),
    CONSTRAINT chat_term_meaning_length_check
        CHECK (char_length(meaning) BETWEEN 1 AND 200),
    CONSTRAINT chat_term_meaning_not_blank_check
        CHECK (btrim(meaning) <> ''),
    CONSTRAINT chat_term_question_length_check
        CHECK (char_length(question) <= 500)
);
