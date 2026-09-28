import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "release"))
from prepare import select, stamp
from publish import publication, find_release
from assemble import assemble

SHA = "abc1234" + "0" * 33


class ReleaseTests(unittest.TestCase):
    def test_release_lookup_handles_pending_drafts_and_fails_closed(self):
        with patch("publish.subprocess.run") as run, patch("publish.api") as api:
            run.return_value = subprocess.CompletedProcess([], 0, '{"databaseId": 42}', "")
            api.return_value = {"id": 42, "draft": True}
            self.assertTrue(find_release("v0.1.0")["draft"])
            api.assert_called_once_with("releases/42")
            run.return_value = subprocess.CompletedProcess([], 1, "", "release not found\n")
            self.assertIsNone(find_release("v0.1.0"))
            run.return_value = subprocess.CompletedProcess([], 1, "", "HTTP 403: permission denied")
            with self.assertRaises(RuntimeError):
                find_release("v0.1.0")

    def test_cargo_wrapper_rejects_missing_lockfile(self):
        wrapper = Path(__file__).resolve().parents[1] / "release/cargo-locked.sh"
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text('[package]\nname = "lock-check"\nversion = "0.1.0"\nedition = "2021"\n')
            (root / "src").mkdir()
            (root / "src/main.rs").write_text("fn main() {}\n")
            result = subprocess.run([str(wrapper), "build", "--offline"], cwd=root, capture_output=True, text=True)
            self.assertNotEqual(result.returncode, 0)
            self.assertIn("--locked", result.stderr)
            self.assertFalse((root / "Cargo.lock").exists())

    def test_assembly_requires_matching_archives_and_embedded_checksums(self):
        spec = select("development", "0.1.0", SHA, "refs/heads/main", 17, 1)
        manifest = {"announcement_tag": spec["tag"], "announcement_is_prerelease": True,
                    "releases": [{"app_version": spec["version"]}]}
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary)
            hashes = []
            for target in spec["targets"]:
                archive = directory / f"solte-{target}.tar.xz"
                archive.write_bytes(target.encode())
                checksum = hashlib.sha256(archive.read_bytes()).hexdigest()
                hashes.append(checksum)
                archive.with_suffix(".xz.sha256").write_text(checksum)
                (directory / f"{target}-dist-manifest.json").write_text(json.dumps(manifest))
            installer = directory / "solte-installer.sh"
            installer.write_text("\n".join(hashes))
            output = directory / "complete"
            assemble(directory, spec, manifest, output)
            self.assertIn(SHA, (output / "release-notes.md").read_text())
            for line in (output / "sha256.sum").read_text().splitlines():
                checksum, name = line.split("  ")
                self.assertEqual(checksum, hashlib.sha256((output / name).read_bytes()).hexdigest())
            installer.write_text("missing checksums")
            with self.assertRaisesRegex(ValueError, "checksum mismatch"):
                assemble(directory, spec, manifest, directory / "bad-checksum")
            installer.write_text("\n".join(hashes))
            local = directory / f"{spec['targets'][0]}-dist-manifest.json"
            local.write_text(json.dumps(dict(manifest, announcement_tag="v0.0.0")))
            with self.assertRaisesRegex(ValueError, "Mixed build versions"):
                assemble(directory, spec, manifest, directory / "mixed")
            local.unlink()
            with self.assertRaises(FileNotFoundError):
                assemble(directory, spec, manifest, directory / "missing")

    def test_development_is_unique_and_always_prerelease(self):
        a = select("development", "0.1.0", SHA, "refs/heads/main", 17, 1)
        b = select("development", "0.1.0", SHA, "refs/heads/main", 17, 2)
        self.assertEqual(a["version"], "0.1.1-dev.17.1.gabc123400000")
        self.assertNotEqual(a["version"], b["version"])
        self.assertTrue(a["prerelease"])
        policy = publication(a, "push", "refs/heads/main", SHA, "abhineet-biju/solte")
        self.assertEqual(policy["make_latest"], "false")

    def test_stable_requires_exact_explicit_tag(self):
        for ref in ["refs/heads/main", "refs/tags/v0.2.1", "refs/tags/v0.2.0-rc.1", "refs/tags/0.2.0"]:
            with self.assertRaises(ValueError):
                select("stable", "0.2.0", SHA, ref, 1, 1)
        spec = select("stable", "0.2.0", SHA, "refs/tags/v0.2.0", 1, 1)
        self.assertFalse(spec["prerelease"])
        self.assertEqual(publication(spec, "push", spec["source_ref"], SHA, "abhineet-biju/solte")["make_latest"], "legacy")

    def test_untrusted_or_check_events_cannot_publish(self):
        spec = select("development", "0.1.0", SHA, "refs/heads/main", 1, 1)
        for event, ref, source, repo in [("pull_request", "refs/heads/main", SHA, "abhineet-biju/solte"), ("push", "refs/heads/other", SHA, "abhineet-biju/solte"), ("push", "refs/heads/main", "f" * 40, "abhineet-biju/solte"), ("push", "refs/heads/main", SHA, "fork/solte")]:
            with self.assertRaises(ValueError):
                publication(spec, event, ref, source, repo)
        check = select("check", "0.1.0", SHA, "refs/heads/main", 1, 1)
        with self.assertRaises(ValueError):
            publication(check, "push", "refs/heads/main", SHA, "abhineet-biju/solte")
        forged = dict(spec, prerelease=False)
        with self.assertRaises(ValueError):
            publication(forged, "push", "refs/heads/main", SHA, "abhineet-biju/solte")

    def test_stamp_changes_only_root_versions(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text('[package]\nname = "solte"\nversion = "0.1.0"\n[dependencies]\nfoo = "1"\n')
            original = 'version = 4\n\n[[package]]\nname = "foo"\nversion = "1.0.0"\nsource = "registry+example"\nchecksum = "unchanged"\n\n[[package]]\nname = "solte"\nversion = "0.1.0"\ndependencies = ["foo"]\n'
            (root / "Cargo.lock").write_text(original)
            stamp(root, "0.1.1-dev.1.1.gabc123400000")
            self.assertEqual((root / "Cargo.lock").read_text(), original.replace('name = "solte"\nversion = "0.1.0"', 'name = "solte"\nversion = "0.1.1-dev.1.1.gabc123400000"'))
            before = (root / "Cargo.lock").read_text()
            stamp(root, "0.1.1-dev.1.1.gabc123400000")
            self.assertEqual((root / "Cargo.lock").read_text(), before)

    def test_mismatched_lockfile_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "Cargo.toml").write_text('[package]\nname = "solte"\nversion = "0.1.0"\n')
            (root / "Cargo.lock").write_text('[[package]]\nname = "solte"\nversion = "0.2.0"\n')
            with self.assertRaises(ValueError):
                stamp(root, "0.1.1-dev.1.1.gabc123400000")


if __name__ == "__main__":
    unittest.main()
