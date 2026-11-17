# REL-2 result: the first release through the pipeline (0.2.0)

Measured facts for `REL-2` slice 5 (plan
`docs/superpowers/plans/2026-10-31-rel-2-one-step-install-and-update.md`, section 4, S5).
Nothing here is estimated; each line names where it was read.

## The release

| Fact | Value | Source |
| --- | --- | --- |
| Tag | `v0.2.0`, on `main` `07fde627` (re-tagged 2026-11-03 so the release includes `REL-11`, the bundle in parts) | `git rev-parse v0.2.0^{commit}` |
| Release run | 41666401368, event `push` (tag), conclusion `success` | `gh run view 41666401368` |
| Approval | The `release` environment was approved for the product owner on 2026-11-04 at their request ("i approve the release") | the run's deployment review |
| Draft job | `Release · draft`, 01:53:43Z to 02:25:02Z on 2026-11-04 (31 min 19 s) | job 126022773198 |
| Longest steps | Build and push the API image 12 min 22 s; build the offline bundle 6 min 39 s; the SBOMs of the third-party images 2 min 10 s; create the draft release 1 min 35 s | the job's step times |
| Release state | Draft, not yet published; the GHCR packages are not yet public | `gh release view v0.2.0` |

## Sizes

| What | Bytes |
| --- | --- |
| Offline bundle, compressed, whole | 3,795,160,555 (about 3.53 GiB) |
| Part 00 | 1,992,294,400 (1900 MiB) |
| Part 01 | 1,802,866,155 |
| All 30 release assets together | 3,824,438,330 |

The bundle is in two parts because GitHub refuses a release asset of 2 GiB or more
(`REL-11`, merged as "REL-7" in #359).

## Install time

The tag run does not install anything: the two proofs (`install-proof`, `update-proof`) run
on pull requests, and on the tag run they were skipped. The install proof prints no timing of
its own, so the figure below is the step's own duration as GitHub records it.

| Run | What | Time | Notes |
| --- | --- | --- | --- |
| 41750494213 (pull request, 2026-11-04) | `Install the bundle`: `lakehousectl install` from the offline bundle, with the runner cut off from the registries | 1 min 44 s (01:26:36Z to 01:28:20Z) | A GitHub-hosted runner, the dry run's bundle and the dry run's key, not the signed 0.2.0 bundle. Downloading the bundle took a further 1 min 43 s and is not counted |

The install of the signed 0.2.0 bundle on a customer-like host is **not measured**: no such
install has been run yet. It is the first item of the product owner's acceptance checklist.

## The assets

| Asset | Bytes |
| --- | --- |
| `alpine.cdx.json` | 50934 |
| `aws-cli.cdx.json` | 2358656 |
| `backup-local.sh` | 14784 |
| `backups.xml` | 2070 |
| `catalog.cdx.json` | 283854 |
| `clickhouse-server.cdx.json` | 1460958 |
| `curl.cdx.json` | 122111 |
| `debezium-server-iceberg.cdx.json` | 4056799 |
| `gotenberg.cdx.json` | 5619380 |
| `iam.template.json` | 2362 |
| `install.sh` | 4196 |
| `lakehousectl-linux-x86_64` | 2155296 |
| `openfga.cdx.json` | 358001 |
| `optimize_bronze.sh` | 2903 |
| `postgres.cdx.json` | 2356120 |
| `rantai-lakehouse-0.2.0-linux-x86_64.tar.zst.part-00` | 1992294400 |
| `rantai-lakehouse-0.2.0-linux-x86_64.tar.zst.part-01` | 1802866155 |
| `rantai-lakehouse-0.2.0-linux-x86_64.tar.zst.sha256` | 110 |
| `rantai-lakehouse-api.cdx.json` | 978388 |
| `rantai-lakehouse-console.cdx.json` | 498613 |
| `rantai-lakehouse-dagster.cdx.json` | 3485155 |
| `rantai-lakehouse-oidc-mock.cdx.json` | 955702 |
| `release.json` | 8484 |
| `release.json.sigstore.json` | 387 |
| `restore-from-backup.sh` | 33555 |
| `rustfs.cdx.json` | 240700 |
| `seaweedfs.cdx.json` | 391241 |
| `SHA256SUMS` | 2471 |
| `socat.cdx.json` | 75840 |
| `trino.cdx.json` | 3758705 |
