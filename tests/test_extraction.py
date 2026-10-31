"""Extractor checks use invented tiny inputs only; no game files are needed."""
import hashlib
import json
from pathlib import Path
import runpy
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
EXTRACTION = ROOT / 'tools/extraction'
COLLISION = ROOT / 'tools/collision'
sys.path.insert(0, str(COLLISION))


def api(name, directory=EXTRACTION):
    return runpy.run_path(str(directory / name))


class ExtractionTests(unittest.TestCase):
    def test_literal_parser_retains_duplicates_and_rejects_code(self):
        tool = api('extract-item-catalog.py')
        parse = lambda text: tool['normalize'](tool['Parser'](text, 'invented.zpr').parse())
        value = parse('Example = { amount = "2.50", enabled = true, name = "alpha" }')
        self.assertEqual(value['Example']['amount'], 2.5)
        self.assertTrue(value['Example']['enabled'])
        duplicate = parse('Example = { key = 1, key = 2 }')
        self.assertEqual([row['value'] for row in duplicate['Example']['$lua_table']], [1, 2])
        with self.assertRaises(ValueError):
            parse('Example = os.execute("invented")')
        with self.assertRaises(ValueError):
            parse('Example = function() return 1 end')

    def test_literal_source_spelling_is_retained(self):
        tool = api('extract-item-catalog.py')
        literals = {}
        result = tool['normalize'](tool['Parser']('Example = { value = "1.250" }', 'invented.zpr').parse(), literals=literals)
        self.assertEqual(result['Example']['value'], 1.25)
        self.assertEqual(literals['/Example/value'], '"1.250"')

    def test_psarc_reader_on_invented_resource_and_truncation(self):
        from archive import Psarc
        from packfile import CollisionFormatError
        names, content = b'/invented/resource.zpr\n', b'Example = { value = "7" }'
        payloads = [names, content]
        toc = 32 + 30 * 2 + 2
        data = bytearray(struct.pack('>4sI4sIIIII', b'PSAR', 0x10003, b'zlib', toc, 30, 2, 256, 0))
        offset = toc
        for i, raw in enumerate(payloads):
            data.extend(bytes(16) + struct.pack('>I', i) + len(raw).to_bytes(5, 'big') + offset.to_bytes(5, 'big'))
            offset += len(raw)
        for raw in payloads:
            data.extend(len(raw).to_bytes(1, 'big'))
        data.extend(names + content)
        arc = Psarc(bytes(data))
        self.assertEqual(arc.names, ['/invented/resource.zpr'])
        self.assertEqual(arc.extract(1), content)
        with self.assertRaises(CollisionFormatError):
            Psarc(bytes(data[:-1]))

    def test_collision_catalog_and_join_use_same_explicit_archive(self):
        from catalog import Catalog
        import instance_linkage
        resources = {
            '/invented/level.clv': b'invented empty level',
            '/invented/physics.zrb': struct.pack('>12I', 6, 50, 0, 50, 0, 50, 0, 50, 0, 50, 0, 0) + bytes(2),
        }
        names = ('\n'.join(resources) + '\n').encode()
        payloads = [names, *resources.values()]
        count = len(payloads)
        toc = 32 + 30 * count + count
        raw = bytearray(struct.pack('>4sI4sIIIII', b'PSAR', 0x10003, b'zlib', toc, 30, count, 256, 0))
        offset = toc
        for i, value in enumerate(payloads):
            raw.extend(bytes(16) + struct.pack('>I', i) + len(value).to_bytes(5, 'big') + offset.to_bytes(5, 'big'))
            offset += len(value)
        raw.extend(bytes(len(value) for value in payloads))
        raw.extend(b''.join(payloads))
        digest = hashlib.sha256(raw).hexdigest()
        with tempfile.TemporaryDirectory() as directory:
            out = Path(directory)
            catalog = Catalog(out, retain_raw=False, include_instance_evidence=False)
            summary = catalog.map(bytes(raw), 'synthetic-build', 'invented-map', {'archive':'invented.psarc'}, digest)
            manifest = json.loads((out / summary['manifest']['file']).read_text())
            self.assertEqual(manifest['source']['archive_sha256'], digest)
            self.assertEqual(len(manifest['resource_members']), 1)
            self.assertNotIn('raw_offline_source_member', manifest['resource_members'][0])
            join = instance_linkage.inspect(bytes(raw), {}, {}, 'synthetic-build', 'invented-map', None)
            self.assertEqual(join['source']['archive_sha256'], digest)
            self.assertEqual(join['counts']['instances'], 0)
            self.assertEqual(join['instances'], [])

    def test_empty_invented_native_resource_header_and_bad_bounds(self):
        from zrb_records import parse
        raw = struct.pack('>12I', 6, 50, 0, 50, 0, 50, 0, 50, 0, 50, 0, 0) + bytes(2)
        self.assertEqual(parse(raw)['counts']['resources'], 0)
        with self.assertRaises(ValueError):
            parse(raw[:45])

    def test_compiled_transform_applies_scale_once(self):
        tool = api('world.py', COLLISION)
        matrix = [2.,0.,0.,10., 0.,2.,0.,20., 0.,0.,2.,30., 0.,0.,0.,2.]
        raw = struct.pack('>IBI17f', 0x10001, 1, 123, *matrix, 2.)
        placement = tool['decode_collision_record'](raw, 0)
        self.assertEqual(tool['bake_vertex']([1.,2.,3.], placement), [12.,24.,36.])
        with self.assertRaises(ValueError):
            tool['decode_collision_record'](raw[:20], 0)

    def test_navmesh_preparation_requires_verified_world_and_preserves_faces(self):
        tool = api('prepare.py', ROOT / 'tools/navmesh')
        region = dict(min=[-1,-1,-1], max=[3,3,3], ground_verified=True, obstacles_verified=True, provenance=['synthetic test'])
        world = dict(format='mag_collision_world', schema_version=1, coordinate_space='world', metadata=dict(map_name='invented-map', physics_regions=[region]), instances=[dict(id='invented', vertices_xyz=[[0,0,0],[0,0,2],[2,0,0]], triangles=[[0,1,2]])])
        result, report = tool['prepare'](world, 'a' * 64)
        self.assertEqual(result['triangles'], [[0,1,2]])
        self.assertEqual(report['triangles'], 1)
        self.assertEqual(len(result['geometry_sha256']), 64)
        world['metadata']['physics_regions'] = []
        with self.assertRaises(ValueError):
            tool['prepare'](world, 'a' * 64)

    def test_loadout_projector_accepts_only_explicit_external_input(self):
        tool = api('extract-loadout-defaults.py')
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory) / 'invented.json'
            path.write_text(json.dumps(dict(faction=0, **{'class':21}, source_sha256={}, loadouts=[dict(index=0, update_id=4, slots=[dict(slot=i,item_id=None) for i in range(22)])])))
            result = json.loads(tool['render'](path))
            self.assertEqual(result['factions'][0]['loadouts'][0]['items'], [-1] * 22)
            self.assertEqual(result['retained_defaults_sha256'], hashlib.sha256(path.read_bytes()).hexdigest())

    def test_cli_help_imports_every_executable_extractor(self):
        for directory in [EXTRACTION, COLLISION, ROOT / 'tools/navmesh']:
            for path in sorted(directory.glob('*.py')):
                if '__main__' not in path.read_text():
                    continue
                with self.subTest(tool=path.name):
                    result = subprocess.run([sys.executable, str(path), '--help'], capture_output=True, text=True)
                    self.assertEqual(result.returncode, 0, result.stderr)
                    self.assertIn('usage:', result.stdout)


if __name__ == '__main__':
    unittest.main()
