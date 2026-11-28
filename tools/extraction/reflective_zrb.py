"""Bounded reader for named, typed reflective property objects.

Inputs are external resources. Importing this module performs no I/O. Unknown
field/element layouts are retained as opaque bytes rather than inferred values.
"""
from dataclasses import dataclass
import hashlib
import math
import struct


class ZrbFormatError(ValueError):
    pass


@dataclass(frozen=True)
class Limits:
    max_bytes: int = 32 * 1024 * 1024
    max_string: int = 4096
    max_depth: int = 40
    max_nodes: int = 30000
    max_fields: int = 100000
    max_vector: int = 10000


class Reader:
    OBJECT = bytes.fromhex('deadbeef00000003')
    NESTED = bytes.fromhex('01deadbeef00000002')

    def __init__(self, data, limits=None):
        self.limits = limits or Limits()
        if any(value < 1 for value in vars(self.limits).values()):
            raise ZrbFormatError('Reader budgets must be positive.')
        if len(data) > self.limits.max_bytes:
            raise ZrbFormatError('Resource exceeds reader byte budget.')
        self.data = memoryview(bytes(data))
        self.nodes = 0
        self.fields = 0

    def _range(self, start, count, end):
        if not (0 <= start <= end <= len(self.data)) or count < 0 or count > end - start:
            raise ZrbFormatError(f'Truncated/out-of-range property at offset {start}.')

    def word(self, offset, end):
        self._range(offset, 4, end)
        return int.from_bytes(self.data[offset:offset + 4], 'big')

    def string(self, offset, end):
        count = self.word(offset, end)
        if count < 1 or count > self.limits.max_string:
            raise ZrbFormatError('Invalid bounded property-name length.')
        begin = offset + 4
        self._range(begin, count, end)
        raw = self.data[begin:begin + count].tobytes()
        if raw[-1] != 0 or b'\0' in raw[:-1]:
            raise ZrbFormatError('Property name is not exactly NUL terminated.')
        try:
            value = raw[:-1].decode('ascii')
        except UnicodeDecodeError as error:
            raise ZrbFormatError('Unsupported property-name encoding.') from error
        return value, begin + count

    def object(self, offset=0, end=None, depth=0):
        end = len(self.data) if end is None else end
        self._range(offset, 8, end)
        if depth >= self.limits.max_depth:
            raise ZrbFormatError('Property nesting exceeds reader budget.')
        if self.data[offset:offset + 8] != self.OBJECT:
            raise ZrbFormatError('Unsupported reflective object tag.')
        self.nodes += 1
        if self.nodes > self.limits.max_nodes:
            raise ZrbFormatError('Object count exceeds reader budget.')
        name, cursor = self.string(offset + 8, end)
        fields = {}
        while cursor < end and self.data[cursor] == 1:
            self.fields += 1
            if self.fields > self.limits.max_fields:
                raise ZrbFormatError('Field count exceeds reader budget.')
            key, cursor = self.string(cursor + 1, end)
            kind, cursor = self.string(cursor, end)
            if not key or not kind or key in fields:
                raise ZrbFormatError('Empty or duplicate reflective field.')
            size = self.word(cursor, end)
            if size < 4:
                raise ZrbFormatError('Invalid reflective field byte length.')
            self._range(cursor, size, end)
            begin, stop = cursor + 4, cursor + size
            value, details = self._value(kind, begin, stop, depth)
            fields[key] = {'type': kind, 'value': value, 'offset': begin,
                           'bytes': stop - begin, **details}
            cursor = stop
        self._range(cursor, 1, end)
        if self.data[cursor] != 0:
            raise ZrbFormatError('Missing reflective object terminator.')
        stop = cursor + 1
        return {'class': name, 'fields': fields, 'offset': offset, 'end': stop,
                'sha256': hashlib.sha256(self.data[offset:stop]).hexdigest()}, stop

    def _value(self, kind, begin, end, depth):
        raw = self.data[begin:end]
        if kind in ('int32', 'uint32', 'float', 'util::Token', 'util::LocalizedString'):
            if len(raw) != 4:
                raise ZrbFormatError('Invalid scalar property width.')
            if kind == 'float':
                value = struct.unpack('>f', raw)[0]
                if not math.isfinite(value):
                    raise ZrbFormatError('Nonfinite reflective scalar.')
            elif kind in ('util::Token', 'util::LocalizedString'):
                value = '0x' + raw.hex()
            else:
                value = int.from_bytes(raw, 'big', signed=kind == 'int32')
            return value, {}
        if kind == 'bool':
            if len(raw) != 1 or raw[0] > 1:
                raise ZrbFormatError('Invalid reflective boolean.')
            return bool(raw[0]), {}
        if kind == 'ztl::string':
            count = self.word(begin, end)
            if count > self.limits.max_string or count != len(raw) - 4:
                raise ZrbFormatError('Invalid reflective string length.')
            if not count:
                return '', {}
            text = self.data[begin + 4:end].tobytes()
            if text[-1] != 0 or b'\0' in text[:-1]:
                raise ZrbFormatError('Reflective string is not exactly NUL terminated.')
            try:
                return text[:-1].decode('utf-8'), {}
            except UnicodeDecodeError as error:
                raise ZrbFormatError('Unsupported reflective string encoding.') from error
        if kind == 'object' and raw[:9] == self.NESTED:
            if self.word(begin + 9, end) != len(raw) - 9:
                raise ZrbFormatError('Nested reflective object length mismatch.')
            value, stop = self.object(begin + 13, end, depth + 1)
            if stop != end:
                raise ZrbFormatError('Trailing bytes in nested reflective object.')
            return value, {}
        if kind == 'ztl::vector':
            if len(raw) < 8:
                raise ZrbFormatError('Truncated reflective vector header.')
            if self.word(begin, end) == 1:
                count = self.word(begin + 4, end)
                if count > self.limits.max_vector:
                    raise ZrbFormatError('Vector count exceeds reader budget.')
                cursor = begin + 8
                if count > end - cursor:
                    raise ZrbFormatError('Vector count exceeds remaining payload bytes.')
                if not count:
                    if cursor != end:
                        raise ZrbFormatError('Trailing bytes in empty reflective vector.')
                    return [], {}
                if self.data[cursor:cursor + 8] == self.OBJECT:
                    values = []
                    for _ in range(count):
                        value, cursor = self.object(cursor, end, depth + 1)
                        values.append(value)
                    if cursor != end:
                        raise ZrbFormatError('Reflective vector count/length mismatch.')
                    return values, {}
                return raw.hex(), {'opaque': True, 'vector_count': count}
        return raw.hex(), {'opaque': True}
