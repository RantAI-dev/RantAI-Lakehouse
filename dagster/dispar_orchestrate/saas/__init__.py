"""The business-app (SaaS) connectors (`SRC-3`, plan
`docs/superpowers/plans/2026-10-19-data-phase-4-connectors.md`).

One adapter, `saas`, with a vendor field in the dial (AD1):

    vendors/    the fixed vendor table (a mirror of `ops/fixtures/saas_vendors.json`)
                and the registry of the per-vendor modules (AD5)
    session.py  `SaasSession`: every HTTP request of a vendor read, under the
                link rule (AD3, decision 8) and the rate-limit rule (decision 13)
    objects.py  what a vendor module declares per object: cursor, primary key,
                deletes, and why an object is re-read in full (decisions 6, 7)

The load itself is `adapters/saas.py`, which `ingest_factory` dispatches to.
"""
