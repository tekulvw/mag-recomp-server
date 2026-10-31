"""Synthetic ELF segment boundaries for external native-witness validation."""
import importlib.util
from pathlib import Path
import struct
import sys
import unittest


TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'collision'
sys.path.insert(0, str(TOOLS))
SPEC = importlib.util.spec_from_file_location('retail_parent_witness', TOOLS / 'retail_parents.py')
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


def synthetic_elf(segments):
    data = bytearray(512)
    data[:6] = b'\x7fELF\x02\x02'
    struct.pack_into('>Q', data, 32, 64)
    struct.pack_into('>HH', data, 54, 56, len(segments))
    for index, (offset, address, size) in enumerate(segments):
        struct.pack_into('>II6Q', data, 64 + 56 * index,
                         1, 5, offset, address, address, size, size, 4)
    data[400:408] = b'syntheti'
    return bytes(data)


class ParentWitnessTests(unittest.TestCase):
    def test_virtual_address_uses_file_offset_and_allows_exact_end(self):
        data = synthetic_elf([(400, 0x2400, 8)])
        self.assertEqual(MODULE.elf_span(data, 0x2402, 6), b'ntheti')

    def test_function_cannot_cross_file_backed_end(self):
        data = synthetic_elf([(400, 0x2400, 8)])
        with self.assertRaisesRegex(ValueError, 'one file-backed segment'):
            MODULE.elf_span(data, 0x2406, 4)

    def test_overlapping_virtual_mappings_are_ambiguous(self):
        data = synthetic_elf([(400, 0x2400, 8), (440, 0x2400, 8)])
        with self.assertRaisesRegex(ValueError, 'one file-backed segment'):
            MODULE.elf_span(data, 0x2400, 4)

    def test_truncated_segment_and_program_table_are_rejected(self):
        data = synthetic_elf([(508, 0x2400, 8)])
        with self.assertRaisesRegex(ValueError, 'segment outside input'):
            MODULE.elf_span(data, 0x2400, 4)
        data = bytearray(synthetic_elf([(400, 0x2400, 8)]))
        struct.pack_into('>Q', data, 32, len(data) - 10)
        with self.assertRaisesRegex(ValueError, 'segment table'):
            MODULE.elf_span(bytes(data), 0x2400, 4)

    def test_invalid_span_and_wrong_elf_format_are_rejected(self):
        data = synthetic_elf([(400, 0x2400, 8)])
        for address, size in [(0x2400, 0), (-1, 4), (0x2400, 1024 * 1024 + 1)]:
            with self.subTest(address=address, size=size), self.assertRaises(ValueError):
                MODULE.elf_span(data, address, size)
        with self.assertRaisesRegex(ValueError, 'big-endian ELF64'):
            MODULE.elf_span(b'not-an-elf', 0, 4)


if __name__ == '__main__':
    unittest.main()
