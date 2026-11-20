import runpy
import struct
import unittest
from pathlib import Path

TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
PROJECTOR = runpy.run_path(str(TOOLS / 'project-retail-active-ability-map.py'))


class AbilityMapProjectionTests(unittest.TestCase):
    def test_bounds_require_matching_terrain_and_empty_polygon(self):
        clv = bytearray(300)
        struct.pack_into('>IIIfI', clv, 0, 31, 0, 1, 17.0, 1)
        struct.pack_into('>H', clv, 40, 128)
        box = (12., 38., 24., 10., 4., 20.)
        struct.pack_into('>6f', clv, 170, *box)
        self.assertEqual(
            PROJECTOR['bounds'](clv, box),
            ({'min_x': 2., 'min_z': 4., 'max_x': 22., 'max_z': 44.}, [34., 42.]),
        )
        with self.assertRaisesRegex(ValueError, 'terrain.*header'):
            PROJECTOR['bounds'](clv, (11., *box[1:]))
        struct.pack_into('>I', clv, 194, 1)
        with self.assertRaisesRegex(ValueError, 'boundary polygon'):
            PROJECTOR['bounds'](clv, box)

    def test_catalog_join_checks_token_index_and_coverage(self):
        catalog = {
            'objects': [{'token': '0x4', 'object_index': 0}, {'token': '0x9', 'object_index': 1}],
        }
        self.assertEqual(PROJECTOR['catalog_indices'](catalog), {4: 0, 9: 1})
        catalog['objects'][1]['object_index'] = 0
        with self.assertRaisesRegex(ValueError, 'duplicate index'):
            PROJECTOR['catalog_indices'](catalog)

    def test_vector_rejects_truncated_and_duplicate_tokens(self):
        links = PROJECTOR['roster_tokens']
        self.assertEqual(links(struct.pack('>4I', 1, 2, 7, 9).hex()), (7, 9))
        with self.assertRaisesRegex(ValueError, 'length'):
            links(struct.pack('>3I', 1, 2, 7).hex())
        with self.assertRaisesRegex(ValueError, 'Duplicate'):
            links(struct.pack('>4I', 1, 2, 7, 7).hex())


if __name__ == '__main__':
    unittest.main()
