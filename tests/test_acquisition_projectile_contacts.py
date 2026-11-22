"""Synthetic source pin and layer-row checks for Acquisition contact projection."""
import hashlib
import json
from pathlib import Path
import runpy
import tempfile
import unittest

PROJECT = runpy.run_path(str(Path(__file__).resolve().parents[1] /
                            "tools/extraction/project-retail-acquisition-projectile-contacts.py"))
PROJECT = PROJECT["project"]


class AcquisitionContactsTests(unittest.TestCase):
    def check_projection(self, change=None):
        with tempfile.TemporaryDirectory() as temp:
            root = Path(temp)

            def put(name, value):
                path = root / name
                path.write_text(json.dumps(value), encoding="utf8")
                return path

            def sha(path):
                return hashlib.sha256(path.read_bytes()).hexdigest()

            tag = 9
            source = "a" * 64
            neutral = "b" * 64
            elf = "e" * 64
            raw_world = put("world", {"synthetic": "represented geometry"})
            dynamic = put("dynamic", {"synthetic": "body catalog"})
            index = put("index", {
                "map": "synthetic",
                "level_archive_sha256": source,
                "physics_sha256": neutral,
                "resources": {"synthetic-token": [{
                    "source_sha256": source, "neutral_sha256": neutral, "collision_tag": tag,
                }]},
            })
            filters = put("filters", {
                "retail_sha256": elf, "initial_filter_enable_pairs": [[9, 23], [23, 22]],
            })
            static = put("static", {
                "map_name": "synthetic", "source_archive_sha256": source,
                "source_elf_sha256": elf, "world_sha256": sha(raw_world),
            })
            proof = put("proof", {
                "row23": f"{(1 | 1 << 9 | 1 << 22):#010x}",
                "dynamic_catalog_sha256": sha(dynamic),
                "native_filter_rows_receipt_sha256": sha(filters),
                "maps": {"synthetic": {
                    "static_world_sha256": sha(raw_world), "static_receipt_sha256": sha(static),
                    "sequence_index_sha256": sha(index), "physics_sha256": neutral,
                    "sequence": [{"source_sha256": source, "neutral_sha256": neutral,
                                  "root": tag, "part_ids": ["synthetic-part"]}],
                }},
            })
            if change:
                (root / change).write_text('{"mutated":true}', encoding="utf8")
            return PROJECT("synthetic", proof, filters, raw_world, static, dynamic, index)

    def test_projection_has_explicit_mode_and_original_filter_row(self):
        result = self.check_projection()
        self.assertEqual(result["mode"], 1)
        self.assertEqual(result["filter_row23"], 1 | 1 << 9 | 1 << 22)
        self.assertEqual(result["sequence"][0]["contact"]["child_filter_info"], 0)
        self.assertNotIn("hospital", result)
        self.assertNotIn("helo", result)

    def test_raw_pin_mismatches_refuse_projection(self):
        for field in ["world", "dynamic"]:
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "source pins"):
                self.check_projection(field)


if __name__ == "__main__":
    unittest.main()
