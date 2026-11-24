"""The code location's environment: the heartbeat TTL reaches the container (OPS-37).

`DAGSTER_CODE_SERVER_HEARTBEAT_TTL_SECONDS` was documented and read by
the guard (`code_server_guard.py`'s `HeartbeatTtlSettings.from_env`), but
nothing carried an operator's value into the container: the compose file
lists the `dagster-code-location` service's environment one variable at a
time and has no `env_file:`, so a setting in `.env` stopped at the host and
the guard always used its own default.

This file holds the guard rails for that path, and reads the REAL
`docker-compose.yml` of the repository root:

- the container's environment carries the variable, in the `${…:-}` form the
  other settings of that service use;
- the variable is given ONCE, because a YAML mapping with a repeated key
  keeps the LAST one, so a duplicate would be invisible to a plain parse;
- an empty value keeps the guard's own default of 300 seconds, which is
  what the template's empty default hands to the container.

The assertions are on the LITERAL `${…}` expression and not on a rendered
file, so a value that is unset in the reader's own environment cannot change
the answer (`ops/release/test_sec78_database_roles.py` reads the template the
same way, for the same reason).

No network, no container and no docker command: the template is a file, and
the guard's setting is read from this process's environment.

Run with:
`~/.venvs/rantai-lakehouse/bin/python -m pytest dagster/dispar_orchestrate/test_code_location_environment.py -q`
"""

from __future__ import annotations

from pathlib import Path

import pytest
import yaml

from dispar_orchestrate import code_server_guard

# The repository root: the compose template lives two levels above this
# file (`dagster/dispar_orchestrate/`), the way `test_file_ingest.py` and
# `test_parquet_ingest.py` reach the repository's own files.
REPO_ROOT = Path(__file__).resolve().parents[2]
COMPOSE_PATH = REPO_ROOT / "docker-compose.yml"

# The service and the variable this file is about.
CODE_LOCATION_SERVICE = "dagster-code-location"
HEARTBEAT_TTL_VARIABLE = "DAGSTER_CODE_SERVER_HEARTBEAT_TTL_SECONDS"

# The expression the template must carry, with the empty default the
# guard's own fallback needs: an operator who sets nothing in `.env` hands
# the container an empty value, and the guard treats it as unset.
HEARTBEAT_TTL_EXPRESSION = "${DAGSTER_CODE_SERVER_HEARTBEAT_TTL_SECONDS:-}"


class _RepeatedKeyLoader(yaml.SafeLoader):
    """A loader that RECORDS a repeated key instead of keeping the last one.

    `yaml.safe_load` keeps the LAST value of a repeated key, so a variable
    listed twice in the template would be invisible to a plain parse: the
    assertion would read the value that won and the one that lost would be
    gone. The names of the repeated keys are collected here, so a test can
    say that the variable is given once.

    A repeated key is a mistake the repository's own rules forbid
    (`docs/CODE-STANDARD.md`, "YAML mapping keys are unique per service"),
    and `docker compose config` refuses to read such a file. This loader
    catches it where a YAML parse alone would not.
    """

    def __init__(self, stream: str) -> None:
        super().__init__(stream)
        self.repeated_keys: list[str] = []

    def construct_mapping(self, node, deep=False):
        if not isinstance(node, yaml.MappingNode):
            raise yaml.constructor.ConstructorError(
                None,
                None,
                f"expected a mapping node, but found {node.id}",
                node.start_mark,
            )
        self.flatten_mapping(node)
        mapping = {}
        for key_node, value_node in node.value:
            key = self.construct_object(key_node, deep=deep)
            value = self.construct_object(value_node, deep=deep)
            if key in mapping:
                self.repeated_keys.append(str(key))
            mapping[key] = value
        return mapping


def _load_compose() -> tuple[dict, list[str]]:
    """The compose template, and the keys it repeats."""
    loader = _RepeatedKeyLoader(COMPOSE_PATH.read_text(encoding="utf-8"))
    try:
        document = loader.get_single_data()
    finally:
        loader.dispose()
    return document, loader.repeated_keys


def _code_location_environment() -> dict[str, str]:
    """The environment the template gives the code location's container."""
    document, _repeated_keys = _load_compose()
    return document["services"][CODE_LOCATION_SERVICE]["environment"]


def test_the_code_location_container_receives_the_heartbeat_ttl_from_the_environment() -> None:
    """The operator's value has to reach the container, or the setting is
    a line in the documentation and nothing else."""
    # The variable the guard reads and the variable the template passes must
    # be the SAME one, or the setting still stops at the host.
    assert HEARTBEAT_TTL_VARIABLE == code_server_guard.HEARTBEAT_TTL_ENV
    environment = _code_location_environment()
    assert HEARTBEAT_TTL_VARIABLE in environment, (
        "the code location's container is given no heartbeat TTL, so the "
        "operator's setting never reaches the guard"
    )
    assert environment[HEARTBEAT_TTL_VARIABLE] == HEARTBEAT_TTL_EXPRESSION


def test_the_heartbeat_ttl_is_given_to_the_code_location_only_once() -> None:
    """A variable listed twice keeps the LAST value, so the template would
    hold a setting with two answers and only one of them would be read."""
    _document, repeated_keys = _load_compose()
    assert HEARTBEAT_TTL_VARIABLE not in repeated_keys


def test_an_empty_heartbeat_ttl_keeps_the_guard_own_default(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    """The template's default is empty, so a deployment that sets nothing
    hands the container an empty value. The guard reads it as unset and
    uses its own default, instead of failing on a malformed number."""
    monkeypatch.setenv(code_server_guard.HEARTBEAT_TTL_ENV, "")
    # Dagster's own variable is the second one the guard reads, and the
    # reader's own environment may carry it: delete it, so this test says
    # what it means and not what the machine happens to hold.
    monkeypatch.delenv(code_server_guard.DAGSTER_HEARTBEAT_TTL_ENV, raising=False)

    settings = code_server_guard.HeartbeatTtlSettings.from_env()

    assert settings.ttl_seconds == code_server_guard.DEFAULT_HEARTBEAT_TTL_SECONDS
