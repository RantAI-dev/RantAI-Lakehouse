"""The vendored HubSpot source (`saas/vendored/hubspot/`, `SRC-3` V-HubSpot.1):
its HTTP goes only through the session it is given (AD3, AD4). No network: the
session is a fake that answers from a table."""

from __future__ import annotations

import ast
from pathlib import Path
from typing import Any

from dispar_orchestrate.saas.vendored.hubspot import helpers

VENDORED = Path(helpers.__file__).parent


class FakeSession:
    def __init__(self, answers: dict[str, Any]) -> None:
        self.answers = answers
        self.asked: list[tuple[str, dict | None]] = []

    def get_json(self, target: str, params: dict | None = None) -> Any:
        self.asked.append((target, params))
        return self.answers[target]


def test_no_vendored_hubspot_module_imports_an_http_library() -> None:
    forbidden = {"requests", "urllib3", "httpx", "http", "urllib.request"}
    for path in VENDORED.glob("*.py"):
        for node in ast.walk(ast.parse(path.read_text())):
            names = [a.name for a in node.names] if isinstance(node, ast.Import) else [node.module or ""] if isinstance(node, ast.ImportFrom) else []
            for name in names:
                assert name.split(".")[0] not in forbidden and "helpers.requests" not in name, f"{path.name} imports {name}"
            if isinstance(node, ast.ImportFrom) and node.module == "dlt.sources.helpers":
                assert all(a.name != "requests" for a in node.names), f"{path.name} imports dlt's requests"


def test_fetch_data_pages_through_the_session_and_follows_the_next_link_with_it() -> None:
    first = {"results": [{"id": "1", "properties": {"name": "a"}}], "paging": {"next": {"link": "https://api.example.invalid/crm/v3/objects/deals?after=1"}}}
    second = {"results": [{"id": "2", "properties": {"name": "b"}}]}
    session = FakeSession({"/crm/v3/objects/deals": first, "https://api.example.invalid/crm/v3/objects/deals?after=1": second})
    pages = list(helpers.fetch_data("/crm/v3/objects/deals", session, params={"limit": 100}))
    assert [[o["id"] for o in page] for page in pages] == [["1"], ["2"]]
    assert session.asked == [("/crm/v3/objects/deals", {"limit": 100}), ("https://api.example.invalid/crm/v3/objects/deals?after=1", None)]
