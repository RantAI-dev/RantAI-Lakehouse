# ADR 0017: how SAML sign-in is built

- Status: **Proposed**. The product owner decides.
- Date: 2026-11-09
- Backlog: `ADM-12`. The owner decided on 2026-11-09 that SAML is built as well as OIDC.

## Context

The API speaks OIDC only (`lakehouse-auth/src/oidc.rs`). SAML 2.0 assertions are signed with
XML Signature. Verifying them safely requires an XML canonicalisation and signature stack,
which is a long-standing source of authentication bypasses: signature wrapping and comment
injection in NameID. We should not write one.

The options that were checked on 2026-11-09:

1. **Dex as a SAML-to-OIDC bridge.** Ruled out: Dex's own documentation says its SAML
   connector is unmaintained and probably vulnerable to authentication bypass.
2. **Native, in the API, with `samael`** (version 0.0.x, MIT).
   - It needs `xmlsec1`, `libxml2`, `libxslt` and OpenSSL as C libraries in the API image and
     in CI.
   - Its docs.rs build has failed since 0.0.18.
   - We would own the SP flow (AuthnRequest, `InResponseTo`, audience, time checks, replay).
3. **Keycloak as a bridge**, behind a compose profile.
   - It is a maintained, widely used SAML SP. The API keeps speaking OIDC to it, and our
     existing OIDC code, tests and role map apply unchanged.
   - The cost is a Java container (about 500 MB of image and 512 MB or more of memory), its
     own Postgres schema, and upgrades.
   - Settings would write the SAML IdP through Keycloak's admin REST API with a service
     account.

## Decision

Not taken yet. The planner recommends **option 3 (Keycloak bridge)**:

- it keeps XML-signature code out of our API;
- the SAML stack is maintained by a project whose job that is;
- it reuses the OIDC path that `ADM-12` makes configurable in Settings.

Option 2 is the fallback if the owner refuses a Java container. If it is chosen, it needs its
own security review and a `cargo deny` exception review for the C dependencies.

## Consequences

- With option 3, a SAML installation runs one more container, and the plan must cover a
  compose `up` proof, Keycloak's own secrets (`${X:?}`, no defaults), and backups of its
  schema.
- With option 2, the API image grows by the C libraries and their CVE stream, and CI needs
  them.
- Either way, the plan that follows this ADR names the SAML fields in Settings (IdP metadata
  URL or XML, entity ID, attribute names for email, name and groups) and the acceptance
  checks against a real IdP.
