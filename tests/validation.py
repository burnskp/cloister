"""Small fixtures for shared validation discovery, failure propagation, and audit scope."""

import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import unittest

SOURCE = Path(__file__).resolve().parent.parent


class ValidationTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        shutil.copy(SOURCE / "Makefile", self.root)
        shutil.copytree(SOURCE / "scripts", self.root / "scripts")
        self.bin = self.root / "bin"
        self.bin.mkdir()
        self.env = dict(os.environ, PATH=f"{self.bin}:{os.environ['PATH']}")
        self.env["CALL_LOG"] = str(self.root / "calls")
        self.stub("git", "exit 0")
        self.stub("nix", '''
if [ "$1" = eval ]; then
    printf '%s' 'package-added treefmt test-cloister-worker-broker test-runtime-worker-broker test-added test-runtime-added'
else
    printf '%s\\n' "$*" >> "$CALL_LOG"
    if [ "$1" = flake ]; then exit 1; fi
fi
''')

    def stub(self, name, body):
        script = self.bin / name
        script.write_text(f"#!{shutil.which('sh')}\n{body}\n")
        script.chmod(0o755)

    def run_command(self, *args):
        return subprocess.run(args, cwd=self.root, env=self.env,
                              text=True, capture_output=True, timeout=30)

    def workspace(self, name):
        directory = self.root / "helpers" / name
        directory.mkdir(parents=True)
        (directory / "Cargo.toml").write_text("")
        (directory / "Cargo.lock").write_text("")
        return f"helpers/{name}"

    def test_new_checks_and_workspace_are_discovered(self):
        helper = self.workspace("new-helper")
        nested = self.root / helper / "member"
        nested.mkdir()
        (nested / "Cargo.toml").touch()
        result = self.run_command("make", "-n", "test")
        self.assertEqual(result.returncode, 0, result.stderr)
        build = result.stdout.splitlines()[-1]
        for name in ["test-added", "test-runtime-added", "test-cloister-worker-broker",
                     "test-runtime-worker-broker"]:
            self.assertEqual(build.count(f"checks.x86_64-linux.{name}"), 1)
        self.assertNotIn("package-added", build)
        result = self.run_command("make", "-n", "rust-test")
        self.assertEqual(result.stdout.count(helper), 1)
        self.assertNotIn("member", result.stdout)
        result = self.run_command("make", "-n", "test-eval", "TEST_CHECKS=test-only")
        self.assertNotIn("test-added", result.stdout)
        self.assertIn("test-only", result.stdout)

    def test_discovery_failure_does_not_build_the_default_package(self):
        self.stub("nix", "exit 1")
        result = self.run_command("make", "test")
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("Unable to discover flake checks", result.stderr)

    def test_check_attempts_every_category_and_fails(self):
        self.workspace("new-helper")
        result = self.run_command("make", "check")
        self.assertNotEqual(result.returncode, 0)
        calls = (self.root / "calls").read_text()
        for category in ["flake check", "rust-check.sh clippy", "rust-check.sh test", "rust-check.sh audit"]:
            self.assertIn(category, calls)
        self.assertNotIn("formatter", calls)

    def test_audit_exceptions_are_scoped_and_failures_propagate(self):
        self.stub("cargo-audit", 'printf "%s\\n" "$*" >> "$CALL_LOG"; exit "${AUDIT_STATUS:-0}"')
        workspaces = [self.workspace(name) for name in [
            "cloister-sandbox", "cloister-wayland-validate", "new-helper"]]
        result = self.run_command("sh", "scripts/rust-check.sh", "audit", *workspaces)
        self.assertEqual(result.returncode, 0, result.stderr)
        calls = (self.root / "calls").read_text().splitlines()
        self.assertEqual(len(calls), 3)
        for call in calls:
            self.assertIn("--deny unsound", call)
        for call in calls[:2]:
            self.assertIn("--ignore RUSTSEC-2026-0194 --ignore RUSTSEC-2026-0195", call)
        self.assertNotIn("--ignore", calls[2])
        self.env["AUDIT_STATUS"] = "1"
        result = self.run_command("sh", "scripts/rust-check.sh", "audit", *workspaces)
        self.assertNotEqual(result.returncode, 0)
        self.assertEqual(len((self.root / "calls").read_text().splitlines()), 6)

    def test_missing_or_untracked_lockfile_fails(self):
        helper = self.workspace("new-helper")
        (self.root / helper / "Cargo.lock").unlink()
        result = self.run_command("sh", "scripts/rust-check.sh", "test", helper)
        self.assertNotEqual(result.returncode, 0)
        (self.root / helper / "Cargo.lock").touch()
        self.stub("git", "exit 1")
        result = self.run_command("sh", "scripts/rust-check.sh", "test", helper)
        self.assertNotEqual(result.returncode, 0)


if __name__ == "__main__":
    unittest.main()
