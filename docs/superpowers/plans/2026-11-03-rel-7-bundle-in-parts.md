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

## 5. Review (planner appends below)
