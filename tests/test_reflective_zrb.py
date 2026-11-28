"""Synthetic reflective-property packets; no original resources or catalogs."""
import importlib.util
import math
from pathlib import Path
import struct
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]


def load_reader():
    name = 'mag_reflective_zrb'
    spec = importlib.util.spec_from_file_location(name, ROOT / 'tools/extraction/reflective_zrb.py')
    module = importlib.util.module_from_spec(spec)
    sys.modules[name] = module
    spec.loader.exec_module(module)
    return module


def text(value):
    data = value.encode('ascii') + b'\0'
    return struct.pack('>I', len(data)) + data


def field(name, kind, raw):
    return b'\1' + text(name) + text(kind) + struct.pack('>I', len(raw) + 4) + raw


def obj(name, *fields):
    return struct.pack('>II', 0xdeadbeef, 3) + text(name) + b''.join(fields) + b'\0'


def nested(value):
    return b'\1' + struct.pack('>III', 0xdeadbeef, 2, len(value) + 4) + value


class ReflectiveZrbTests(unittest.TestCase):
    def setUp(self):
        self.api = load_reader()

    def test_scalars_and_offsets_with_unknown_values_retained(self):
        data = obj('Synthetic', field('signed', 'int32', struct.pack('>i', -7)),
                   field('unsigned', 'uint32', struct.pack('>I', 9)),
                   field('ratio', 'float', struct.pack('>f', 1.5)),
                   field('enabled', 'bool', b'\1'),
                   field('token', 'util::Token', bytes.fromhex('12345678')),
                   field('name', 'ztl::string', text('Synthetic')),
                   field('unknown', 'FutureType', b'opaque'))
        result, end = self.api.Reader(data).object(0)
        self.assertEqual(end, len(data))
        self.assertEqual(result['class'], 'Synthetic')
        values = {key: value['value'] for key, value in result['fields'].items()}
        self.assertEqual(values, dict(signed=-7, unsigned=9, ratio=1.5, enabled=True,
                                     token='0x12345678', name='Synthetic', unknown=b'opaque'.hex()))
        self.assertTrue(result['fields']['unknown']['opaque'])
        value = result['fields']['signed']
        self.assertEqual(data[value['offset']:value['offset'] + value['bytes']], struct.pack('>i', -7))

    def test_nested_objects_and_vector_order(self):
        first = obj('Row', field('id', 'uint32', struct.pack('>I', 19)))
        second = obj('Row', field('id', 'uint32', struct.pack('>I', 3)))
        data = obj('Root', field('single', 'object', nested(first)),
                   field('rows', 'ztl::vector', struct.pack('>II', 1, 2) + first + second))
        result, end = self.api.Reader(data).object(0)
        self.assertEqual(end, len(data))
        self.assertEqual(result['fields']['single']['value']['fields']['id']['value'], 19)
        rows = result['fields']['rows']['value']
        self.assertEqual([row['fields']['id']['value'] for row in rows], [19, 3])

    def test_empty_and_opaque_vectors_are_not_guessed(self):
        raw = struct.pack('>II', 1, 3) + b'abc'
        data = obj('Root', field('empty', 'ztl::vector', struct.pack('>II', 1, 0)),
                   field('unknown', 'ztl::vector', raw))
        parsed, _ = self.api.Reader(data).object(0)
        self.assertEqual(parsed['fields']['empty']['value'], [])
        unknown = parsed['fields']['unknown']
        self.assertEqual(unknown['value'], raw.hex())
        self.assertTrue(unknown['opaque'])
        self.assertEqual(unknown['vector_count'], 3)

    def test_every_truncated_prefix_fails(self):
        data = obj('Root', field('name', 'ztl::string', text('sample')))
        for length in range(len(data)):
            with self.subTest(length=length), self.assertRaises(self.api.ZrbFormatError):
                self.api.Reader(data[:length]).object(0)

    def test_duplicate_fields_and_invalid_scalar_widths(self):
        valid = field('n', 'uint32', struct.pack('>I', 1))
        malformed = [obj('Root', valid, valid), obj('Root', field('n', 'int32', b'\0')),
                     obj('Root', field('b', 'bool', b'\2')),
                     obj('Root', field('f', 'float', struct.pack('>f', math.inf))),
                     obj('Root', field('f', 'float', struct.pack('>f', math.nan))),
                     obj('Root', field('v', 'ztl::vector', struct.pack('>II', 1, 0) + b'extra'))]
        for data in malformed:
            with self.subTest(data=data.hex()), self.assertRaises(self.api.ZrbFormatError):
                self.api.Reader(data).object(0)

    def test_declared_lengths_and_object_boundaries(self):
        data = obj('Root', field('n', 'uint32', struct.pack('>I', 1)))
        for start, end in [(-1, len(data)), (0, len(data) + 1), (0, 5)]:
            with self.subTest(start=start, end=end), self.assertRaises(self.api.ZrbFormatError):
                self.api.Reader(data).object(start, end)
        bad_size = b'\1' + text('n') + text('uint32') + struct.pack('>I', 0xffffffff)
        with self.assertRaises(self.api.ZrbFormatError):
            self.api.Reader(obj('Root', bad_size)).object(0)
        with self.assertRaises(self.api.ZrbFormatError):
            self.api.Reader(obj('Root', field('s', 'ztl::string', struct.pack('>I', 5) + b'ab\0'))).object(0)

    def test_resource_depth_node_field_and_vector_budgets(self):
        child = obj('Child')
        data = obj('Root', field('child', 'object', nested(child)))
        for limits in [self.api.Limits(max_depth=1), self.api.Limits(max_nodes=1), self.api.Limits(max_bytes=8)]:
            with self.subTest(limits=limits), self.assertRaises(self.api.ZrbFormatError):
                self.api.Reader(data, limits).object(0)
        with self.assertRaises(self.api.ZrbFormatError):
            self.api.Reader(obj('Root', field('x', 'uint32', b'\0' * 4), field('y', 'uint32', b'\0' * 4)),
                            self.api.Limits(max_fields=1)).object(0)
        with self.assertRaises(self.api.ZrbFormatError):
            self.api.Reader(obj('Root', field('v', 'ztl::vector', struct.pack('>II', 1, 2))),
                            self.api.Limits(max_vector=1)).object(0)


if __name__ == '__main__':
    unittest.main()
