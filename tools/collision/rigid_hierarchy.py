"""Strict source readers for reference-pose rigid render hierarchies.

Only resource version 63, Renderable version 29 and compiled rigid instance version 7 are accepted. The
reader follows typed scalar/block reads; it never searches windows for tokens,
quaternions or node rows. Attachment definitions are unsupported. Trailing
resource metadata is returned as an explicit unparsed span after the nodes.

This parses original source structure only. It neither establishes native
animation/collision parity nor emits collision geometry or verified regions.
"""
import hashlib
import math
import struct

try:
    from .parent_transforms import CompactTransform, Node, ParentTransformError
except ImportError:
    from parent_transforms import CompactTransform, Node, ParentTransformError

RESOURCE_CLASS = 0xa4f4d23c
INSTANCE_CLASS = 0xe3c2fba0
END = 0xffff
MAX_ENTRIES = END
MAX_HEADER_BYTES = 16 * 1024 * 1024


class RigidHierarchyError(ValueError):
    """Unsupported or inconsistent original source structure."""


def _require(ok, message):
    if not ok:
        raise RigidHierarchyError(message)


def _sha(raw):
    return hashlib.sha256(raw).hexdigest()


def _integer(value, name):
    _require(type(value) is int and value >= 0, f"invalid {name}")
    return value


class _Reader:
    def __init__(self, raw, start, end):
        _require(isinstance(raw, (bytes, bytearray)), "expected byte input")
        _integer(start, "source offset")
        _integer(end, "source end")
        _require(start <= end <= len(raw), "source range out of bounds")
        self.raw, self.pos, self.end = raw, start, end

    def take(self, size):
        _integer(size, "read size")
        _require(size <= self.end - self.pos, "truncated source structure")
        value = bytes(self.raw[self.pos:self.pos + size])
        self.pos += size
        return value

    def unpack(self, fmt):
        return struct.unpack(fmt, self.take(struct.calcsize(fmt)))

    def u32(self):
        return self.unpack('>I')[0]

    def u16(self):
        return self.unpack('>H')[0]

    def u8(self):
        return self.take(1)[0]


def _compact(reader, index):
    offset = reader.pos
    raw = reader.take(32)
    values = struct.unpack('>8f', raw)
    try:
        value = CompactTransform(tuple(values[:4]), tuple(values[4:7]), values[7])
    except (ValueError, ParentTransformError) as error:
        raise RigidHierarchyError(f"invalid compact transform: {error}") from error
    return value, {'index': index, 'source_offset': offset, 'source_bytes': 32,
                   'source_sha256': _sha(raw)}


def _node(reader, ordinal):
    offset = reader.pos
    raw = reader.take(48)
    _require(all(math.isfinite(v) for v in struct.unpack_from('>6f', raw)),
             "nonfinite node bounding box")
    token, word, *shorts = struct.unpack_from('>II8H', raw, 24)
    # Serialized ABB is 24 bytes; native padded ABB is 32 bytes. Consequently
    # source 38/46 correspond to native transform-index+0x2e/parent+0x36.
    value = Node(ordinal, shorts[3], shorts[7], token)
    evidence = {'node_id': ordinal, 'source_offset': offset, 'source_bytes': 48,
                'source_sha256': _sha(raw), 'name_token': token,
                'transform_index': shorts[3], 'parent_node_id': shorts[7],
                'next_sibling_id': shorts[5], 'first_child_id': shorts[6],
                'opaque_word_24': word, 'opaque_u16_fields': {
                    '28': shorts[0], '2a': shorts[1], '2c': shorts[2], '30': shorts[4]}}
    return value, evidence


def _validate_graph(nodes, evidence, transform_count):
    _require(nodes and len({n.name_token for n in nodes}) == len(nodes),
             "empty or duplicate node names")
    _require(nodes[0].parent_node_id == END, "native root 0 must terminate parent chain")
    for i, node in enumerate(nodes):
        _require(node.transform_index < transform_count, "node transform index out of bounds")
        _require(node.parent_node_id == END or node.parent_node_id < len(nodes),
                 "parent node index out of bounds")
        _require(i == 0 or node.parent_node_id != END, "multiple native roots")
        for key in ('next_sibling_id', 'first_child_id'):
            target = evidence[i][key]
            _require(target == END or target < len(nodes), "child/sibling index out of bounds")
        child = evidence[i]['first_child_id']
        sibling = evidence[i]['next_sibling_id']
        _require(child == END or nodes[child].parent_node_id == i,
                 "child parent link is inconsistent")
        _require(sibling == END or nodes[sibling].parent_node_id == node.parent_node_id,
                 "sibling parent link is inconsistent")
    resolved = set()
    for i in range(len(nodes)):
        chain, current = set(), i
        while current != END and current not in resolved:
            _require(current not in chain, "node parent cycle")
            chain.add(current)
            current = nodes[current].parent_node_id
        resolved.update(chain)
    visited, pending = set(), [0]
    while pending:
        current = pending.pop()
        _require(current not in visited, "cyclic or repeated child/sibling links")
        visited.add(current)
        for key in ('first_child_id', 'next_sibling_id'):
            target = evidence[current][key]
            if target != END:
                pending.append(target)
    _require(len(visited) == len(nodes), "node is unreachable by native token lookup")


def _atomic_mesh(reader, ordinal):
    start = reader.pos
    # AtomicMesh version 63 reads 2*u32 and 3*u16 before Renderable::ReadFromStream.
    reader.unpack('>II3H')
    version_offset = reader.pos
    _require(reader.u32() == 29, "unsupported Renderable version; expected 29")
    reader.u32()                    # token at native+a4
    reader.u8()                     # native+a2
    reader.unpack('>4I')             # native+88,+8c,+90,+94
    reader.take(32)                  # native block+0
    reader.u32()                    # native+30
    reader.take(16)                  # native block+20
    reader.u16()                    # native+9e
    reader.u8()                     # native+a0
    reader.u8()                     # native+a1
    reader.u16()                    # native+9c
    reader.unpack('>2I')             # native+34,+38
    # Retail format descriptor differs from DEV: 5*u32, 4*u16, 4*u8.
    reader.unpack('>5I4H4B')
    reader.unpack('>2I')             # tail native+44,+48
    material_token = reader.u32()   # GeometryBase reads the following token
    return {'index': ordinal, 'source_offset': start, 'bytes': reader.pos - start,
            'renderable_version_offset': version_offset, 'material_token': material_token,
            'source_sha256': _sha(reader.raw[start:reader.pos])}


def parse_rigid_resource(raw, record):
    """Parse one caller-bounded ZRB resource header using its typed record.

    raw is the whole containing ZRB byte string. record supplies header_offset,
    header_end, resource_version, class_token and name_token from the resource
    container reader. No node-array offset or transform-count hint is accepted.
    """
    _require(isinstance(record, dict), "expected resource record")
    try:
        start, end = record['header_offset'], record['header_end']
        _integer(start, "header offset")
        _integer(end, "header end")
        _require(44 <= end - start <= MAX_HEADER_BYTES, "invalid resource header size")
        reader = _Reader(raw, start, end)
        common = reader.unpack('>11I')
        _require(common[0] == 6 and common[1] == 0x600df00d,
                 "unsupported common resource header")
        _require(common[3] == record['resource_version'] == 63,
                 "unsupported rigid resource version; expected 63")
        _require(common[8] == RESOURCE_CLASS and record['class_token'] == f'{RESOURCE_CLASS:08x}',
                 "wrong rigid resource class")
        _require(record['name_token'] == f'{common[9]:08x}', "resource name binding mismatch")
        flags = reader.u32()
        _require(all(math.isfinite(v) for v in reader.unpack('>4f6f')),
                 "nonfinite resource bounds")
        render_flags, byte_field = reader.u16(), reader.u8()
        mesh_count, attachment_count = reader.u32(), reader.u32()
        _require(0 <= mesh_count <= MAX_ENTRIES, "mesh count limit")
        _require(attachment_count == 0, "unsupported attachment definitions")
        _require(mesh_count <= (reader.end - reader.pos) // 149, "mesh count exceeds header")
        meshes = [_atomic_mesh(reader, i) for i in range(mesh_count)]
        node_count, transform_count, extra_count, word_cc, word_d0 = reader.unpack('>HHHII')
        _require(0 < node_count <= MAX_ENTRIES and 0 < transform_count <= MAX_ENTRIES,
                 "invalid hierarchy counts")
        _require(32 * transform_count + 48 * node_count <= reader.end - reader.pos,
                 "hierarchy counts exceed header")
        compact_pairs = [_compact(reader, i) for i in range(transform_count)]
        node_start = reader.pos
        node_pairs = [_node(reader, i) for i in range(node_count)]
        nodes, evidence = [p[0] for p in node_pairs], [p[1] for p in node_pairs]
        _validate_graph(nodes, evidence, transform_count)
        return {'format': 'rigid_resource63_prefix_v1', 'name_token': common[9],
                'resource_version': 63, 'node_count': node_count, 'transform_count': transform_count,
                'mesh_count': mesh_count, 'node_array_offset': node_start,
                'nodes': nodes, 'node_evidence': evidence,
                'reference_transforms': [p[0] for p in compact_pairs],
                'transform_evidence': [p[1] for p in compact_pairs], 'mesh_evidence': meshes,
                'source_sha256': _sha(raw[start:end]), 'parsed_prefix_end': reader.pos,
                'opaque_header_fields': {'flags': flags, 'render_flags': render_flags,
                    'byte_field': byte_field, 'extra_count': extra_count,
                    'word_cc': word_cc, 'word_d0': word_d0},
                'unparsed_trailer': {'offset': reader.pos, 'bytes': end - reader.pos,
                                      'sha256': _sha(raw[reader.pos:end])}}
    except (KeyError, TypeError, struct.error, ParentTransformError) as error:
        raise RigidHierarchyError(f"invalid resource header: {error}") from error


def read_compiled_hierarchy(clv, world_geometry_offset, collision_offset):
    """Read a bounded compiled WorldGeometry rigid render instance7 prefix.

    collision_offset is the separately decoded collision record's start. This
    reader verifies the two serialized end pointers and exact root-trailer size;
    it checks the collision record version/presence prefix and leaves its matrix
    and token validation to the separate collision-record decoder.
    """
    reader = _Reader(clv, world_geometry_offset, collision_offset)
    _require(collision_offset + 5 <= len(clv)
             and struct.unpack_from('>I', clv, collision_offset)[0] == 0x10001
             and clv[collision_offset + 4] == 1,
             "compiled collision record is absent or has unsupported version")
    _require(reader.end - reader.pos >= 40, "short compiled hierarchy")
    start = reader.pos
    world_version, collision_end = reader.unpack('>II')
    _require(world_version == 0x10001 and collision_end == collision_offset,
             "wrong compiled WorldGeometry version or collision boundary")
    version, render_end, cls, geometry, name, ordinal, flags, count = reader.unpack('>8I')
    _require(version == 7 and cls == INSTANCE_CLASS, "unsupported compiled rigid instance")
    _require(0 < count <= MAX_ENTRIES, "invalid compiled transform count")
    _require(render_end == reader.pos + count * 32 and render_end + 72 == collision_offset,
             "compiled render end/count/root trailer mismatch")
    pairs = [_compact(reader, i) for i in range(count)]
    _require(reader.pos == render_end, "compiled render boundary mismatch")
    reader.take(72)
    return {'format': 'compiled_rigid_instance7_v1', 'version': version,
            'class_token': cls, 'geometry_token': geometry, 'instance_name_token': name,
            'ordinal': ordinal, 'flags': flags, 'render_end_offset': render_end,
            'transforms': [p[0] for p in pairs], 'transform_evidence': [p[1] for p in pairs],
            'source_offset': start, 'source_bytes': collision_offset - start,
            'source_sha256': _sha(clv[start:collision_offset])}
