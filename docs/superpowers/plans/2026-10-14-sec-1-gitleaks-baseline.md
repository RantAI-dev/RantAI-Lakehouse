# SEC-1: baseline the rotated findings in the gitleaks history scan

Plan by the planner, 2026-10-14. Base: `main`.

## Why

`gitleaks (full git history)` is red on every PR. That hides any new leak
behind the same red X.

On 2026-10-14 the product owner confirmed that every real secret it finds is
rotated:
- the eight staging values once committed in `.env.staging`, now repository
  secrets `STAGING_*` (#130);
- the internal LLM node key once in `src/services/clients/llm.ts`.

What remains in history can no longer be used. A history rewrite would break
every open branch, so the findings are baselined instead. Each one is listed
by its exact fingerprint (commit, file, rule, line), so the same secret in a
new commit, or any new secret, still turns the scan red.

## Findings to baseline (scan of `a13e5ea`, job 115776111835)

| Fingerprint | What | Why it may be ignored |
| --- | --- | --- |
| `1d2c45c…:.env.staging:generic-api-key:21,22,23,24,25,26,35` (7 lines) | Staging tokens and secret | Rotated (owner, 2026-10-14); `.env.staging` holds none since #130 |
| `8ac6ee2…:src/services/clients/llm.ts:rantai-llm-node-api-key:9` | Internal LLM node key | Rotated (owner, 2026-10-14) |
| `8ac6ee2…:src/services/clients/llm.ts:rantai-internal-lan-host:7`, `c5954ca…:clickhouse.ts:…:11`, `7075682…:dagster.ts:…:6`, `7075682…:api/ops/[kind]/route.ts:…:108`, `27d1e2b…:api/ai/build-status/route.ts:…:5` | Private LAN addresses in old commits | Not secrets. None of these files holds them today |
| `31ec837…:dagster/dispar_orchestrate/test_schema_observer.py:generic-api-key:26` | Test token literal | False positive; the literal is gone since #129 |
| `4ae1bcd…:ops/g6/src11_connector_operations_test.py:generic-api-key:576` | `password=matrix.CONNECTOR_MYSQL_PASSWORD` on branch `feat/phase-1-rest` (#144) | False positive: a reference to a CI fixture constant. The history scan reads every branch. #144 is asked to change the line so its working-tree scan is clean |

## Tasks (developer)

1. Add `.gitleaksignore` at the repository root.
   - Use the full 40-character fingerprints exactly as gitleaks prints them, one per line.
   - Put a `#` comment above each group naming what it is, the rotation date, and `SEC-1`/`SEC-13`.
   - Check gitleaks 8.30.1's documentation that `.gitleaksignore` comments are allowed and that the file is read from `--source` by default. If comments are not allowed, keep the reasons in this plan and `docs/CI.md` instead.
2. Update the `history-scan` comments and step summary in `.github/workflows/security.yml`, and the "history-scan" section of `docs/CI.md`.
   - They say the scan finds nothing, which is false.
   - Say instead that it now reports only findings not in the baseline, which findings were baselined and why (link this plan), and that a new finding turns it red.
   - Keep the job separate and non-required. Do not add `continue-on-error`. Do not touch the working-tree job or `.gitleaks.toml`.
3. If `gitleaks` 8.30.1 can be installed locally, download it to a temp directory and run both of these at the base commit. Quote the counts.
   - `gitleaks detect --source . --config .gitleaks.toml --redact`, expecting 0 findings for history. If a fingerprint doesn't match, fix it from the output.
   - `gitleaks detect --no-git --source . --config .gitleaks.toml --redact`.
   - If it cannot be installed, say *not verified*.

## Not included

- A history rewrite.
- Changing gitleaks rules.
- The `CONNECTOR_*`/storage defaults that staging still takes from `.env.example` (`SEC-18`, PR #83).
