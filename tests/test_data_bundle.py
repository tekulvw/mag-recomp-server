"""Synthetic-only archive trust-boundary tests; no extracted game inputs."""
import gzip
import hashlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tarfile
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
SCRIPT = ROOT / "tools/data-bundle.py"
MANIFEST = ".mag-data-manifest.json"


class DataBundleTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.inputs = self.root / "inputs"
        self.inputs.mkdir()
        (self.inputs / "catalog.json").write_text('{"synthetic":true,"entries":[1,2]}')

    def cli(self, *args, ok=True):
        result = subprocess.run([sys.executable, str(SCRIPT), *map(str, args)],
                                capture_output=True, text=True)
        self.assertEqual(result.returncode == 0, ok, result.stderr)
        return result

    def pack(self, name="bundle.tar.gz", **kwargs):
        archive = self.root / name
        self.cli("pack", "--input-dir", self.inputs, "--output", archive,
                 "--include", kwargs.get("include", "catalog.json"),
                 "--data-revision", "synthetic-v1", "--source-hash", "synthetic")
        return archive

    def unpack(self, archive, ok=True, *extra):
        output = self.root / "catalogs"
        output.mkdir(exist_ok=True)
        self.cli("unpack", "--archive", archive, "--sha256",
                 hashlib.sha256(archive.read_bytes()).hexdigest(),
                 "--output-dir", output, *extra, ok=ok)
        return output

    def malicious(self, members, manifest=None):
        data = b'{"synthetic":true}'
        if manifest is None:
            manifest = {"schema": "mag-data-bundle", "version": 1,
                        "compatibility": "operator-validated", "data_revision": "test",
                        "source_hash": "test", "files": [{"path": "catalog.json",
                        "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}]}
        archive = self.root / "malicious.tar.gz"
        with tarfile.open(archive, "w:gz", format=tarfile.USTAR_FORMAT) as tar:
            entries = [(MANIFEST, json.dumps(manifest).encode(), None)] + members
            for name, content, kind in entries:
                info = tarfile.TarInfo(name)
                info.size = len(content)
                if kind:
                    info.type = kind
                    info.linkname = "outside.json"
                tar.addfile(info, io.BytesIO(content))
        return archive

    def test_deterministic_roundtrip_and_readonly_files(self):
        first, second = self.pack(), self.pack("second.tar.gz")
        self.assertEqual(first.read_bytes(), second.read_bytes())
        output = self.unpack(first)
        self.assertEqual((output / "catalog.json").read_bytes(), (self.inputs / "catalog.json").read_bytes())
        self.assertEqual((output / "catalog.json").stat().st_mode & 0o777, 0o444)
        self.assertEqual(json.loads((output / MANIFEST).read_text())["data_revision"], "synthetic-v1")

    def test_checksum_failure_leaves_target_empty(self):
        archive = self.pack()
        output = self.root / "catalogs"
        output.mkdir()
        self.cli("unpack", "--archive", archive, "--sha256", "0" * 64,
                 "--output-dir", output, ok=False)
        self.assertEqual(list(output.iterdir()), [])

    def test_existing_output_never_overwritten(self):
        archive = self.pack()
        before = archive.read_bytes()
        self.cli("pack", "--input-dir", self.inputs, "--include", "catalog.json",
                 "--output", archive, ok=False)
        self.assertEqual(before, archive.read_bytes())
        target = self.root / "catalogs"
        target.mkdir()
        (target / "sentinel").write_text("preserve")
        self.unpack(archive, ok=False)
        self.assertEqual((target / "sentinel").read_text(), "preserve")

    def test_invalid_json_and_secret_data_are_rejected(self):
        for value in ['{"a":1,"a":2}', '{"a":NaN}', '{"a":1e999}',
                      '{"password":"synthetic"}', json.dumps({"x": "-----BEGIN " + "PRIVATE KEY-----"}), '{']:
            with self.subTest(value=value):
                (self.inputs / "catalog.json").write_text(value)
                self.cli("pack", "--input-dir", self.inputs, "--include", "catalog.json",
                         "--output", self.root / "invalid.tar.gz", ok=False)
                self.assertFalse((self.root / "invalid.tar.gz").exists())

    def test_unsafe_selection_is_rejected(self):
        for name in ["accounts.json", "characters.json", "credentials.json", "original.pak"]:
            (self.inputs / name).write_text("{}")
            self.cli("pack", "--input-dir", self.inputs, "--include", name,
                     "--output", self.root / "invalid.tar.gz", ok=False)
        (self.inputs / "link.json").symlink_to(self.inputs / "catalog.json")
        self.cli("pack", "--input-dir", self.inputs, "--include", "link.json",
                 "--output", self.root / "invalid.tar.gz", ok=False)
        self.cli("pack", "--input-dir", self.inputs, "--include", "../catalog.json",
                 "--output", self.root / "invalid.tar.gz", ok=False)

    def test_hostile_archive_members_leave_no_files(self):
        data = b'{"synthetic":true}'
        cases = [[("../outside.json", data, None)], [("/outside.json", data, None)],
                 [("catalog.json", b"", tarfile.SYMTYPE)],
                 [("catalog.json", b"", tarfile.LNKTYPE)],
                 [("catalog.json", b"", tarfile.CHRTYPE)],
                 [("catalog.json", data, None), ("catalog.json", data, None)],
                 [("extra.json", data, None)], [("catalog.json", b"{}", None)], []]
        for members in cases:
            with self.subTest(members=members):
                output = self.unpack(self.malicious(members), ok=False)
                self.assertEqual(list(output.iterdir()), [])
        self.assertFalse((self.root / "outside.json").exists())

    def test_resource_limits_and_symlink_target(self):
        archive = self.pack()
        output = self.unpack(archive, False, "--max-file-bytes", "8")
        self.assertEqual(list(output.iterdir()), [])
        self.unpack(archive, False, "--max-total-bytes", "8")
        self.unpack(archive, False, "--max-archive-bytes", "8")
        output.rmdir()
        output.symlink_to(self.inputs, target_is_directory=True)
        self.cli("unpack", "--archive", archive, "--sha256",
                 hashlib.sha256(archive.read_bytes()).hexdigest(), "--output-dir", output, ok=False)

    def test_nested_selection_manifest_and_symlink_parent(self):
        nested = self.inputs / "nested"
        nested.mkdir()
        (nested / "catalog.json").write_text('{"synthetic":true}')
        selection = self.root / "selection.json"
        selection.write_text('{"files":["nested/catalog.json"]}')
        archive = self.root / "selected.tar.gz"
        self.cli("pack", "--input-dir", self.inputs, "--selection-manifest", selection,
                 "--output", archive)
        output = self.unpack(archive)
        self.assertTrue((output / "nested/catalog.json").is_file())
        (self.inputs / "redirect").symlink_to(nested, target_is_directory=True)
        self.cli("pack", "--input-dir", self.inputs, "--include", "redirect/catalog.json",
                 "--output", self.root / "invalid.tar.gz", ok=False)

    def test_manifest_schema_and_limits_are_enforced(self):
        archive = self.pack()
        with tarfile.open(archive) as tar:
            original = json.load(tar.extractfile(MANIFEST))
        for change in [{"version": 2}, {"files": []}, {"extra": True},
                       {"files": original["files"] * 2},
                       {"files": [{"path": "catalog.json", "size": -1, "sha256": "0" * 64}]},
                       {"compatibility": "automatically-proven"}]:
            with self.subTest(change=change):
                bad = dict(original, **change)
                output = self.unpack(self.malicious([], bad), ok=False)
                self.assertEqual(list(output.iterdir()), [])

    def test_valid_hash_does_not_make_invalid_json_acceptable(self):
        for data in [b'{"x":1,"x":2}', b'{"x":Infinity}', b'{', b'{"private_key":"synthetic"}']:
            with self.subTest(data=data):
                manifest = {"schema": "mag-data-bundle", "version": 1,
                            "compatibility": "operator-validated", "data_revision": "test",
                            "source_hash": "test", "files": [{"path": "catalog.json",
                            "size": len(data), "sha256": hashlib.sha256(data).hexdigest()}]}
                output = self.unpack(self.malicious([("catalog.json", data, None)], manifest), ok=False)
                self.assertEqual(list(output.iterdir()), [])

    def test_extension_record_and_gzip_tail_are_rejected(self):
        output = self.unpack(self.malicious([("catalog.json", b"x", tarfile.XHDTYPE)]), ok=False)
        self.assertEqual(list(output.iterdir()), [])
        archive = self.pack()
        archive.write_bytes(archive.read_bytes() + gzip.compress(b"x" * 20000))
        self.unpack(archive, ok=False)

    def test_pack_refuses_git_checkout_output(self):
        checkout = self.root / "checkout"
        checkout.mkdir()
        (checkout / ".git").mkdir()
        self.cli("pack", "--input-dir", self.inputs, "--include", "catalog.json",
                 "--output", checkout / "private-data.tar.gz", ok=False)
        self.assertFalse((checkout / "private-data.tar.gz").exists())


if __name__ == "__main__":
    unittest.main()
