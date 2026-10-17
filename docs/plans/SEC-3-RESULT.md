# `SEC-3` result: sign-in against real identity providers

Measured 2026-10-16 by the developer on the build machine, under the heavy
lock, in the `sec3sso` compose project (ports remapped, `down -v` at the
end). The automated half is Keycloak; the two manual halves (Entra ID,
Google) are the product owner's to fill in below.

## Keycloak (automated) — `SEC-3-AC1`

Environment: `quay.io/keycloak/keycloak:26.8.1`, pinned by tag and digest;
the realm is rendered at run time from `ops/sso/realm.template.json` by
`ops/sso/render_realm.py` (no secret is stored in the repository). The gate
is `ops/g10/g10_sso_test.py`, run inside the compose network by the
`sso-test-runner` service. Run script and log (not in the repository):
`review-logs/sec3-t2.sh`, `review-logs/sec3-t2b.log`.

The gate's full output:

```
[g10] lakehouse-api /health: visible after 0.0s
[g10] keycloak discovery document: visible after 0.2s
[g10] discovery reachable; issuer and endpoints match the API's configuration
[g10] alice signed in through Keycloak; permissions=['catalog:read', 'lineage:read', 'query:read']
[g10] bob (no group) signed in with no mapped role; permissions=[]
[g10] a token minted for audience lakehouse-wrongaud was refused with 401
[g10] a token past its 5s expiry was refused with 401
[g10] PASS
```

| What `SEC-3-AC1` asks | Result |
| --- | --- |
| Signs in a user through the provider | Yes: user `alice` posts Keycloak's own login form (authorization code + PKCE S256) and lands with a session. |
| Maps a group to a role | Yes: `alice` is in group `analysts`, `OIDC_ROLE_MAP=analysts=Analyst`, and `/api/auth/me` reports `permissions=['catalog:read', 'lineage:read', 'query:read']`. |
| Refuses a wrong audience | Yes: an `id_token` minted for `lakehouse-wrongaud` is refused with `401`. |
| Refuses an expired token | Yes: a token whose `exp` is 5 s after `iat` is refused with `401` once it expires. |

Cost basis for the CI job's `timeout-minutes: 20`: cold `build` of
`lakehouse-api` 11:25:07→11:30:04, then `up`→gate 74 s; a warm rebuild
11:33:58→11:34:03 with `up`→gate 34 s. The gate itself is seconds; the time
is the image build, which CI replaces with the shared `lakehouse-api` image.

Group-to-role mapping on Keycloak is exact only because the realm's
`oidc-group-membership-mapper` sets `full.path=false`; the default `true`
would emit `/analysts` and match no `OIDC_ROLE_MAP` key.

## Microsoft Entra ID (manual) — `SEC-3-AC2`

The product owner runs this after merge; record the date, the settings (no
secrets), the outcome and any notes. Settings and limits are in
`docs/OPERATIONS.md` → "Microsoft Entra ID".

| Date | Settings used | Outcome | Notes |
| --- | --- | --- | --- |
| | issuer, authorize/token URLs, `OIDC_JWKS_URL`, groups claim, redirect URI, mapped group id | | |

## Google (manual) — `SEC-3-AC3`

Settings and limits are in `docs/OPERATIONS.md` → "Google". `SEC-3-AC3` asks
only that a test user signs in; Google sends no groups claim, so no role
mapping is expected (D3).

| Date | Settings used | Outcome | Notes |
| --- | --- | --- | --- |
| | issuer, authorize/token URLs, `OIDC_JWKS_URL`, redirect URI | | |

## Not measured

- Entra ID and Google: not tested here (D2 makes them manual and
  owner-run); this page's two tables are empty on purpose.
- The overage case (an Entra user in more groups than the token can carry):
  documented as a limitation in `docs/OPERATIONS.md`, not exercised.
- Keycloak behind TLS, and any provider other than the three above.
