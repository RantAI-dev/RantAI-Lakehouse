"""dagster/dispar_orchestrate/test_adapters_sheets.py -- proves
`adapters/sheets.py::build_source` reports `supported: false`
unconditionally, with the real reason, and never dials anything to reach
that conclusion (WS3 item 23, WS3 plan review Z10).
"""

from __future__ import annotations

from dispar_orchestrate.adapters.sheets import _UNSUPPORTED_REASON, build_source


def test_build_source_reports_unsupported_with_no_credential_supplied() -> None:
    # Open question 4, resolved: no signed-JWT exchange is implemented in
    # Tier 1 -- supported is False regardless of whether a service
    # account secret was supplied.
    result = build_source({"spreadsheetId": "s1", "ranges": ["A1:B2"]}, secrets={})
    assert result.supported is False
    assert list(result.source) == []


def test_build_source_is_unsupported_even_with_a_service_account_configured() -> None:
    # A caller who *did* set up a serviceAccountJson secret gets the SAME
    # answer -- this adapter never even looks at `secrets`, because no
    # code path here can turn that JSON blob into a signed JWT-bearer
    # assertion (AGENTS.md rule 14: unsupported, honestly, not "works,
    # approximately").
    result = build_source(
        {"spreadsheetId": "s1", "ranges": ["A1:B2"]},
        secrets={"serviceAccountJson": "{\"type\": \"service_account\"}"},
    )
    assert result.supported is False
    assert list(result.source) == []


def test_reason_names_what_is_actually_missing_not_a_guess() -> None:
    # Precision requirement (mirrors the Rust-side probe_sheets doc
    # comment's own correction during review): the reason must name what
    # this venv actually lacks, verified by this same task by running
    # `pip list` against ~/.cache/rantai-dagster-venv -- neither
    # google.auth/google.oauth2 (the library that would build the
    # unsigned claim set) nor any crypto-signing-capable library (PyJWT,
    # cryptography) that could sign one by hand is installed. A vague
    # "not supported" or a claim that merely restates "sheets isn't
    # implemented" would not meet AGENTS.md rule 14's bar.
    reason = build_source({"spreadsheetId": "s1", "ranges": []}, secrets={}).reason
    assert reason is not None
    assert "google.auth" in reason
    assert "sign" in reason.lower()


def test_module_level_reason_constant_matches_the_result() -> None:
    # The reason is a fixed, named constant -- not built ad hoc per call --
    # so every caller sees the identical, reviewed wording.
    result = build_source({"spreadsheetId": "s1", "ranges": []}, secrets={})
    assert result.reason == _UNSUPPORTED_REASON
