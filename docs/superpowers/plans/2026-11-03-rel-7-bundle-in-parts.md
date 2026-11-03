# REL-7: the offline bundle is published in parts, because a release file may not exceed 2 GiB

- **Planner:** Claude Opus, 2026-11-03. **Base:** `main` at `88a9481f` or later.
- **Branch:** `fix/rel-7-bundle-parts`. **Blocks:** publishing 0.2.0.

## 0. What is wrong, with evidence

- The `v0.2.0` release run `41554469838` built, signed and pushed every image. It then failed in "Create the draft release":
  ```
  HTTP 422: Validation Failed (…/releases/432229791/assets?…name=rantai-lakehouse-0.2.0-linux-x86_64.tar.zst)
  size must be less than 2147483648
  ```
- **The bundle is too big for one file.** `make_bundle.sh` wrote 3,794,642,569 bytes (3,618 MiB). Zstandard gets almost nothing back (3.59 GiB → 3.53 GiB) because image layers are already compressed. GitHub limits each release asset to 2 GiB, so the one-file bundle of REL-2 D11 cannot be attached.
- **The dry run did not catch it.** It builds the same bundle but never uploads it. The 0.1.0 measurement in the REL-2 plan was 72 MiB, before the bundle held every third-party image (REL-2 S2 round 6).
- **The draft release `v0.2.0`** exists with every asset except the bundle. It is a draft, so nobody can see it.

## 1. Decisions

- **D1. The release attaches the bundle in parts.**
  - After `make_bundle.sh` writes `rantai-lakehouse-<v>-linux-x86_64.tar.zst`, the release job splits it with `split -b 1900M -d -a 2` into `….tar.zst.part-00`, `.part-01` and so on. 1900 MiB leaves a margin under the 2 GiB limit.
  - It attaches the parts instead of the whole file.
  - The whole file's existing `.sha256` stays an asset, because the joined file must match it.
  - The asset list's `SHA256SUMS` lists every part as well.
- **D2. One way to join.**
  - `ops/release/install.sh`, when it fetches an offline bundle, downloads every part, checks each against `SHA256SUMS`, and joins them in name order (`cat …part-* > …tar.zst`). It then checks the whole file against its `.sha256` before unpacking. Any mismatch stops with a message that names the part.
  - If `install.sh` does not download bundles today, the docs say how:
    `cat rantai-lakehouse-<v>-linux-x86_64.tar.zst.part-* > rantai-lakehouse-<v>-linux-x86_64.tar.zst && sha256sum -c rantai-lakehouse-<v>-linux-x86_64.tar.zst.sha256 && tar --zstd -xf rantai-lakehouse-<v>-linux-x86_64.tar.zst`.
- **D3. `lakehousectl` does not change.** It already reads an unpacked bundle directory (`install --bundle <dir>`).
- **D4. The dry run proves the split.** It runs the same split and join and checks the joined file's sha256 against the bundle's. A bundle that does not round-trip fails the dry run.
- **D5. The release is idempotent on a re-run.** `gh release create` on a tag whose draft already exists must not fail. Use `gh release upload --clobber` when the draft exists, or delete and recreate the DRAFT only, never a published release. So the planner can re-run the job on the same tag once this merges.

## 2. Tasks

1. **T1. The split, the sums and the upload (D1, D5),** in `.github/workflows/release.yml`'s release job.
2. **T2. The join (D2),** in `install.sh` or the docs (`docs/OPERATIONS.md` and the install section of `README.md`), whichever owns the offline download today.
3. **T3. The dry run's round trip (D4).**
4. **T4. Tests.**
   - `ops/release/test_release_workflow.py` pins the split size, the parts in the asset list and the round-trip step.
   - `test_make_bundle.py` or a new test splits and joins a small file, and checks that a damaged part is refused.
   - *Accept:* `python3 -m pytest ops/release -q` (or the `uvx` fallback), `bash -n` and `shellcheck -S warning` on every touched script, and the PR's own `Release · dry run` green.

## 3. Out of scope

Making the bundle smaller (a separate item, if wanted), and any `lakehousectl` change.

## 4. Handoff (developer appends below)

**Developer report, 2026-11-03.** Branch `fix/rel-7-bundle-parts`, off `origin/main`
`d932256a` (the plan's `0855177e` is its own commit on this branch). Pushed after every
commit; no PR opened.

### Commits

| Commit | Task | What it does |
|---|---|---|
| `beaa9849` | T1 (D1, D5) | The release job splits the bundle at 1900 MiB with `split -b 1900M -d -a 2` straight into `release-assets/`, keeps the whole file's `.sha256` as an asset, lists the parts in `SHA256SUMS`, and the notes name the part count and size. `Create the draft release` writes into a DRAFT that already exists (`gh release upload --clobber` + `gh release edit`) and refuses a PUBLISHED release. |
| `2a292dbd` | T2 (D2) | `docs/OPERATIONS.md` gets the one way to join (check every part against `SHA256SUMS`, join in name order, check the joined file against its `.sha256`, unpack, run the bundle's own installer). `docs/CI.md` gets steps 9–10 and the round trip. `install.sh` is unchanged: it downloads the installer, not the bundle, and `lakehousectl` reads an unpacked directory (D3). |
| `eade7cc1` | T3 (D4) | The dry run's new step splits the bundle it just built, writes a `SHA256SUMS` over the parts, checks every part, joins them in name order and checks the joined file against `make_bundle.sh`'s `.sha256` — the operator's own commands. Also refuses a part at 2 GiB or more. |
| `cf47fd4c` | T4 | New `ops/release/test_bundle_parts.py`: a 2 KiB file split into three parts, `SHA256SUMS` written the release's way, the documented join run, a damaged part refused before the join and named, two releases' parts refused by the `.sha256`, and the docs/workflow/parts names pinned against each other. `test_release_workflow.py`: the split size and numbering, the parts in the asset list, no whole-bundle asset, the round-trip step and its order, the draft re-run — each with a synthetic workflow that must be reported. |
| `a744bafd` | T2 fixup | The join's last two commands unpack into `bundle/` and run `bundle/lakehousectl`, which is the file the bundle carries. |

**One deviation, stated plainly:** the dry run's round-trip step (T3) was in the working
tree when T1 was committed, so its FIRST form is in `beaa9849`. `eade7cc1` is the T3
change proper: it replaces that first form with the operator's commands (a per-part
`SHA256SUMS` check and `sha256sum -c "$name.sha256"`, instead of an `awk` comparison of
two digests). History was not rewritten to hide it — the branch is pushed after each
commit.

### Commands, as run

- `python3 -m pytest ops/release -q` — **265 passed, 1 warning** (on the final code, and
  on `cf47fd4c`; 111 of them the workflow suite before the REL-7 tests were added).
- `bash -n` on the three new/changed `run:` blocks, with `${{ … }}` replaced by `X` —
  `The release's assets`, `Create the draft release`, and the dry run's round-trip step:
  **ok, ok, ok**. The YAML itself parses (`yaml.safe_load`).
- `uvx --from shellcheck-py shellcheck -S warning` on the same three blocks: **rc 0 on
  each, no findings**. No tracked `*.sh` file was touched, so no script file needed these
  beyond the workflow blocks. (`shellcheck` is not installed on this machine; the `uvx`
  form the task names is what ran.)
- A local simulation of the round trip (`split -b 1900M -d -a 2`, per-part
  `sha256sum -c SHA256SUMS`, `cat …part-*`, `sha256sum -c …sha256`) on a 5,000,000-byte
  file: **1 part, both checks OK**.

### Not verified

- **The dry run itself.** The task says not to run the release workflow locally and that
  CI's `Release · dry run` proves it. The workflow's split/join commands were run by hand
  on a small file (above) and by `test_bundle_parts.py` on three parts, but the step has
  not run inside Actions, and the **real** 3.5 GiB bundle has not been split into its
  two parts on a runner.
- **The draft re-run against GitHub.** `gh release upload --clobber` into the existing
  `v0.2.0` draft, and the refusal of a published release, are pinned by a test on the
  step's text; neither has been exercised against the live API. The first re-run of the
  `v0.2.0` tag is the proof.
- **The product-owner acceptance checklist** for the release itself (REL-2-AC3 and the
  release's own checks) is untouched by this change.

## 5. Review (planner appends below)
