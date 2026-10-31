# `REL-2` Install and update in one step, online or air-gapped

| | |
| --- | --- |
| Backlog | `REL-2` in [BACKLOG.md](../BACKLOG.md) |
| Module | Delivery |
| Size | L (planner's estimate: S = days, M = one to two weeks, L = several weeks), in four slices |
| Priority and status | In [BACKLOG.md](../BACKLOG.md), the one place they are kept |
| Blocked by | Nothing open. Four decisions below need the product owner |

## Why

The product owner (2026-10-30): installing and updating must be one step, or close to it, "like
T3 Code's installer and updater", and must work on an air-gapped server too. That means a signed
offline bundle, a backup before an update, and a way back. "State of the art, easy and reliable"
(2026-10-31).

## What is there today

Read at `main` `677845db`, from the code, not tested.

| Piece | Today |
| --- | --- |
| Install | Clone the repo, install Bun, run `ops/init-env.sh`, edit a 911-line `.env`, build every image from source (`scripts/compose.sh up --build`), then start the console by hand on the host (`README.md:89-149`). |
| Console | Not in `docker-compose.yml`. `Dockerfile.frontend` exists; only the staging deploy uses it. |
| Images | None is published. CI builds and throws them away (`docker.yml` `push: false`). Third-party images are pinned by tag, some floating (`alpine/socat:latest`, `postgres:16`, `mysql:8`). Nothing is pinned by digest except Debezium. |
| Signing | None. No image signature, no provenance. One SBOM, as a CI artifact. |
| Versions | One tag, `v0.1.0`. The SemVer policy in `reference/release-policy.md` is proposed, not signed. There is no version endpoint. |
| Update | Pull new code, rebuild, restart. Migrations run when the API starts, forward only. An old API refuses to start on a newer schema, so going back means restoring the database. |
| Backup | Scripts and a `backups` profile exist (`docs/OPERATIONS.md:1364-1530`), but the full backup needs an S3-compatible bucket, which an air-gapped site may not have. **The connector-secrets volume (`lakehouse_connector_secrets`) is in no backup**, although its compose comment says it must be backed up with Postgres. Losing it loses every stored connector credential. |
| Health | `GET /health` answers 200 whenever the process answers. There is no readiness check that a gate could trust. |

## How the best products do it (2026-10)

| Product | One-step install | Pre-flight | Signed offline bundle | Backup before update | Rollback |
| --- | --- | --- | --- | --- | --- |
| GitLab Omnibus | Package | Partial | Package file | **Automatic** | Restore |
| Replicated Embedded Cluster | One archive | **Host and app checks** | **Yes** | Snapshots | Snapshots |
| Zarf | One package | — | **Yes, verified offline** | — | — |
| Airbyte `abctl` | `curl \| bash` + one command | — | Images list only | — | — |
| Coolify | `curl \| bash` | **Disk, Docker version** | — | `.env` only | — |
| Sentry self-hosted | `install.sh` | Advisory | `docker save`, by hand | Advised | — |
| T3 Code | `npx t3@<version>` | — | — | — | Run the old version |

No product that runs on Docker Compose has all five. Replicated and Zarf have the signed bundle
and the checks, but they need Kubernetes. The target below takes GitLab's automatic backup and
upgrade stops, Zarf's offline-verified signed bundle, and Replicated's pre-flight checks, and adds
automatic rollback. All of it runs on Docker Compose, on one host.

## What users get

**Install, online:**

```sh
curl -fsSL https://<install address>/install.sh | sh
```

- The script downloads the `lakehousectl` command-line tool for the host, checks its checksum,
  and runs `lakehousectl install`.
- **Pre-flight checks:** the operating system and architecture, Docker and Compose versions, CPU,
  memory, free disk and free ports. Any failure stops the install and says what to fix.
- `lakehousectl` writes the configuration with every secret generated (no hand editing), pulls
  the release's images by digest, verifies their signatures, and starts the stack, the console
  included.
- It waits until the stack is ready, then prints the console address and the first
  administrator's sign-in. The password is shown once and must be changed at first sign-in.

**Install, air-gapped:**

- One file, `rantai-lakehouse-<version>-linux-<arch>.tar.zst`, carried in.
- Run `tar -xf` on it, then `./lakehousectl install --bundle .`.
- The bundle's signature and every image's checksum are verified with no network, against the
  release key built into `lakehousectl`. A changed byte is refused.

**Update:**

```sh
lakehousectl update               # online: the newest release
lakehousectl update --bundle .    # air-gapped
```

1. It verifies the new release.
2. It checks the upgrade path: a release can require a stop on the way, as GitLab's do.
3. It takes a backup automatically to local disk:
   - every Postgres database;
   - ClickHouse;
   - the connector secrets;
   - the configuration.
4. It moves to the new images and waits until the stack is ready.
5. **If the stack is not ready, it rolls back by itself**: it restores the backup, returns to the
   previous images, and checks again. It reports what happened.

**Other commands:**

- `lakehousectl rollback` returns to the previous release from its pre-update backup, and says
  first that data written since the update is lost.
- `lakehousectl backup` and `lakehousectl restore` work on demand, to local disk or an S3 bucket.
- `lakehousectl status` shows the version, each service's state, the backups, and whether an
  update exists (online).
- `lakehousectl self-update` updates the tool itself. An offline bundle carries its own copy.

**The console** shows its version, and says when the server it talks to has a different one, as
T3 Code does.

## Decisions the product owner makes

| # | Question | Planner's proposal |
| --- | --- | --- |
| 1 | The tool's name | `lakehousectl` |
| 2 | Where `install.sh` and online releases live | GitHub Releases on the public repo, plus a short address on rantai.dev that redirects there (for example `get.rantai.dev/lakehouse`) |
| 3 | Who holds the signing key | A cosign key pair. The private key lives in a GitHub Actions environment secret, released only to tag builds, with a required reviewer; the public key is built into `lakehousectl` and published. Keyless (Sigstore) is rejected: its offline verification is less simple. |
| 4 | The version policy | Sign `reference/release-policy.md` as proposed (SemVer, minor for any operator action), and cut **0.2.0** as the first release through the new pipeline |

## Target specs

Numbers marked *(proposed)* need the product owner's sign-off.

| Capability | Today | Target |
| --- | --- | --- |
| Install steps | About 10, from source | **One command**, with no question asked *(proposed)* |
| Time to a ready console | Not measured (source build) | **15 minutes or less** on a 4-core, 16 GB host, excluding download *(proposed; measured per release)* |
| Hand-edited settings | A 911-line `.env` | **None required**. Optional settings are passed as flags or edited later. |
| Images | Built on site | Published, pinned by digest, signed, each with an SBOM |
| Offline install | Not supported | One signed file, verified with no network |
| Pre-flight | None | OS and architecture, Docker 24 or newer, Compose 2.24 or newer, 4 or more CPUs, 16 GB or more RAM, 50 GB or more free disk, free ports *(proposed minimums)* |
| Update | Rebuild by hand | **One command** |
| Backup before update | By hand, needs S3 | **Automatic, to local disk**, covering the connector secrets too |
| Failed update | Manual restore | **Automatic rollback** to the previous release and its data |
| Upgrade path | Undefined | A release may name required stops, and the tool takes them. A direct update from the previous two minor releases always works *(proposed)* |
| Proof | None | CI installs the previous release, updates to the new one, and passes. It also forces a failed update and proves the rollback, on every release. |
| Hosts | Any (source build) | Linux x86_64 first; arm64 later *(proposed)* |
| Bundle size | n/a | Measured and published with each release, never assumed |

## Acceptance checklist

Run on a clean host by the product owner. A step not performed is never a pass.

- `REL-2-AC1` On a fresh Linux host with only Docker, the one install command gives a working
  console within the time target, with no file edited.
- `REL-2-AC2` On a host with no network, the bundle installs the same way. Changing one byte of
  the bundle makes the install refuse.
- `REL-2-AC3` A host below a minimum (for example 8 GB RAM) is refused, with a message that says
  what to fix.
- `REL-2-AC4` `lakehousectl update` from the previous release keeps every dashboard, pipeline and
  connector (its credential still works), and the console shows the new version.
- `REL-2-AC5` An update made to fail on purpose returns to the previous release with its data,
  and says so.
- `REL-2-AC6` `lakehousectl rollback` after a good update returns to the previous release.
- `REL-2-AC7` The release page lists the bundle with its size, SBOMs and signature, and the
  signature verifies with the published public key and `cosign`.

## Not included

- Kubernetes or Helm installs.
- Multi-host or high-availability layouts.
- Zero-downtime updates: the stack stops during an update.
- Windows and macOS hosts.
- Automatic updates without an operator running the command.
- Reversible database migrations: going back restores the backup.

## Asking the assistant

No. This is an operator tool.

## Before anyone builds it

The four decisions and the *(proposed)* numbers need the product owner's sign-off. The plan is
`docs/superpowers/plans/2026-10-31-rel-2-one-step-install-and-update.md`, and its first slice
does not depend on the decisions.
