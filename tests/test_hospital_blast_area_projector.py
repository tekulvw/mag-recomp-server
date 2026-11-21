"""Synthetic regressions for the hospital area projector."""
from pathlib import Path
import runpy
import unittest


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-hospital-blast-areas.py'


class HospitalBlastAreaTests(unittest.TestCase):
    def test_faction_binding_rejects_wrong_operational_pack_layer_and_shape(self):
        tool = runpy.run_path(str(TOOL))
        join = tool['operational_source']
        mesh = {'vertices_xyz': [[0, 0, 0]], 'triangles': [[0, 0, 0]],
                'subpart_index': 0, 'extrusion_xyz': [0, 0, 0], 'collision_margin': 0}
        neutral = {'member': '/body.zrb', 'member_sha256': 'a' * 64,
                   'physics_token': '0x12345678', 'packfile_sha256': 'b' * 64,
                   'geometry': {'meshes': [mesh]}}
        body = {'layer': 22, 'motion_type': 6, 'collision_tag': 9, 'material_index': 8,
                'packfile_sha256': 'b' * 64, 'source_member_sha256': 'a' * 64,
                'body_local': {'vertices_xyz': mesh['vertices_xyz'], 'triangles': mesh['triangles']}}
        self.assertEqual(join(body, neutral)['resource_token'], '12345678')
        for key, value in [('layer', 17), ('packfile_sha256', 'c' * 64), ('body_local', {})]:
            with self.subTest(key=key), self.assertRaises(ValueError):
                join({**body, key: value}, neutral)
