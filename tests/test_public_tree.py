"""Exercise publication checks with disposable synthetic trees."""
import importlib.util
import hashlib
import json
from pathlib import Path
import tempfile
import unittest

spec = importlib.util.spec_from_file_location(
    "check_public_tree", Path(__file__).resolve().parents[1] / "tools/check_public_tree.py"
)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)


class PublicationBoundaryTests(unittest.TestCase):
    def check(self, name, content):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(content)
            return guard.check_tree(root)

    def test_source_and_synthetic_data_are_accepted(self):
        self.assertEqual([], self.check("src/example.rs", b"pub const VERSION: u8 = 1;\n"))
        self.assertEqual([], self.check("gameplay/data/example.json", b'{"source":"synthetic example","values":[]}'))

    def test_binary_capture_is_rejected(self):
        self.assertTrue(self.check("tests/fixtures/packet.bin", bytes([1, 2, 3])))

    def test_machine_path_is_rejected(self):
        self.assertTrue(self.check("src/example.rs", ("// /" + "Users/example/private/input").encode()))

    def test_windows_and_home_paths_are_rejected(self):
        for path in ["C:" + "\\Users\\example\\source", "~" + "/projects/example", "$" + "HOME/private/file", "/" + "Users/example"]:
            with self.subTest(path=path):
                self.assertTrue(self.check("src/example.cs", path.encode()))

    def test_concrete_private_lan_default_is_rejected(self):
        self.assertTrue(self.check("src/example.cs", ('"' + '192.168.' + '20.42"').encode()))
        self.assertEqual([], self.check("src/example.cs", b'"127.0.0.1"'))

    def test_known_map_dataset_helper_is_rejected(self):
        self.assertTrue(self.check("gameplay/src/replication.rs", ('pub fn alyeska' + '_initial() {}').encode()))

    def test_private_research_reference_is_rejected(self):
        self.assertTrue(self.check("README.md", ("See client" + "-evidence/result.json").encode()))

    def test_embedded_private_key_is_rejected(self):
        self.assertTrue(self.check("src/example.cs", ("-----BEGIN " + "RSA PRIVATE KEY-----").encode()))

    def test_opaque_encoded_blob_is_rejected(self):
        self.assertTrue(self.check("src/example.rs", ('const IMAGE: &str = "' + 'a1' * 150 + '";').encode()))

    def test_fixed_decimal_private_key_is_rejected(self):
        self.assertTrue(self.check("src/keys.cs", ('new BigInteger("' + '1234567890' * 15 + '");').encode()))

    def test_unmarked_runtime_catalog_is_rejected(self):
        self.assertTrue(self.check("gameplay/data/catalog.json", b'{"items":[]}'))

    def test_strict_schema_provenance_is_bound_to_exact_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "gameplay/data/catalog.json"
            path.parent.mkdir(parents=True)
            path.write_text('{"items":[]}')
            manifest = root / "tools/synthetic-data.json"
            manifest.parent.mkdir()
            manifest.write_text(json.dumps({"gameplay/data/catalog.json": {
                "source": "synthetic schema example",
                "sha256": hashlib.sha256(path.read_bytes()).hexdigest(),
            }}))
            self.assertEqual([], guard.check_tree(root))
            path.write_text('{"items":[123]}')
            self.assertTrue(guard.check_tree(root))

    def test_malformed_provenance_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            path = root / "gameplay/data/catalog.json"
            path.parent.mkdir(parents=True)
            path.write_text('{"items":[]}')
            manifest = root / "tools/synthetic-data.json"
            manifest.parent.mkdir()
            manifest.write_text("[]")
            self.assertTrue(guard.check_tree(root))

    def test_rust_bin_sources_are_checked(self):
        self.assertTrue(self.check("gameplay/src/bin/example.rs", ('"' + 'b2' * 150 + '"').encode()))

    def test_symlink_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "target.txt").write_text("source")
            (root / "link.txt").symlink_to("target.txt")
            self.assertTrue(guard.check_tree(root))


if __name__ == "__main__":
    unittest.main()
