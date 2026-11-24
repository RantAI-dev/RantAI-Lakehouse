#!/usr/bin/env python3
"""Tests for `ops/release/airgap_firewall.sh` (REL-12).

The script is the release proofs' air-gap cutoff, and it must hold two
properties at once:

- dockerd's root-owned outbound 80/443 is rejected in host OUTPUT, while
  the non-root GitHub Actions runner, loopback and container egress are
  NOT touched;
- the cutoff is idempotent and can always be removed, including when
  `ip6tables` is not installed.

No root and no real firewall are needed: `iptables`/`ip6tables` are stood
in for by a stateful script on PATH that records every call and answers
`-L`/`-C` from the state the enable call built. The proof jobs' wiring in
`.github/workflows/release.yml` is checked as well, so a future edit that
drops the `if: always()` restore or the must-fail pull is caught here.
"""

from __future__ import annotations

import os
import subprocess
import tempfile
import unittest
from pathlib import Path
from typing import Any

import yaml

REPO_ROOT = Path(__file__).resolve().parents[2]
RELEASE_DIR = Path(__file__).resolve().parent
SCRIPT = RELEASE_DIR / "airgap_firewall.sh"
WORKFLOW = REPO_ROOT / ".github" / "workflows" / "release.yml"

# A stateful stand-in for iptables/ip6tables. It records every invocation
# in $IPT_LOG and keeps the chains/rules it was asked to add in
# $IPT_STATE, so `-L` and `-C` report what a real kernel would. That is
# what makes the idempotency assertions meaningful instead of cosmetic.
IPTABLES_STUB = r'''#!/usr/bin/python3
import os
import sys

args = sys.argv[1:]
is_ipv6 = os.path.basename(sys.argv[0]) == "ip6tables"
log_path = os.environ.get("IPT_LOG6" if is_ipv6 else "IPT_LOG")
if log_path:
    with open(log_path, "a", encoding="utf-8") as handle:
        handle.write(" ".join(args) + "\n")

state_path = os.environ.get("IPT_STATE6" if is_ipv6 else "IPT_STATE")
chains: set[str] = set()
rules: list[str] = []
if state_path and os.path.exists(state_path):
    with open(state_path, encoding="utf-8") as handle:
        for line in handle:
            kind, sep, value = line.rstrip("\n").partition(" ")
            if not sep:
                continue
            if kind == "CHAIN":
                chains.add(value)
            elif kind == "RULE":
                rules.append(value)


def save() -> None:
    if not state_path:
        return
    with open(state_path, "w", encoding="utf-8") as handle:
        for chain in sorted(chains):
            handle.write("CHAIN " + chain + "\n")
        for rule in rules:
            handle.write("RULE " + rule + "\n")


commands = {"-N", "-L", "-C", "-A", "-I", "-D", "-F", "-X"}
command = next((arg for arg in args if arg in commands), None)
if command is None:
    sys.exit(0)
rest = args[args.index(command) + 1:]
target = rest[0] if rest else ""
full = " ".join(rest)

if command == "-N":
    chains.add(target)
elif command == "-L":
    sys.exit(0 if target in chains else 1)
elif command == "-C":
    sys.exit(0 if full in rules else 1)
elif command in ("-A", "-I"):
    rules.append(full)
elif command == "-D":
    if full in rules:
        rules.remove(full)
elif command == "-F":
    rules = [rule for rule in rules if rule.split(" ", 1)[0] != target]
elif command == "-X":
    chains.discard(target)
    rules = [rule for rule in rules if rule.split(" ", 1)[0] != target]
save()
'''

CUT_STEP = "Cut the runner off from the registries"
RESTORE_STEP = "Restore outbound firewall rules"
SCRIPT_PATH = "ops/release/airgap_firewall.sh"


class AirgapFirewallTest(unittest.TestCase):
    """The script's own behaviour, with the firewall tools stubbed out."""

    def setUp(self) -> None:
        self._directory = tempfile.TemporaryDirectory()
        self.root = Path(self._directory.name)
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.write_tool("iptables")
        self.write_tool("ip6tables")
        self.log4 = self.root / "iptables.log"
        self.log6 = self.root / "ip6tables.log"
        self.state4 = self.root / "iptables.state"
        self.state6 = self.root / "ip6tables.state"

    def tearDown(self) -> None:
        self._directory.cleanup()

    def write_tool(self, name: str) -> None:
        path = self.bin / name
        path.write_text(IPTABLES_STUB, encoding="utf-8")
        path.chmod(0o755)

    def run_script(self, action: str, *, ipv6: bool = True) -> subprocess.CompletedProcess[str]:
        # A PATH containing ONLY the stubs proves that a missing
        # `ip6tables` cannot accidentally resolve to the host's real one.
        environment = {
            "PATH": str(self.bin),
            "IPT_LOG": str(self.log4),
            "IPT_STATE": str(self.state4),
            "IPT_LOG6": str(self.log6),
            "IPT_STATE6": str(self.state6),
        }
        if not ipv6:
            (self.bin / "ip6tables").unlink(missing_ok=True)
        return subprocess.run(
            ["/bin/bash", str(SCRIPT), action],
            capture_output=True,
            text=True,
            env=environment,
            check=False,
        )

    def commands(self, *, ipv6: bool = False) -> list[str]:
        log = self.log6 if ipv6 else self.log4
        if not log.exists():
            return []
        return log.read_text(encoding="utf-8").splitlines()

    def assert_family(self, *, ipv6: bool, udp_reject: str) -> None:
        """The exact rules the job needs, for one IP family."""
        commands = self.commands(ipv6=ipv6)
        lo_rule = "-A LAKEHOUSE_AIRGAP -o lo -j RETURN"
        self.assertIn("-N LAKEHOUSE_AIRGAP", commands)
        self.assertIn("-I OUTPUT -j LAKEHOUSE_AIRGAP", commands)
        self.assertIn(lo_rule, commands)
        self.assertIn(
            "-A LAKEHOUSE_AIRGAP -m owner --uid-owner 0 -p tcp -m multiport "
            "--dports 80,443 -j REJECT --reject-with tcp-reset",
            commands,
        )
        self.assertIn(
            f"-A LAKEHOUSE_AIRGAP -m owner --uid-owner 0 -p udp --dport 443 "
            f"-j REJECT --reject-with {udp_reject}",
            commands,
        )
        # Every REJECT is owner-scoped: a blanket rule would cut the
        # runner's own HTTPS and hang the workflow (REL-12).
        rejects = [line for line in commands if " -j REJECT " in line]
        self.assertTrue(rejects)
        for line in rejects:
            self.assertIn("--uid-owner 0", line, line)
        # Loopback is accepted BEFORE anything else in the chain.
        self.assertLess(
            commands.index(lo_rule),
            min(commands.index(line) for line in rejects),
        )
        # No chain/rule in the FORWARD path: container egress (with its
        # `apk add` init containers) is REL-16's work, not ours.
        for line in commands:
            self.assertNotIn("DOCKER-USER", line)
            self.assertNotIn("-A FORWARD", line)
            self.assertNotIn("-I FORWARD", line)

    def test_enable_blocks_root_outbound_80_and_443_on_both_families(self) -> None:
        """TCP 80/443 and UDP 443 are rejected for uid 0, with lo spared."""
        result = self.run_script("enable")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assert_family(ipv6=False, udp_reject="icmp-port-unreachable")
        self.assert_family(ipv6=True, udp_reject="icmp6-adm-prohibited")

    def test_enable_twice_does_not_duplicate_the_chain_jump_or_rules(self) -> None:
        """Enabling again leaves one jump and one copy of each rule."""
        self.assertEqual(self.run_script("enable").returncode, 0)
        self.assertEqual(self.run_script("enable").returncode, 0)
        for ipv6 in (False, True):
            commands = self.commands(ipv6=ipv6)
            self.assertEqual(commands.count("-N LAKEHOUSE_AIRGAP"), 1)
            self.assertEqual(commands.count("-I OUTPUT -j LAKEHOUSE_AIRGAP"), 1)
            self.assertEqual(
                commands.count("-A LAKEHOUSE_AIRGAP -o lo -j RETURN"), 1
            )
            self.assertEqual(
                len(
                    [
                        line
                        for line in commands
                        if line.startswith("-A ") and " -j REJECT " in line
                    ]
                ),
                2,
            )

    def test_disable_removes_the_jump_and_deletes_both_chains(self) -> None:
        """Teardown undoes exactly what enable installed, then exits 0."""
        self.assertEqual(self.run_script("enable").returncode, 0)
        result = self.run_script("disable")
        self.assertEqual(result.returncode, 0, result.stderr)
        for ipv6 in (False, True):
            commands = self.commands(ipv6=ipv6)
            self.assertIn("-D OUTPUT -j LAKEHOUSE_AIRGAP", commands)
            self.assertIn("-F LAKEHOUSE_AIRGAP", commands)
            self.assertIn("-X LAKEHOUSE_AIRGAP", commands)
            state = self.state6 if ipv6 else self.state4
            self.assertEqual(state.read_text(encoding="utf-8"), "", ipv6)

    def test_disable_is_idempotent_when_no_chain_exists(self) -> None:
        """A restore on a host that was never cut off is still a no-op."""
        result = self.run_script("disable")
        self.assertEqual(result.returncode, 0, result.stderr)
        for ipv6 in (False, True):
            commands = self.commands(ipv6=ipv6)
            self.assertFalse([line for line in commands if " -D " in line])
            self.assertFalse([line for line in commands if " -F " in line])
            self.assertFalse([line for line in commands if " -X " in line])
        self.assertEqual(self.run_script("disable").returncode, 0)
        result = self.run_script("enable")
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertEqual(self.run_script("disable").returncode, 0)
        self.assertEqual(self.run_script("disable").returncode, 0)

    def test_missing_ip6tables_does_not_fail_ipv4_setup_or_teardown(self) -> None:
        """IPv6 is best-effort; the IPv4 cutoff is still installed."""
        result = self.run_script("enable", ipv6=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("ip6tables is unavailable", result.stderr)
        self.assert_family(ipv6=False, udp_reject="icmp-port-unreachable")
        self.assertEqual(self.commands(ipv6=True), [])
        result = self.run_script("disable", ipv6=False)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("-D OUTPUT -j LAKEHOUSE_AIRGAP", self.commands())


class ProofWiringTest(unittest.TestCase):
    """Both jobs enable the cutoff, prove it, and always restore it."""

    @classmethod
    def setUpClass(cls) -> None:
        with WORKFLOW.open(encoding="utf-8") as handle:
            cls.document: dict[str, Any] = yaml.safe_load(handle)

    def steps(self, job: str) -> list[dict[str, Any]]:
        return list(self.document["jobs"][job]["steps"])

    def step(self, job: str, name: str) -> dict[str, Any]:
        for candidate in self.steps(job):
            if candidate.get("name") == name:
                return candidate
        self.fail(f"{job} has no step named {name!r}")

    def test_both_proofs_enable_the_firewall_and_prove_the_cut(self) -> None:
        """The must-fail pull runs AFTER the firewall is enabled."""
        for job in ("install-proof", "update-proof"):
            with self.subTest(job=job):
                run = str(self.step(job, CUT_STEP).get("run") or "")
                enable = run.find("airgap_firewall.sh enable")
                pull = run.find("docker pull alpine:3.20")
                self.assertNotEqual(enable, -1, job)
                self.assertNotEqual(pull, -1, job)
                self.assertLess(enable, pull, job)
                self.assertIn("exit 1", run, job)
                # The hosts sinkhole stays, as belt-and-suspenders.
                self.assertIn("/etc/hosts", run)
                self.assertIn("registry-1.docker.io", run)

    def test_both_proofs_restore_the_firewall_with_if_always(self) -> None:
        """The restore is the last step and runs even after a failure."""
        for job in ("install-proof", "update-proof"):
            with self.subTest(job=job):
                steps = self.steps(job)
                names = [str(step.get("name")) for step in steps]
                self.assertEqual(names[-1], RESTORE_STEP, job)
                restore = steps[-1]
                self.assertEqual(restore.get("if"), "always()", job)
                run = str(restore.get("run") or "")
                self.assertIn(f"{SCRIPT_PATH} disable", run, job)
                self.assertIn("sudo", run, job)
                # Restore must come after the cut; a job that never cut
                # the runner off has nothing to restore.
                self.assertLess(names.index(CUT_STEP), len(steps) - 1, job)


if __name__ == "__main__":
    unittest.main()
