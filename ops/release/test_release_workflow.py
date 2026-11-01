#!/usr/bin/env python3
"""Tests for `.github/workflows/release.yml` (REL-2 S2 review BLOCKER 2).

The release pipeline is the one workflow in this repository that holds the
signing key, so two of its properties are checked here, on every commit, and
not only when a release is cut:

  - no `secrets.COSIGN_*` sits in a job-level `env:`, where EVERY step of
    the job could read it — a third-party action included;
  - every third-party action is pinned to a commit SHA, because a tag can
    move under the pipeline's feet and a commit cannot.

Two more are checked since REL-2 S2 D3a, for the same reason — a property
of the pipeline that only a release would otherwise prove:

  - the manifest's signature is a Sigstore bundle
    (`release.json.sigstore.json`), not a detached signature;
  - nothing is uploaded to the public transparency log (Rekor), in either
    job: the dry run would publish throwaway entries, and the manifest's
    signature must verify on an air-gapped host.

And two more since REL-2 S2 review round 4, from the PR's own dry run:

  - the SBOM folder is created before the SBOM action writes into it;
  - the signing key is not written to `$GITHUB_ENV`, which would put it in
    the environment of every later step of the job.

And one more since REL-2 S2 review round 5, from the PR's second dry run:

  - every SBOM is written as CycloneDX **JSON**, because `cosign attest
    --type cyclonedx` reads a JSON predicate and the action's plain
    `cyclonedx` format is XML.

The workflow is parsed with PyYAML (`pyyaml==6.0.3`, installed by the jobs
that run these tests). The checks are run against synthetic workflows too,
so a check that cannot fail is itself a failure. No network, no repository:
the tests read one file and write nothing.
"""

from __future__ import annotations

import re
import unittest
from pathlib import Path
from typing import Any

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
WORKFLOW_PATH = REPO_ROOT / ".github" / "workflows" / "release.yml"

# A third-party action, pinned: `owner/repo` and a 40-hex commit SHA.
# A SUB-ACTION (`owner/repo/path@<sha>`) is pinned the same way: it is
# the same repository, at the same commit, and the pipeline uses it to
# install the SBOM action's own syft (REL-2 S2 review round 6 BLOCKER 1).
PINNED_ACTION = re.compile(
    r"^[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+(/[A-Za-z0-9_.-]+)*@[0-9a-f]{40}$"
)

# The SBOM action, and the one step that is NOT it: the action's own
# `download-syft` sub-action installs syft, it does not write an SBOM.
SBOM_ACTION = re.compile(r"^anchore/sbom-action@")

# The three steps that need the signing key, and the two secrets of the
# `release` environment.
SIGNING_STEPS = (
    "Sign each image",
    "Attest each SBOM to its image",
    "Sign the manifest and verify the signature",
)
SECRET_NAMES = ("COSIGN_PRIVATE_KEY", "COSIGN_PASSWORD")

# The manifest's signature, as REL-2 S2 D3a decided it: a Sigstore bundle.
SIGSTORE_BUNDLE = "release.json.sigstore.json"

# The SBOM format every SBOM step must ask for: CycloneDX **JSON**.
# `anchore/sbom-action` takes syft's format names, and syft writes the
# plain `cyclonedx` name as XML — `cosign attest --type cyclonedx` reads a
# JSON predicate, so the XML answer is "invalid character '<' looking for
# beginning of value" (REL-2 S2 review round 5 BLOCKER 1).
SBOM_FORMAT = "cyclonedx-json"

# cosign v3's way to sign with a key and NO transparency log. Both flags
# are needed: v3 refuses `--tlog-upload=false` while the signing config
# is in effect, and that config names the public log service.
NO_TLOG_FLAGS = ("--use-signing-config=false", "--tlog-upload=false")

# A workflow with a signing secret in a job-level `env:`.
SYNTHETIC_JOB_SECRET = """
name: Synthetic
on: workflow_dispatch
jobs:
  release:
    runs-on: ubuntu-latest
    env:
      COSIGN_PASSWORD: ${{ secrets.COSIGN_PASSWORD }}
    steps:
      - name: Sign each image
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        run: cosign sign --key "$COSIGN_KEY" "$ref"
"""

# A workflow with the same secret in a STEP's `env:`, which is where it
# belongs: a step's environment is that step's alone.
SYNTHETIC_STEP_SECRET = """
name: Synthetic
on: workflow_dispatch
jobs:
  release:
    runs-on: ubuntu-latest
    steps:
      - name: Sign each image
        uses: actions/checkout@3d3c42e5aac5ba805825da76410c181273ba90b1 # v7.0.1
        env:
          COSIGN_PRIVATE_KEY: ${{ secrets.COSIGN_PRIVATE_KEY }}
        run: cosign sign --key "$COSIGN_KEY" "$ref"
"""

# A workflow with an action pinned to a tag that can move.
SYNTHETIC_MOVING_TAG = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The SBOM of the API image
        uses: anchore/sbom-action@v0
        run: syft packages
"""

# A workflow whose SBOM action writes into a folder nobody creates.
SYNTHETIC_NO_SBOM_FOLDER = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The SBOM of the API image
        uses: anchore/sbom-action@e22c389904149dbc22b58101806040fa8d37a610 # v0.24.0
        with:
          output-file: sbom/rantai-lakehouse-api.cdx.json
"""

# A workflow that puts the signing key into the environment of every
# later step of the job.
SYNTHETIC_KEY_IN_THE_GITHUB_ENV = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: Generate a throwaway signing key
        run: |
          {
            echo 'COSIGN_PRIVATE_KEY<<COSIGN_PRIVATE_KEY_EOF'
            cat cosign.key
            echo 'COSIGN_PRIVATE_KEY_EOF'
          } >> "$GITHUB_ENV"
"""

# A workflow whose SBOM is CycloneDX XML, which `cosign attest` cannot
# read as a predicate (REL-2 S2 review round 5 BLOCKER 1).
SYNTHETIC_XML_SBOM = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: The SBOM of the API image
        uses: anchore/sbom-action@e22c389904149dbc22b58101806040fa8d37a610 # v0.24.0
        with:
          format: cyclonedx
"""

# A workflow that never pulls the third-party images, so its bundle
# would hold only the four images this repository publishes
# (REL-2 S2 review round 6 BLOCKER 1).
SYNTHETIC_NO_THIRD_PARTY_PULL = """
name: Synthetic
on: workflow_dispatch
jobs:
  dry-run:
    runs-on: ubuntu-latest
    steps:
      - name: Build the offline bundle
        run: ops/release/make_bundle.sh --manifest release.json
"""


def load_workflow(path: Path = WORKFLOW_PATH) -> dict[str, Any]:
    """The workflow, parsed."""
    return yaml.safe_load(path.read_text(encoding="utf-8"))


def job_environments(document: dict[str, Any]) -> dict[str, dict[str, Any]]:
    """Every job's OWN `env:` mapping, and the workflow's.

    A step's `env:` is not in here: a step's environment is that step's
    alone, and the signing key may sit in one.
    """
    found: dict[str, dict[str, Any]] = {}
    workflow_env = document.get("env")
    if isinstance(workflow_env, dict):
        found["(workflow)"] = workflow_env
    for name, job in (document.get("jobs") or {}).items():
        env = job.get("env")
        if isinstance(env, dict):
            found[name] = env
    return found


def cosign_secrets_in_a_job_environment(document: dict[str, Any]) -> list[str]:
    """Every job-level environment that holds a signing secret."""
    found: list[str] = []
    for where, env in job_environments(document).items():
        for key, value in env.items():
            if "secrets.COSIGN_" in str(value):
                found.append(f"{where}.{key}")
    return found


def is_pinned(value: str) -> bool:
    """Whether an action is pinned to a commit SHA.

    A local action (`./path`) is code in this repository: it has no tag to
    move, so it needs no SHA.
    """
    return value.startswith("./") or bool(PINNED_ACTION.match(value))


def actions(document: dict[str, Any]) -> list[str]:
    """Every `uses:` of every step of every job."""
    found: list[str] = []
    for job in (document.get("jobs") or {}).values():
        for step in job.get("steps") or []:
            if "uses" in step:
                found.append(str(step["uses"]))
    return found


def unpinned_actions(document: dict[str, Any]) -> list[str]:
    """Every action that is NOT pinned to a commit SHA."""
    return [value for value in actions(document) if not is_pinned(value)]


def actions_without_a_version_comment(text: str, values: list[str]) -> list[str]:
    """Every action whose line does not name its release in a comment.

    The SHA is what runs; the comment says which release it is, so a
    reader — and the next bump — knows what was pinned.
    """
    missing: list[str] = []
    for value in values:
        for line in text.splitlines():
            if f"uses: {value}" not in line:
                continue
            if not re.search(r"#\s*v\d", line):
                missing.append(value)
            break
    return missing


def steps_of(document: dict[str, Any], job_name: str) -> list[dict[str, Any]]:
    """The steps of one job."""
    job = (document.get("jobs") or {}).get(job_name) or {}
    return list(job.get("steps") or [])


def step_named(document: dict[str, Any], job_name: str, name: str) -> dict[str, Any]:
    """One step, by its name."""
    for step in steps_of(document, job_name):
        if step.get("name") == name:
            return step
    raise AssertionError(f"no step named {name!r} in the job {job_name!r}")


def step_index(document: dict[str, Any], job_name: str, name: str) -> int:
    """Where a step sits in its job."""
    for index, step in enumerate(steps_of(document, job_name)):
        if step.get("name") == name:
            return index
    raise AssertionError(f"no step named {name!r} in the job {job_name!r}")


def third_party_pull_steps(document: dict[str, Any]) -> list[str]:
    """Every job that pulls the third-party images, by digest."""
    found: list[str] = []
    for job_name, job in (document.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            run = str(step.get("run") or "")
            if "compose_images.py" in run and "docker pull" in run:
                found.append(job_name)
                break
    return found


def is_sbom_step(step: dict[str, Any]) -> bool:
    """Whether a step writes an SBOM with the SBOM action.

    The action's `download-syft` SUB-action is not one: it installs
    syft, and the loop below runs it.
    """
    return SBOM_ACTION.match(str(step.get("uses") or "")) is not None


def creates_the_sbom_folder(document: dict[str, Any], job_name: str) -> bool:
    """Whether the job creates `sbom/` before its first SBOM step.

    The SBOM action writes into `sbom/` and does not create the folder,
    so a job that does not create it first fails with ENOENT.
    """
    steps = steps_of(document, job_name)
    first = next(
        (
            index
            for index, step in enumerate(steps)
            if is_sbom_step(step)
        ),
        None,
    )
    if first is None:
        return False
    return any("mkdir -p sbom" in str(step.get("run") or "") for step in steps[:first])


def shell_commands(run: str) -> str:
    """A `run:` block without its comment lines.

    A comment is not a command: the workflow explains itself in them,
    and the checks below are about what the steps DO.
    """
    return "\n".join(
        line for line in run.splitlines() if not line.strip().startswith("#")
    )


def sbom_steps(document: dict[str, Any]) -> list[tuple[str, dict[str, Any]]]:
    """Every SBOM step of every job, with the job it is in."""
    found: list[tuple[str, dict[str, Any]]] = []
    for job_name, job in (document.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            if is_sbom_step(step):
                found.append((job_name, step))
    return found


def sbom_formats(document: dict[str, Any]) -> list[str]:
    """Every SBOM step whose format is not CycloneDX JSON.

    `cosign attest --type cyclonedx` reads a JSON predicate, and the
    action's plain `cyclonedx` format is XML (REL-2 S2 review round 5
    BLOCKER 1).
    """
    found: list[str] = []
    for job_name, step in sbom_steps(document):
        with_ = step.get("with") or {}
        if str(with_.get("format")) != SBOM_FORMAT:
            found.append(f"{job_name}: {step.get('name')}")
    return found


def steps_writing_the_key_to_the_github_env(document: dict[str, Any]) -> list[str]:
    """Every step that writes the signing key into `$GITHUB_ENV`.

    A variable written to `$GITHUB_ENV` is in the environment of EVERY
    later step of the job, and the runner prints it in each step's
    environment block (REL-2 S2 review round 4 SHOULD-FIX 1).
    """
    found: list[str] = []
    for job_name, job in (document.get("jobs") or {}).items():
        for step in job.get("steps") or []:
            commands = shell_commands(str(step.get("run") or ""))
            if "$GITHUB_ENV" in commands and "COSIGN_PRIVATE_KEY" in commands:
                found.append(f"{job_name}: {step.get('name')}")
    return found


class ReleaseWorkflowTest(unittest.TestCase):
    """The release workflow's own properties."""

    @classmethod
    def setUpClass(cls) -> None:
        cls.text = WORKFLOW_PATH.read_text(encoding="utf-8")
        cls.document = yaml.safe_load(cls.text)

    def test_the_workflow_holds_the_dry_run_and_the_real_release(self) -> None:
        self.assertIn("dry-run", self.document["jobs"])
        self.assertIn("release", self.document["jobs"])

    def test_no_signing_secret_sits_in_a_job_level_environment(self) -> None:
        self.assertEqual(cosign_secrets_in_a_job_environment(self.document), [])

    def test_every_third_party_action_is_pinned_to_a_commit_sha(self) -> None:
        self.assertEqual(unpinned_actions(self.document), [])

    def test_every_pinned_action_names_its_release_in_a_trailing_comment(self) -> None:
        self.assertEqual(
            actions_without_a_version_comment(self.text, actions(self.document)),
            [],
        )

    def test_the_release_job_hands_the_key_to_the_three_signing_steps(self) -> None:
        for name in SIGNING_STEPS:
            step = step_named(self.document, "release", name)
            env = step.get("env") or {}
            for secret in SECRET_NAMES:
                self.assertEqual(
                    str(env.get(secret)),
                    "${{ secrets." + secret + " }}",
                    f"{name} does not carry {secret}",
                )

    def test_no_other_step_of_the_release_job_sees_a_signing_secret(self) -> None:
        seen = set()
        for step in steps_of(self.document, "release"):
            for value in (step.get("env") or {}).values():
                if "secrets.COSIGN_" in str(value):
                    seen.add(str(step.get("name")))
        self.assertEqual(seen, set(SIGNING_STEPS))

    def test_the_release_job_signs_with_the_key_from_the_environment(self) -> None:
        env = self.document["jobs"]["release"].get("env") or {}
        self.assertEqual(env.get("COSIGN_KEY"), "env://COSIGN_PRIVATE_KEY")
        for name in SIGNING_STEPS:
            step = step_named(self.document, "release", name)
            self.assertIn('--key "$COSIGN_KEY"', str(step.get("run")), name)

    def test_the_dry_run_signs_with_the_key_from_the_environment(self) -> None:
        env = self.document["jobs"]["dry-run"].get("env") or {}
        self.assertEqual(env.get("COSIGN_KEY"), "env://COSIGN_PRIVATE_KEY")
        for name in SIGNING_STEPS:
            step = step_named(self.document, "dry-run", name)
            self.assertIn('--key "$COSIGN_KEY"', str(step.get("run")), name)

    def test_the_dry_run_publishes_its_throwaway_key_as_a_step_output(self) -> None:
        """The key travels as an output, not as the job's environment.

        The real job's signing steps take the key from the `release`
        environment's secrets; the dry run's take it from the step that
        generated it (REL-2 S2 review round 4 SHOULD-FIX 1).
        """
        step = step_named(self.document, "dry-run", "Generate a throwaway signing key")
        run = str(step.get("run"))
        commands = shell_commands(run)
        self.assertIn("$GITHUB_OUTPUT", commands)
        self.assertNotIn("$GITHUB_ENV", commands)
        self.assertTrue(step.get("id"), "the step needs an id: its output carries the key")

    def test_no_step_writes_the_signing_key_into_the_github_environment(self) -> None:
        """REL-2 S2 review round 4 SHOULD-FIX 1: the key is not for every step.

        A variable written to `$GITHUB_ENV` is in the environment of every
        later step of the job, and the runner prints it in each step's
        environment block — the SBOM step's log showed the throwaway key.
        """
        self.assertEqual(steps_writing_the_key_to_the_github_env(self.document), [])

    def test_the_dry_run_hands_its_throwaway_key_to_the_three_signing_steps(self) -> None:
        """The dry run takes the same path as the real job.

        The key is in the `env:` of the three signing steps and of no
        other step: it comes from the step that generated it.
        """
        for name in SIGNING_STEPS:
            step = step_named(self.document, "dry-run", name)
            env = step.get("env") or {}
            self.assertIn("COSIGN_PRIVATE_KEY", env, name)

    def test_the_dry_run_job_environment_holds_no_key(self) -> None:
        """The job's environment holds the reference, not the key."""
        env = self.document["jobs"]["dry-run"].get("env") or {}
        self.assertNotIn("COSIGN_PRIVATE_KEY", env)

    def test_the_sbom_folder_is_created_before_the_first_sbom_step(self) -> None:
        """REL-2 S2 review round 4 BLOCKER 1: the action does not create it.

        `anchore/sbom-action` writes into `sbom/` and fails with ENOENT
        when the folder is not there, so both jobs create it first.
        """
        for job in ("dry-run", "release"):
            self.assertTrue(
                creates_the_sbom_folder(self.document, job),
                f"{job}: sbom/ is not created before the first SBOM step",
            )

    def test_a_workflow_without_the_sbom_folder_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_NO_SBOM_FOLDER)
        self.assertFalse(creates_the_sbom_folder(document, "dry-run"))

    def test_every_sbom_is_written_as_cyclonedx_json(self) -> None:
        """REL-2 S2 review round 5 BLOCKER 1: the predicate must be JSON.

        `anchore/sbom-action` takes syft's format names, and syft writes
        the plain `cyclonedx` name as XML. `cosign attest --type cyclonedx`
        reads a JSON predicate, so with the XML the attestation failed with
        "invalid character '<' looking for beginning of value".
        """
        for job in ("dry-run", "release"):
            steps = [step for name, step in sbom_steps(self.document) if name == job]
            self.assertTrue(steps, f"{job}: no SBOM step to check")
        self.assertEqual(sbom_formats(self.document), [])

    def test_a_workflow_with_an_xml_sbom_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_XML_SBOM)
        self.assertEqual(sbom_formats(document), ["dry-run: The SBOM of the API image"])

    def test_the_manifest_is_signed_into_a_sigstore_bundle_in_both_jobs(self) -> None:
        """REL-2 S2 D3a: the signature is a bundle, not a detached file.

        cosign v3 ignores `--output-signature` (it is deprecated), so a
        workflow that still used it would write no signature at all.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Sign the manifest and verify the signature")
            run = str(step.get("run"))
            self.assertIn(f"--bundle {SIGSTORE_BUNDLE}", run, job)
            self.assertNotIn("--output-signature", run, job)

    def test_the_bundle_is_verified_offline_in_both_jobs(self) -> None:
        """The verification right after signing must not need a log.

        The bundle carries no transparency-log entry (nothing is uploaded
        to Rekor), so the verification skips the log check and runs
        offline.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Sign the manifest and verify the signature")
            run = str(step.get("run"))
            self.assertIn("cosign verify-blob", run, job)
            self.assertIn("--insecure-ignore-tlog", run, job)
            self.assertIn("--offline", run, job)

    def test_no_step_of_either_job_uploads_to_the_transparency_log(self) -> None:
        """REL-2 S2 D3a: nothing is uploaded to the public Rekor log.

        Every cosign command that SIGNS something carries both flags, so
        neither job can publish an entry — the dry run's throwaway key
        least of all. The commands that only make or read a key
        (`generate-key-pair`, `public-key`) upload nothing and are not
        checked.
        """
        signing = re.compile(r"cosign (sign|sign-blob|attest)\b")
        for job in ("dry-run", "release"):
            for step in steps_of(self.document, job):
                run = str(step.get("run") or "")
                if not signing.search(run):
                    continue
                for flag in NO_TLOG_FLAGS:
                    self.assertIn(flag, run, f"{job}: {step.get('name')}: {flag}")

    def test_the_bundle_script_is_given_the_sigstore_bundle(self) -> None:
        """The bundle carries the bundle, under its own name."""
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Build the offline bundle")
            run = str(step.get("run"))
            self.assertIn(f"--sigstore-bundle {SIGSTORE_BUNDLE}", run, job)

    def test_both_jobs_pull_the_third_party_images_by_digest(self) -> None:
        """REL-2 S2 review round 6 BLOCKER 1: the bundle holds EVERY image.

        The third-party images are the ones the bundle was missing: the
        manifest names them, so the pipeline pulls them by digest and
        the bundle saves them.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "The third-party images, by digest")
            run = str(step.get("run"))
            self.assertIn("compose_images.py", run, job)
            self.assertIn("--published-images images.json", run, job)
            self.assertIn("docker pull", run, job)

    def test_a_workflow_without_the_third_party_pull_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_NO_THIRD_PARTY_PULL)
        self.assertEqual(third_party_pull_steps(document), [])

    def test_the_third_party_images_are_pulled_before_the_bundle_is_built(self) -> None:
        for job in ("dry-run", "release"):
            self.assertLess(
                step_index(self.document, job, "The third-party images, by digest"),
                step_index(self.document, job, "Build the offline bundle"),
                job,
            )

    def test_both_jobs_write_an_sbom_for_every_third_party_image(self) -> None:
        """Every image in a release carries an SBOM — the third-party
        ones included. The bundle refuses an image that has none."""
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "The SBOM of each third-party image")
            run = str(step.get("run"))
            self.assertIn("cyclonedx-json", run, job)
            self.assertIn("image_name.sh", run, job)
            self.assertIn("third-party-images.txt", run, job)

    def test_the_syft_the_pipeline_uses_is_the_sbom_action_own(self) -> None:
        """The same scanner, at the same commit, as the SBOM action.

        The action's own `download-syft` sub-action installs the syft
        the action itself uses (v1.42.3), so the SBOMs of the
        third-party images and the SBOMs of ours come from one scanner.
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Install syft")
            uses = str(step.get("uses"))
            self.assertIn("anchore/sbom-action/download-syft@", uses, job)
            self.assertTrue(step.get("id"), "the step needs an id: its output carries syft")
            scan = step_named(self.document, job, "The SBOM of each third-party image")
            self.assertIn("steps.syft.outputs.cmd", str(scan.get("env")), job)

    def test_both_jobs_free_the_runner_disk_before_the_images_are_pulled(self) -> None:
        """The bundle holds every image now: the runner needs the room."""
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Free runner disk")
            run = str(step.get("run"))
            self.assertIn("docker image prune", run, job)
            self.assertIn("df -h /", run, job)
            self.assertLess(
                step_index(self.document, job, "Free runner disk"),
                step_index(self.document, job, "The third-party images, by digest"),
                job,
            )

    def test_the_bundle_step_prints_the_disk_before_and_after(self) -> None:
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Build the offline bundle")
            run = str(step.get("run"))
            self.assertEqual(run.count("df -h /"), 2, job)

    def test_the_manifest_is_written_from_the_published_images_and_the_compose_file(self) -> None:
        """The manifest names every image the release runs.

        Ours come from the published digests, the third-party ones from
        the rendered Compose file — so the flag says "published".
        """
        for job in ("dry-run", "release"):
            step = step_named(self.document, job, "Write the release manifest")
            run = str(step.get("run"))
            self.assertIn("--published-images", run, job)
            self.assertIn("--compose docker-compose.release.yml", run, job)

    def test_a_job_level_secret_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_JOB_SECRET)
        self.assertEqual(
            cosign_secrets_in_a_job_environment(document),
            ["release.COSIGN_PASSWORD"],
        )

    def test_a_step_level_secret_is_not_a_job_level_one(self) -> None:
        document = yaml.safe_load(SYNTHETIC_STEP_SECRET)
        self.assertEqual(cosign_secrets_in_a_job_environment(document), [])

    def test_an_action_pinned_to_a_moving_tag_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_MOVING_TAG)
        self.assertEqual(unpinned_actions(document), ["anchore/sbom-action@v0"])

    def test_a_key_in_the_github_environment_is_reported(self) -> None:
        document = yaml.safe_load(SYNTHETIC_KEY_IN_THE_GITHUB_ENV)
        self.assertEqual(
            steps_writing_the_key_to_the_github_env(document),
            ["dry-run: Generate a throwaway signing key"],
        )


if __name__ == "__main__":
    unittest.main()
