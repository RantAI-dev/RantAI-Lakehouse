# No default credentials in compose

Backlog `SEC-18`. Spec: [`../specs/sec-18.md`](../specs/sec-18.md).
Plan: `docs/superpowers/plans/2026-10-08-sec-18-no-default-credentials.md`.

## What the user can rely on

A fresh install never runs with a password, key or token that is written
in the repository. If a required secret is not set, the stack refuses to
start and says which one.

## Decisions

The product owner asked on 2026-10-08 for the Phase 0 security items to be
fixed as their specs say. The spec's Target table is taken as signed. The
choices below are the planner's, where the spec leaves room.

| # | Decision | Choice |
| --- | --- | --- |
| 1 | Which variables are must-set | Every one whose default today is a known credential: the PostgreSQL password, the object-store access and secret keys, the catalog's encryption key, and the SeaweedFS keys. |
| 2 | Variables that fall back to another secret | `CONNECTOR_PG_PASSWORD` and `CONNECTOR_S3_*` may still fall back to the must-set variable above, never to a literal. |
| 3 | Secrets whose default is empty | Stay as they are: an empty token already fails closed in the code. Not part of this item. |
| 4 | `.env.example` | Placeholders only (`change-me`-style words are not placeholders if the stack accepts them: the value is left empty with a comment saying how to generate one). |
| 5 | Developer convenience | No hidden defaults. A documented one-line command writes a local `.env` with generated values. |
| 6 | CI and gates | Set their own throwaway values where they build their environment, marked as CI-only. |

## Limits

- An existing install whose `.env` already sets these values is not
  affected. One that relied on the defaults stops starting until the
  values are set; the release note says so and how to keep the data (set
  the old value explicitly, then rotate).
- This does not rotate anything already deployed with a default.

## Acceptance checklist (product owner)

- [ ] `docker compose up` on a clean project without the variables refuses to start and names the missing variable
- [ ] With them set, a real `up` works
- [ ] `.env.example` holds no usable credential
