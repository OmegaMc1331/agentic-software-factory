import copy
import hashlib
import json
from pathlib import Path
import subprocess
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch
import zipfile

import gates
import package
import assemble
import dependencies


class QualificationBoundaries(unittest.TestCase):
    def test_generated_package_passes_the_release_checksum_verifier(self):
        verifier = package.ROOT / "scripts/release-checksums.mjs"
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            documents = root / "docs/v2/release"
            documents.mkdir(parents=True)
            for name in ["INSTALLATION", "UPGRADE", "SECURITY", "KNOWN-ISSUES", "USER-GUIDE"]:
                (documents / f"{name}.md").write_text("Fixture documentation\n", encoding="utf-8")
            (root / "scripts/v2_p14").mkdir(parents=True)
            for name in ["start-workspace.ps1", "start-workspace.sh"]:
                (root / "scripts/v2_p14" / name).write_text("Fixture launcher\n", encoding="utf-8")
            (root / "apps/dashboard").mkdir(parents=True)
            for name in ["LICENSE", "Cargo.lock", "rust-toolchain.toml", ".node-version", "apps/dashboard/package-lock.json"]:
                (root / name).write_text("Fixture content\n", encoding="utf-8")
            (root / "Cargo.toml").write_text('[workspace.package]\nversion = "1.0.0"\n', encoding="utf-8")
            binary = root / "factory.exe"
            binary.write_bytes(b"fixture executable")
            manifest = root / "build.json"
            manifest.write_text(json.dumps({"result": "PASS", "sourceUnchanged": True, "sourceCommit": "a" * 40, "sourceFingerprint": "b" * 64, "finalSourceFingerprint": "b" * 64, "binary": {"sha256": package.sha(binary)}, "embeddedAssets": []}), encoding="utf-8")

            def inventory(output):
                output.mkdir()
                for name in ["THIRD-PARTY-NOTICES.txt", "sbom.cdx.json", "license-review.json"]:
                    (output / name).write_text("Fixture inventory\n", encoding="utf-8")

            def git(*args):
                return {"status": "", "rev-parse": "a" * 40, "show": "1700000000"}[args[0]]

            output = root / "packages"
            with patch.multiple(package, ROOT=root, git=git, host=lambda _: None, tracked_fingerprint=lambda: "b" * 64, verify_machine=lambda *_: None, inventory=inventory), patch.object(package.subprocess, "check_output", return_value="factory 1.0.0\n"):
                package.build(SimpleNamespace(target="x86_64-pc-windows-msvc", output=output, binary=binary, build_record=manifest))
            archive = output / "factory-windows-x86_64.zip"
            verification = subprocess.run(["node", str(verifier), "verify", str(archive)], capture_output=True, text=True)
            self.assertEqual(verification.returncode, 0, verification.stderr)

    def test_upstream_notice_requires_matching_published_revision_and_file_hash(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            package_root = root / "crate"
            notices_root = root / "licenses"
            package_root.mkdir()
            notices_root.mkdir()
            (package_root / ".cargo_vcs_info.json").write_text(json.dumps({"git": {"sha1": "a" * 40}}))
            (notices_root / "LICENSE").write_bytes(b"actual license fixture")
            entry = {"purl": "pkg:cargo/fixture@1", "file": "LICENSE", "sha256": hashlib.sha256(b"actual license fixture").hexdigest(), "revision": "a" * 40, "url": "https://example.invalid/source/LICENSE"}
            self.assertEqual(len(dependencies.supplemental_notices(entry["purl"], package_root, notices_root, [entry])), 1)
            for mutation in [{"revision": "b" * 40}, {"sha256": "0" * 64}, {"file": "../LICENSE"}]:
                with self.assertRaises(ValueError):
                    dependencies.supplemental_notices(entry["purl"], package_root, notices_root, [{**entry, **mutation}])

    def test_incomplete_native_inventory_never_produces_a_distribution(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            with self.assertRaises(ValueError):
                assemble.assemble(root, root / "assembled", "a" * 40)
            self.assertFalse((root / "assembled").exists())

    def test_missing_unknown_or_inapplicable_gate_cannot_produce_go(self):
        data = {"sourceCommit": "a" * 40, "gates": [{"id": key, "status": "BLOCKED"} for key in gates.GATES]}
        with tempfile.TemporaryDirectory() as folder:
            self.assertEqual(gates.evaluate(data, Path(folder))["decision"], "NO-GO")
            missing = copy.deepcopy(data)
            missing["gates"].pop()
            with self.assertRaises(ValueError):
                gates.evaluate(missing, Path(folder))
            invalid = copy.deepcopy(data)
            invalid["gates"][0]["status"] = "probably passed"
            with self.assertRaises(ValueError):
                gates.evaluate(invalid, Path(folder))
            data["gates"][0]["status"] = "NOT_APPLICABLE"
            self.assertEqual(gates.evaluate(data, Path(folder))["decision"], "NO-GO")

    def test_pass_requires_current_evidence_and_verified_hash_without_path_escape(self):
        with tempfile.TemporaryDirectory() as folder:
            root = Path(folder)
            (root / "proof.json").write_text("{}", encoding="utf-8")
            item = {"path": "proof.json", "sha256": hashlib.sha256(b"{}").hexdigest(), "scope": "candidate", "sourceCommit": "a" * 40, "result": "PASS"}
            data = {"sourceCommit": "a" * 40, "gates": [{"id": key, "status": "PASS", "evidence": [item]} for key in gates.GATES]}
            self.assertEqual(gates.evaluate(data, root)["decision"], "NO-GO")
            for mutation in [{"sourceCommit": "b" * 40}, {"sha256": "0" * 64}, {"path": "../outside.json"}, {"scope": "historical"}]:
                invalid = copy.deepcopy(data)
                invalid["gates"][0]["evidence"][0].update(mutation)
                with self.assertRaises(ValueError):
                    gates.evaluate(invalid, root)

    def test_untrusted_package_paths_and_links_are_rejected_before_extraction(self):
        for name in ["../escape", "/absolute", "C:/outside", "directory\\escape"]:
            self.assertFalse(package.valid_member(name))
        for kind in ["traversal", "symlink", "duplicate"]:
            with tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                archive = root / "bad.zip"
                with zipfile.ZipFile(archive, "w") as source:
                    entry = zipfile.ZipInfo("../escape" if kind == "traversal" else "factory")
                    if kind == "symlink":
                        entry.create_system = 3
                        entry.external_attr = 0o120777 << 16
                    source.writestr(entry, b"not executable")
                    if kind == "duplicate":
                        source.writestr(entry, b"duplicate")
                with self.assertRaises(ValueError):
                    package.unpack(archive, root / "installed")
                self.assertFalse((root / "installed").exists())
                self.assertFalse((root / "escape").exists())

    def test_package_manifest_detects_missing_and_tampered_inventory(self):
        for entry in [{"path": "factory", "sha256": "0" * 64}, {"path": "missing", "sha256": hashlib.sha256(b"fixture").hexdigest()}]:
            with tempfile.TemporaryDirectory() as folder:
                root = Path(folder)
                archive = root / "bad.zip"
                with zipfile.ZipFile(archive, "w") as source:
                    source.writestr("factory", b"fixture")
                    source.writestr("PACKAGE-MANIFEST.json", json.dumps({"files": [entry]}))
                with self.assertRaises(ValueError):
                    package.unpack(archive, root / "installed")

    def test_binary_validation_never_accepts_another_platform_fixture(self):
        for target in package.TARGETS:
            with self.assertRaises(ValueError):
                package.verify_machine(b"fixture, not a native binary" + b"\0" * 128, target)


if __name__ == "__main__":
    unittest.main()
