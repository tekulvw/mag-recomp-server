import hashlib
from pathlib import Path
import runpy
from tempfile import TemporaryDirectory
import unittest

from test_native_projectors import archive_bytes


SCRIPT = Path(__file__).resolve().parents[1] / 'tools/navmesh/project-retail-movement-profile.py'
MEMBER = '/run/mag/ps3/scripts/character.zpr'


def sha(value):
    return hashlib.sha256(value).hexdigest()


class RetailMovementProfileTests(unittest.TestCase):
    def setUp(self):
        self.tool = runpy.run_path(str(SCRIPT))
        self.folder = TemporaryDirectory()
        self.addCleanup(self.folder.cleanup)
        self.elf = Path(self.folder.name) / 'invented.elf'
        self.archive = self.elf.with_suffix('.psarc')
        self.elf.write_bytes(b'invented ELF source')
        self.sources = self.tool['SOURCES']

    def stage(self, settings, *, member=MEMBER):
        member_data = f'CharacterSettings = {{ {settings} }}'.encode()
        self.archive.write_bytes(archive_bytes({member: member_data}))
        self.sources.update(
            retail_elf_sha256=sha(self.elf.read_bytes()),
            game_archive_sha256=sha(self.archive.read_bytes()),
            character_zpr_sha256=sha(member_data),
        )

    def produce(self):
        return self.tool['produce'](self.elf, self.archive)

    def test_exact_sources_and_equal_authored_scalars(self):
        self.stage('fMaxAscentAngle = "42.5", fMaxNoSlideAngle = 42.5')
        self.assertEqual(self.produce(), {
            'schema_version': 1,
            'application_id': 20231,
            'source': self.sources,
            'max_ascent_degrees': 42.5,
            'max_no_slide_degrees': 42.5,
        })

    def test_rejects_wrong_source_digest_and_missing_member(self):
        settings = 'fMaxAscentAngle = 42, fMaxNoSlideAngle = 42'
        self.stage(settings, member='/invented.zpr')
        with self.assertRaisesRegex(ValueError, 'missing.*character.zpr'):
            self.produce()
        self.stage(settings)
        for key in self.sources:
            with self.subTest(key=key):
                prior = self.sources[key]
                self.sources[key] = '0' * 64
                try:
                    with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                        self.produce()
                finally:
                    self.sources[key] = prior

    def test_rejects_unusable_or_ambiguous_authored_values(self):
        for settings in (
            'fMaxAscentAngle = 42',
            'fMaxAscentAngle = 42, fMaxNoSlideAngle = 43',
            'fMaxAscentAngle = 0, fMaxNoSlideAngle = 0',
            'fMaxAscentAngle = 90, fMaxNoSlideAngle = 90',
            'fMaxAscentAngle = "NaN", fMaxNoSlideAngle = "NaN"',
            'fMaxAscentAngle = 1e500, fMaxNoSlideAngle = 1e500',
            'fMaxAscentAngle = 42, fMaxAscentAngle = 42, fMaxNoSlideAngle = 42',
        ):
            with self.subTest(settings=settings):
                self.stage(settings)
                with self.assertRaises(ValueError):
                    self.produce()
