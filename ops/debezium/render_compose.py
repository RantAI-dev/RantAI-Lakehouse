"""Render a per-connector Debezium Server `docker-compose.override.yml`-shaped
fragment from a connector's `GET /api/connectors/{id}/debezium-properties`
response (WS3 Phase G).

# What this module mirrors

`docker-compose.yml:2051`'s hand-written `debezium-server:` service is the
ONE static, statically-configured demo CDC connector this repo ships (P5/G4,
ADR 0007: "no dynamic per-connector provisioning" that phase) — pinned by
digest (R4: no versioned tag exists upstream for
`ghcr.io/memiiso/debezium-server-iceberg`), `depends_on:
g4-source-init`/`lakekeeper-authz-init` (both `service_completed_successfully`),
a named `lakehouse_debezium_data` volume, and a `subpath`-mounted, read-only
`lakehouse_oidc_tokens` volume carrying `/tokens/debezium.jwt`.
`render_compose_fragment` reproduces that EXACT shape for a dynamically
rendered `debezium-<sanitized-id>` service: same image digest (cited from
`docker-compose.yml:2053` as `DEBEZIUM_IMAGE_DIGEST`, a module-level constant
so a copy/paste drift at render time is impossible), same `depends_on` keys,
same volumes shape but with a per-connector data volume
(`lakehouse_debezium_data_<sanitized-id>` — two Debezium instances cannot
share one data directory) and a per-connector token subpath
(`debezium-<sanitized-id>.jwt`, distinct from the static demo connector's own
`debezium.jwt`).

# What this module does NOT do

It does not call `ops/debezium/deprovision_connector.sh` — deprovisioning is
triggered by `DELETE /api/connectors/{id}` (Phase B, already wired in Rust;
see that route's own doc comment for why it does NOT drop the replication
slot itself). This module's job is only the forward direction: bring a CDC
connector's Debezium service INTO existence as a compose fragment. It does
not resolve a `secretRef`, does not open a network connection, and does not
call Docker/Compose itself — it is a pure string-rendering function over an
already-fetched `properties_response`, the same "pure function, IO stays
with the caller" shape as
`rust/crates/lakehouse-store/src/cdc.rs`'s `render_debezium_properties`.

# The property that matters

A rendered compose fragment is a file on disk that other tooling reads. A
resolved credential written into it is a leak that outlives this process.
Every credential-shaped value stays an environment-variable reference
(`${ENV_VAR_NAME}`) end to end:

- the `environment:` block declares each name with compose's must-set form
  (`${NAME:?}`, per `AGENTS.md`'s compose rule: `${X:?}` for must-set,
  never a default token) — never resolved, never given a fallback;
- the rendered `command:` heredoc re-embeds the SAME `${NAME}` references
  (escaped to `$${NAME}` so compose's own interpolation does not consume
  them at render time and bake a resolved value into the file — the exact
  pattern `docker-compose.yml:2094`'s static service already uses for
  `$${LAKEKEEPER_TOKEN}` and friends);
- [`render_compose_fragment`] runs [`looks_like_a_resolved_secret`] over the
  caller-supplied `properties_response` text before rendering anything, and
  raises [`RenderComposeError`] rather than write a single byte to disk if
  the upstream API ever handed it something that looks like an already
  -resolved credential instead of a reference. Fail closed (`AGENTS.md`
  principle 3).

Slot and publication names are NOT derived from the connector id/slug here.
`ops/debezium/deprovision_connector.sh` (WS3 plan review X4) already made
this change on the deprovisioning side: slot/publication names are
registry-owned values from the connector's `dial`
(`dial.slotName`/`dial.publicationName`), not guessed from a slug. This
generator is that same caller's forward-direction counterpart — the rendered
`debezium.source.slot.name`/`debezium.source.publication.name` properties
come from whatever `properties_response["properties"]` already says (the
Rust side, `render_debezium_properties`, is what puts the dial's real slot
and publication names there), never re-derived from `connector_id` in this
module.
"""

from __future__ import annotations

import re

# Cited verbatim from `docker-compose.yml:2053` -- the static demo
# connector's own pinned image reference. A module-level constant, not
# re-typed by hand at render time, so a copy/paste drift between the two
# services is impossible.
DEBEZIUM_IMAGE_DIGEST = (
    "ghcr.io/memiiso/debezium-server-iceberg@sha256:"
    "c49ebdaae01762a5509804926710d6a831e45d56f70ce98cf69bac57cc6a6bf9"
)

# Mirrors `ConnectorSlug::new`'s validation
# (`rust/crates/lakehouse-api/src/routes/connectors.rs:615`'s
# `connector_slug_for_id`, cited in `ops/debezium/deprovision_connector.sh`'s
# own header comment) -- a connector id, after `-` -> `_`, must already be a
# valid slug: `^[a-z0-9][a-z0-9_]{0,62}$`.
_SANITIZED_ID_RE = re.compile(r"^[a-z0-9][a-z0-9_]{0,62}$")

# `${ENV_VAR_NAME}` references, as `debezium-properties` responses render
# them (WS0/Phase E) -- e.g. `${CONNECTOR_MYSQL_PASSWORD}`.
_ENV_VAR_REF_RE = re.compile(r"\$\{([A-Za-z_][A-Za-z0-9_]*)\}")


class RenderComposeError(Exception):
    """Raised when a connector id fails slug validation, or when
    `properties_response` looks like it carries an already-resolved
    credential instead of an `${ENV_VAR_NAME}` reference (fail closed --
    AGENTS.md principle 3)."""


def sanitize_connector_id(connector_id: str) -> str:
    """Mirror `connector_slug_for_id`'s `id.replace('-', '_')` transform,
    then validate the result against `ConnectorSlug::new`'s pattern.

    # Raises

    [`RenderComposeError`] if the transformed id does not match
    `^[a-z0-9][a-z0-9_]{0,62}$`.
    """
    sanitized = connector_id.replace("-", "_")
    if not _SANITIZED_ID_RE.match(sanitized):
        raise RenderComposeError(
            f"connector id {connector_id!r} does not sanitize to a valid "
            f"ConnectorSlug (got {sanitized!r}, expected "
            r"^[a-z0-9][a-z0-9_]{0,62}$"
        )
    return sanitized


def looks_like_a_resolved_secret(value: str) -> bool:
    """Independently-tested Python port of
    `lakehouse_store::connectors::looks_like_raw_secret`
    (`rust/crates/lakehouse-store/src/connectors.rs:386`) -- the SAME three
    checks (a `scheme://user@host` DSN shape, a PEM block, a three-part
    dot-separated JWT-looking string), plus the same "long unbroken
    alnum-ish run with no `:`" heuristic. Documented as intentionally
    duplicated logic citing the Rust original: this script has no FFI into
    Rust, and "loose is the safe direction" (a false positive just means a
    legitimate value gets rejected and the caller investigates) applies
    here exactly as it does there.
    """
    trimmed = value.strip()
    if "://" in trimmed and "@" in trimmed:
        return True
    if "BEGIN" in trimmed and "PRIVATE KEY" in trimmed:
        return True
    dot_parts = trimmed.split(".")
    if len(dot_parts) == 3 and all(
        part and all(c.isalnum() or c in "_-" for c in part) for part in dot_parts
    ):
        return True
    alnum_run = all(c.isalnum() or c in "+/=-_" for c in trimmed)
    if alnum_run and len(trimmed) >= 32 and ":" not in trimmed:
        return True
    return False


def _extract_env_var_names(properties_text: str) -> list[str]:
    """`${ENV_VAR_NAME}` reference names found in `properties_text`, in
    first-seen order, deduplicated -- these become this service's
    `environment:` keys."""
    seen: list[str] = []
    for match in _ENV_VAR_REF_RE.finditer(properties_text):
        name = match.group(1)
        if name not in seen:
            seen.append(name)
    return seen


def _escape_for_compose_command(text: str) -> str:
    """Escape `${` to `$${` so this text, once embedded inside a compose
    `command: |` block, survives compose's OWN `${VAR}` interpolation at
    parse time and reaches the container's shell as a literal `${VAR}` --
    the exact pattern the static `debezium-server` service already uses
    (`docker-compose.yml:2094`, `$${LAKEKEEPER_TOKEN}`). Without this
    escape, compose would resolve the reference against the HOST's
    environment at render time and bake a plaintext value into the
    container's command line -- exactly the leak this module exists to
    prevent.
    """
    return text.replace("${", "$${")


def render_compose_fragment(connector_id: str, properties_response: dict) -> str:
    """Render a `docker-compose.override.yml`-shaped fragment defining a
    `debezium-<sanitized-id>` service for `connector_id`, from a
    `GET /api/connectors/{id}/debezium-properties` response.

    Mirrors `docker-compose.yml:2051`'s static `debezium-server:` service
    (see this module's docstring for the exact correspondence). Every
    credential-shaped value in `properties_response["properties"]` stays an
    `${ENV_VAR_NAME}` reference all the way through: declared `${NAME:?}`
    in `environment:`, and re-embedded as `$${NAME}` (never resolved) in
    the rendered `command:` heredoc.

    # Raises

    [`RenderComposeError`] if `connector_id` does not sanitize to a valid
    `ConnectorSlug`, or if `properties_response["properties"]` looks like
    it carries an already-resolved credential
    ([`looks_like_a_resolved_secret`]) instead of an environment-variable
    reference -- fail closed rather than write a leak to disk.
    """
    properties_text = properties_response["properties"]
    if looks_like_a_resolved_secret(properties_text):
        raise RenderComposeError(
            "properties_response['properties'] looks like an already-resolved "
            "credential (DSN/PEM/JWT/raw-token shape), not an ${ENV_VAR_NAME} "
            "reference -- refusing to render a fragment that would leak it to disk"
        )

    sanitized_id = sanitize_connector_id(connector_id)
    service_name = f"debezium-{sanitized_id}"
    data_volume = f"lakehouse_debezium_data_{sanitized_id}"
    token_subpath = f"{service_name}.jwt"

    env_var_names = _extract_env_var_names(properties_text)
    if env_var_names:
        environment_lines = "\n".join(
            f"      {name}: ${{{name}:?}}" for name in env_var_names
        )
    else:
        environment_lines = "      {}"

    escaped_properties = _escape_for_compose_command(properties_text)
    # Indent every properties line to sit inside the heredoc.
    indented_properties = "\n".join(
        f"        {line}" if line else "" for line in escaped_properties.splitlines()
    )

    fragment = f"""\
# Rendered by ops/debezium/render_compose.py for connector
# {connector_id!r} -- mirrors docker-compose.yml:2051's static
# `debezium-server:` service. Every credential-shaped value below stays an
# ${{ENV_VAR_NAME}} reference: declared must-set (`${{NAME:?}}`) in
# `environment:`, never resolved, never given a default token.
services:
  {service_name}:
    profiles: ["dagster"]
    image: {DEBEZIUM_IMAGE_DIGEST}
    depends_on:
      g4-source-init:
        condition: service_completed_successfully
      lakekeeper-authz-init:
        condition: service_completed_successfully
    environment:
{environment_lines}
    volumes:
      - {data_volume}:/debezium/data
      - type: volume
        source: lakehouse_oidc_tokens
        target: /tokens/{token_subpath}
        volume:
          subpath: {token_subpath}
        read_only: true
    entrypoint: ["/bin/sh", "-c"]
    command:
      - |
        set -eu
        mkdir -p /debezium/config && cat > /debezium/config/application.properties <<EOF
{indented_properties}
        EOF
        exec /debezium/run.sh
    restart: unless-stopped

volumes:
  {data_volume}:
"""
    return fragment
