# `SEC-3` Single sign-on works against real identity providers

| | |
| --- | --- |
| Backlog | `SEC-3` in [BACKLOG.md](../BACKLOG.md) |
| Module | Administration & Security |
| Size | M |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |

## Why

Browser sign-in through OIDC (authorization code with PKCE,
`/api/auth/oidc/start` and `/api/auth/oidc/callback`) is built and passes
the G4 gate, but only against `ops/oidc-mock`, which says of itself that it
is not a real identity provider. Real providers differ in exactly the
places a mock does not exercise: the issuer string, where the signing keys
are published, which claims carry the email and the groups, and how a
group arrives (a name, or an opaque id). A customer's first sign-in must
not be our first test. (`README.md`'s claim that single sign-on is unbuilt
was corrected by `DOC-1`, #171.)

## Decisions (product owner, 2026-10-16)

| # | Decision |
| --- | --- |
| D1 | An automated gate signs in through Keycloak, a real OIDC server run as a container, on every relevant pull request. |
| D2 | Two manual sign-in tests, Microsoft Entra ID and Google, with test app registrations and test users the product owner creates. Their results are recorded with the date and the settings used, never the secrets. |
| D3 | Whatever a provider needs that the console cannot do is written down per provider, not worked around silently. |

## Target

| Capability | Today | Target |
| --- | --- | --- |
| Providers proven | `oidc-mock` only | Keycloak (automated), Entra ID and Google (manual, recorded) |
| Group to role mapping | Proven on the mock | Proven on Keycloak; on Entra ID with its group ids; Google: stated honestly (Google sends no groups claim) |
| Setup instructions | General | One section per provider in `docs/OPERATIONS.md`: the exact settings, and the known limits |

## Acceptance checklist

- `SEC-3-AC1` The Keycloak gate signs in a user, maps a group to a role, and refuses a user with a wrong audience or an expired token
- `SEC-3-AC2` A test user signs in with Microsoft Entra ID and lands with the mapped role (moved to `SEC-40`, the product owner's test, 2026-10-16)
- `SEC-3-AC3` A test user signs in with Google (moved to `SEC-40`)
- `SEC-3-AC4` `docs/OPERATIONS.md` lets an operator set up each of the three without reading code

## Not included

SAML; SCIM user provisioning; throttling of failed OIDC sign-ins (a separate
item if wanted).
