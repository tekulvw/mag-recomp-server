import copy
import json
from pathlib import Path
import runpy
import unittest


TOOLS = Path(__file__).resolve().parents[1] / 'tools/navmesh'


class RegionalWalkCoverageTests(unittest.TestCase):
    def test_scoped_prepare_preserves_source_and_requires_capability(self):
        ns = runpy.run_path(str(TOOLS / 'prepare.py'))
        world = {
            'format': 'mag_collision_world', 'schema_version': 1, 'coordinate_space': 'world',
            'metadata': {'map_name': 'invented', 'physics_regions': []},
            'instances': [{'id': 'terrain:invented', 'vertices_xyz': [[0, 0, 0], [0, 0, 8], [8, 0, 0]],
                           'triangles': [[0, 1, 2]]}],
        }
        raw_before = json.dumps(world)
        sha = 'a' * 64
        reference, _ = ns['prepare'](world, sha, reference=True)
        coverage = {
            'schema': 'mag_bot_regional_walk_coverage_v1', 'policy': 'represented_fixed_triangles_walk',
            'map_name': 'invented', 'world_sha256': sha, 'geometry_sha256': reference['geometry_sha256'],
            'bounds': {'min': [1, -1, 1], 'max': [5, 4, 5]},
            'source': {k: 'b' * 64 for k in ('level_archive_sha256', 'level_clv_sha256', 'assembly_sha256',
                                             'root_inventory_sha256', 'absence_sha256')},
        }
        with self.assertRaisesRegex(ValueError, 'no verified static coverage'):
            ns['prepare'](world, sha)
        scoped, _ = ns['prepare_regional'](world, sha, coverage, 'c' * 64)
        self.assertEqual(scoped['schema'], 'mag_navmesh_regional_walk_input_v1')
        self.assertEqual(scoped['coverage_sha256'], 'c' * 64)
        self.assertEqual(scoped['mesh']['triangles'], reference['triangles'])
        self.assertEqual(scoped['mesh']['bounds'], coverage['bounds'])
        self.assertEqual(scoped['mesh']['physics_regions'], [])
        self.assertEqual(json.dumps(world), raw_before)
        with self.assertRaisesRegex(ValueError, 'world'):
            ns['prepare_regional'](world, 'd' * 64, coverage, 'c' * 64)

    def test_projector_requires_complete_fixed_roots_and_bound_omissions(self):
        ns = runpy.run_path(str(TOOLS / 'project-regional-walk-coverage.py'))
        h = lambda c: c * 64
        world = {
            'format': 'mag_collision_world', 'schema_version': 1, 'coordinate_space': 'world',
            'metadata': {'map_name': 'invented', 'build': 'synthetic', 'physics_regions': []},
            'instances': [{'id': 'terrain:invented:pack-0:mesh-0',
                           'vertices_xyz': [[0, 0, 0], [0, 0, 8], [8, 0, 0]], 'triangles': [[0, 1, 2]]}],
        }
        excluded = [{'collision_offset': 100, 'token': '01234567'}]
        scans = [{'sha256': h('a'), 'token_matches': 0}, {'sha256': h('b'), 'token_matches': 0}]
        spans = [{'start': '01000000', 'bytes': 16, 'sha256': h('f')}]
        absent = {'schema': 'mag_retail_absent_collision_resource_v1',
                  'excluded_exact_references': excluded, 'archives': scans, 'native_ranges': spans,
                  'source_elf_sha256': h('f')}
        assembly = {'schema': 'mag_retail_compiled_reuse_v1', 'scope': 'initial_compiled_static_geometry',
                    'map': 'invented', 'world_sha256': h('c'), 'missing_resources': [],
                    'counts': {'source_bound_absent_resource_references': 1, 'compiled_present': 1},
                    'instances': 1, 'triangles': 1, 'compiled_joined': 0, 'bindings': []}
        row = {'map': 'invented', 'original_clv_sha256': h('e'),
               'native_qualification': {'original_retail_sha256': h('f')},
               'source_inputs': [{'sha256': h(c)} for c in ('a', 'c', 'd')],
               'source_membership': {'source_meshes_omitted': [], 'mesh_instances': 1, 'source_roots': 1,
                                     'native_present_compiled_records': 1,
                                     'compiled_objects_with_resolved_resources': 0},
               'native_missing_resource_no_roots': {'excluded_exact_references': excluded,
                    'archives_reproduced': scans, 'native_ranges_verified': spans},
               'root_inventory': [{'root_id': 'terrain:invented:pack-0', 'kind': 'terrain',
                                   'parent_token': '00000000', 'initial_motion_type': 7}]}
        roots = {'schema': 'mag_private_acquisition_source_root_exclusions_v2', 'maps': [row]}
        args = (world, h('1'), world, h('c'), assembly, h('d'), roots, h('4'), absent, h('5'), h('a'),
                {'min': [1, -1, 1], 'max': [5, 4, 5]})
        source = dict(level_clv_sha=h('e'), main_archive_sha=h('b'), retail_elf_sha=h('f'))
        with self.assertRaisesRegex(ValueError, 'lineage'):
            ns['project'](*args, **dict(source, level_clv_sha=h('c')))
        projected = ns['project'](*args, **source)
        different = copy.deepcopy(world)
        different['instances'][0]['vertices_xyz'][0][1] = 1
        with self.assertRaisesRegex(ValueError, 'geometry'):
            ns['project'](different, *args[1:], **source)
        with self.assertRaisesRegex(ValueError, 'absence'):
            ns['project'](*args, **dict(source, main_archive_sha=h('c')))
        self.assertEqual(projected['source']['absence_sha256'], h('5'))
        self.assertEqual(projected['bounds'], args[-1])
        row['root_inventory'][0]['parent_token'] = '00000001'
        with self.assertRaisesRegex(ValueError, 'fixed'):
            ns['project'](*args, **source)
        row['root_inventory'][0]['parent_token'] = '00000000'
        absent['excluded_exact_references'] = []
        with self.assertRaisesRegex(ValueError, 'absence'):
            ns['project'](*args, **source)
