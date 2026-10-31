"""Source-bound retail parent attachments for explicit reference-pose exports.

This validates the identity of an externally reviewed native witness. It does
not reproduce that native analysis or observe animation. Original inputs and
the witness are supplied locally; neither belongs in the public repository.
"""
import hashlib
import json
import math
import struct

from parent_transforms import (
    Hierarchy, collision_reference_matrix, compact_to_mat44, transform_point,
)
from retail_terrain import RETAIL_SHA, require
from rigid_hierarchy import parse_rigid_resource, read_compiled_hierarchy
from zrb_records import parse


REQUIRED_NATIVE_FUNCTIONS = frozenset({
    'RigidMeshNode_Read', 'GeometryRigidHierachy_ReadHeader',
    'GeometryRigidHierachyInst_Read', 'FindNodeRecursive', 'FindNode',
    'AnimatedObject_UpdateCollision', 'AnimatedObject_SetCollisionData',
    'GetTransformInWorldSpace_Mat44', 'TransformCompactToMat44',
    'Mat44_multiply', 'Quat_ctor_from_Mat44', 'GeometryBase_ReadHeader',
    'GeometryInstBase_Read', 'AtomicMesh_ReadDataFromStream',
    'CPhysicsResource_ReadHeader', 'Renderable_ReadFromStream', 'util_Token_Read',
})


def sha(data):
    return hashlib.sha256(data).hexdigest()


def elf_span(data, address, size):
    """Read a file-backed virtual span; never execute or load the input."""
    require(len(data) >= 64 and data[:6] == b'\x7fELF\x02\x02',
            'parent witness requires a big-endian ELF64 input')
    require(type(address) is int and type(size) is int and address >= 0
            and 0 < size <= 1024 * 1024, 'invalid native function span')
    phoff = struct.unpack_from('>Q', data, 32)[0]
    entry_size, count = struct.unpack_from('>HH', data, 54)
    require(entry_size == 56 and 0 < count <= 1024
            and phoff + entry_size * count <= len(data), 'invalid ELF segment table')
    matches = []
    for index in range(count):
        kind, _, offset, virtual, _, length, _, _ = struct.unpack_from(
            '>II6Q', data, phoff + index * entry_size)
        require(offset + length <= len(data), 'ELF segment outside input')
        if kind == 1 and virtual <= address and address + size <= virtual + length:
            start = offset + address - virtual
            matches.append(data[start:start + size])
    require(len(matches) == 1, 'native witness span lacks one file-backed segment')
    return matches[0]


def verify_native_proof(proof_path, retail_path):
    raw = proof_path.read_bytes()
    proof = json.loads(raw)
    require(proof.get('schema') == 'mag_retail_parent_transforms_v1',
            'unsupported external parent witness')
    elf = retail_path.read_bytes()
    require(sha(elf) == RETAIL_SHA == proof.get('source_elf_sha256'),
            'parent witness ELF identity mismatch')
    records = proof.get('functions')
    require(isinstance(records, list) and len(records) <= 128,
            'missing or excessive native parent witnesses')
    names = set()
    for record in records:
        name = record['name']
        require(name not in names, 'duplicate native witness function')
        names.add(name)
        address = int(record['retail_address'], 16)
        require(sha(elf_span(elf, address, record['retail_size']))
                == record['retail_sha256'], 'native function witness bytes changed')
    require(REQUIRED_NATIVE_FUNCTIONS <= names, 'incomplete native parent witness')
    require(proof.get('composition') == 'row_vector_child_parent_to_root_once'
            and proof.get('scope') == 'compiled_reference_pose',
            'unsupported parent witness scope/composition')
    return {'schema': proof['schema'], 'sha256': sha(raw),
            'source_elf_sha256': RETAIL_SHA,
            'function_names': sorted(names),
            'verification': 'External analysis identity and source spans checked; '
                            'native analysis is not regenerated.'}


class ParentResolver:
    """Derive attachments from typed original headers and compiled transforms."""

    def __init__(self, archive, clv, proof):
        candidates = [name for name in archive.names
                      if name.endswith('/resource_blocks/globaldata.zrb')]
        require(len(candidates) == 1, 'parent geometry requires one globaldata member')
        self.member = candidates[0]
        self.raw = archive.extract(archive.names.index(self.member) + 1)
        self.source_sha256 = sha(self.raw)
        parsed = parse(self.raw, retail13_compat=True)
        self.resources = {}
        for record in parsed['records']:
            token = int(record['name_token'], 16)
            # Different resource classes may share a token. Only the typed
            # rigid hierarchy is eligible; duplicate hierarchies are ambiguous.
            if record['class_token'] == 'a4f4d23c':
                require(token not in self.resources, 'duplicate rigid hierarchy resource')
                self.resources[token] = record
        self.clv = clv
        self.proof = proof
        self.cache = {}
        self.bindings = []

    def resolve(self, record, pack):
        render = read_compiled_hierarchy(
            self.clv, record['world_geometry_offset'], record['collision_offset'])
        token = render['geometry_token']
        require(token in self.resources, 'compiled rigid hierarchy resource is absent')
        if token not in self.cache:
            self.cache[token] = parse_rigid_resource(self.raw, self.resources[token])
        resource = self.cache[token]
        require(len(render['transforms']) == resource['transform_count'],
                'compiled and resource transform counts differ')
        hierarchy = Hierarchy(resource['nodes'], enumerate(render['transforms']))
        parent_token = int(pack['resource_binding']['parent_token'], 16)
        selected = [node for node in resource['nodes'] if node.name_token == parent_token]
        require(len(selected) == 1, 'collision parent does not resolve uniquely')
        chain = []
        current = selected[0]
        while True:
            chain.append(current)
            if current.parent_node_id == 0xffff:
                break
            current = hierarchy.nodes[current.parent_node_id]
        # Native collision scales the shape using the compiled root scalar.
        # Child scale is not covered by that consumer contract.
        require(all(render['transforms'][node.transform_index].uniform_scale == 1.0
                    for node in chain[:-1]), 'nonunit child collision scale is unsupported')
        root_matrix = compact_to_mat44(render['transforms'][chain[-1].transform_index])
        placement = record['placement']
        error = max(abs(root_matrix[row][column] - placement['matrix'][column][row])
                    for row in range(4) for column in range(3))
        require(math.isfinite(error) and error <= 1e-4,
                'hierarchy root differs from compiled collision placement')
        world = hierarchy.world_matrix_for_token(parent_token)
        matrix = collision_reference_matrix(world, placement['scale'])
        self.bindings.append({
            'collision_offset': record['collision_offset'],
            'record_sha256': placement['sha256'],
            'pack_sha256': pack['source_sha256'],
            'parent_token': f'{parent_token:08x}',
            'geometry_token': f'{token:08x}',
            'node_chain_selected_to_root': [node.node_id for node in chain],
            'transform_indices_selected_to_root': [node.transform_index for node in chain],
            'compiled_render_sha256': render['source_sha256'],
            'geometry_header_offset': self.resources[token]['header_offset'],
            'geometry_header_sha256': sha(self.raw[
                self.resources[token]['header_offset']:self.resources[token]['header_end']]),
            'root_matrix_max_abs_difference': error,
            'compiled_shape_scale': placement['scale'],
            'reference_collision_matrix_row_vector': matrix,
        })
        return matrix

    def report(self):
        return {'scope': 'compiled_reference_pose', 'live_animation_reconstructed': False,
                'arithmetic': 'scalar float64 model; not bit-exact native SIMD',
                'source_member': self.member, 'source_sha256': self.source_sha256,
                'native_witness': self.proof, 'bindings': self.bindings}


def bake_parent_vertex(vertex, matrix):
    values = transform_point(vertex, matrix)
    return [struct.unpack('>f', struct.pack('>f', value))[0] for value in values]
