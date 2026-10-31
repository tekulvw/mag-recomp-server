"""Reference-pose geometry arithmetic for compact row-vector hierarchies.

Matrices are 4 rows of 4 Python floats; points multiply on the left. Compact
poses contain xyzw quaternion, xyz translation and one uniform scale. The scale
multiplies the three basis rows, not the pose's own translation. The affine last
column is (0,0,0,1). A node's world matrix is local * parent * ... * root, once.

This replacement math supports explicit reference-pose geometry only. The
collision_reference_matrix helper additionally models raw scaled-basis
quaternion extraction for a reference collision pose, normalizing its resulting
quaternion and then using the separate compiled shape scale. It never removes
basis scale before extraction. This is scalar float64 arithmetic, not bit-exact
PPC SIMD/fused f32 emulation, live animation reconstruction or a claim of complete
collision-consumer parity. It does not parse assets or convert the separate
compiled collision matrix storage representation. That representation's source
bottom-right scale is NOT affine homogeneous W. No extra root step is added.

Limits: 65,535 nodes and transform entries; native 0xffff is parent terminator;
default maximum depth 4,096 nodes. IDs must be explicit unsigned 16-bit values
below the sentinel, tokens unsigned 32-bit. Duplicate IDs/tokens, absent links,
cycles and unresolvable queries raise ParentTransformError. Quaternion squared
norm for CompactTransform must be within 1e-4 of one; compact input is never
silently renormalized. Finite zero and negative compact scales are representable
math. The collision helper requires a positive compiled shape scale and a finite
nonsingular affine world matrix; nonroot scales must be checked by its caller.
"""
from dataclasses import dataclass
import math
from types import MappingProxyType
from typing import Iterable


PARENT_END = 0xffff
MAX_ENTRIES = 0xffff
DEFAULT_MAX_DEPTH = 4096
QUATERNION_NORM_TOLERANCE = 1e-4
AFFINE_W_TOLERANCE = 1e-9
RESULT_QUATERNION_MIN_NORM_SQUARED = 1.0000000031710769e-30
Matrix44 = tuple[tuple[float, float, float, float], ...]


class ParentTransformError(ValueError):
    """A malformed/unresolved pose cannot produce a substitute world pose."""


def _scalar(value, name: str) -> float:
    if isinstance(value, bool) or not isinstance(value, (int, float)):
        raise ParentTransformError(f"{name} must be a finite number")
    try:
        result = float(value)
    except (OverflowError, ValueError) as error:
        raise ParentTransformError(f"{name} must be finite") from error
    if not math.isfinite(result):
        raise ParentTransformError(f"{name} must be finite")
    return result


def _vector(values, size: int, name: str) -> tuple[float, ...]:
    try:
        result = tuple(values)
    except TypeError as error:
        raise ParentTransformError(f"{name} must have {size} components") from error
    if len(result) != size:
        raise ParentTransformError(f"{name} must have {size} components")
    return tuple(_scalar(value, name) for value in result)


def _index(value, name: str, maximum: int = PARENT_END - 1) -> int:
    if type(value) is not int or not 0 <= value <= maximum:
        raise ParentTransformError(f"{name} outside integer range 0..{maximum}")
    return value


def _matrix(matrix) -> Matrix44:
    try:
        rows = tuple(matrix)
    except TypeError as error:
        raise ParentTransformError("matrix must have four rows") from error
    if len(rows) != 4:
        raise ParentTransformError("matrix must have four rows")
    return tuple(_vector(row, 4, "matrix row") for row in rows)


@dataclass(frozen=True)
class CompactTransform:
    quaternion_xyzw: tuple[float, float, float, float]
    translation_xyz: tuple[float, float, float]
    uniform_scale: float = 1.0

    def __post_init__(self):
        quaternion = _vector(self.quaternion_xyzw, 4, "quaternion")
        translation = _vector(self.translation_xyz, 3, "translation")
        scale = _scalar(self.uniform_scale, "uniform scale")
        norm_squared = sum(value * value for value in quaternion)
        if not math.isfinite(norm_squared) or abs(norm_squared - 1.0) > QUATERNION_NORM_TOLERANCE:
            raise ParentTransformError("quaternion squared norm must be within 1e-4 of one")
        object.__setattr__(self, "quaternion_xyzw", quaternion)
        object.__setattr__(self, "translation_xyz", translation)
        object.__setattr__(self, "uniform_scale", scale)


def compact_to_mat44(transform: CompactTransform) -> Matrix44:
    """Return the affine matrix; positive Z quarter-turn maps +X to +Y."""
    if not isinstance(transform, CompactTransform):
        raise ParentTransformError("expected CompactTransform")
    x, y, z, w = transform.quaternion_xyzw
    scale = transform.uniform_scale
    return _matrix((
        ((1.0 - 2.0 * (y*y + z*z)) * scale,
         2.0 * (x*y + z*w) * scale, 2.0 * (x*z - y*w) * scale, 0.0),
        (2.0 * (x*y - z*w) * scale,
         (1.0 - 2.0 * (x*x + z*z)) * scale, 2.0 * (y*z + x*w) * scale, 0.0),
        (2.0 * (x*z + y*w) * scale, 2.0 * (y*z - x*w) * scale,
         (1.0 - 2.0 * (x*x + y*y)) * scale, 0.0),
        (*transform.translation_xyz, 1.0),
    ))


def mat44_multiply(left: Matrix44, right: Matrix44) -> Matrix44:
    """Ordinary matrix product; for row vectors left acts before right."""
    left, right = _matrix(left), _matrix(right)
    return _matrix(tuple(tuple(sum(left[row][k] * right[k][column] for k in range(4))
                               for column in range(4)) for row in range(4)))


def transform_point(point_xyz, matrix: Matrix44) -> tuple[float, float, float]:
    """Apply one affine matrix. Nonunit homogeneous W is rejected, not divided."""
    point = (*_vector(point_xyz, 3, "point"), 1.0)
    matrix = _matrix(matrix)
    if any(abs(matrix[row][3] - (1.0 if row == 3 else 0.0)) > AFFINE_W_TOLERANCE
           for row in range(4)):
        raise ParentTransformError("point transform requires affine last column (0,0,0,1)")
    result = tuple(_scalar(sum(point[k] * matrix[k][column] for k in range(4)),
                           "transformed point") for column in range(4))
    if abs(result[3] - 1.0) > AFFINE_W_TOLERANCE:
        raise ParentTransformError("point transform requires affine homogeneous W=1")
    return result[:3]


def collision_reference_matrix(world_matrix: Matrix44, compiled_shape_scale: float) -> Matrix44:
    """Model the reference collision pose using raw-basis quaternion extraction.

    This intentionally does NOT remove world basis scale before extracting the
    quaternion. Only the resulting quaternion is normalized. The reconstructed
    affine matrix carries the separate positive compiled shape scale and world
    translation. Callers must verify that nonroot compact scales are exactly one
    at the resolver layer; that fact cannot be inferred from this matrix alone.

    This uses scalar float64 sqrt normalization, not SIMD reciprocal-square-root
    estimation/refinement. It models branch semantics, not native bit equality,
    arbitrary collision systems or live animated transforms. Malformed affine
    matrices, singular bases, nonfinite arithmetic and nonpositive compiled shape
    scales fail closed before any replacement collision pose is returned.
    """
    matrix = _matrix(world_matrix)
    if any(abs(matrix[row][3] - (1.0 if row == 3 else 0.0)) > AFFINE_W_TOLERANCE
           for row in range(4)):
        raise ParentTransformError("collision reference requires affine last column (0,0,0,1)")
    shape_scale = _scalar(compiled_shape_scale, "compiled shape scale")
    if shape_scale <= 0.0:
        raise ParentTransformError("compiled shape scale must be positive")
    determinant = _scalar(
        matrix[0][0] * (matrix[1][1]*matrix[2][2] - matrix[1][2]*matrix[2][1])
        - matrix[0][1] * (matrix[1][0]*matrix[2][2] - matrix[1][2]*matrix[2][0])
        + matrix[0][2] * (matrix[1][0]*matrix[2][1] - matrix[1][1]*matrix[2][0]),
        "world basis determinant",
    )
    if determinant == 0.0:
        raise ParentTransformError("singular world basis")
    trace = _scalar(matrix[0][0] + matrix[1][1] + matrix[2][2], "world basis trace")
    if trace > 0.0:
        root = math.sqrt(trace + 1.0)
        factor = 0.5 / root
        quaternion = [
            (matrix[1][2] - matrix[2][1]) * factor,
            (matrix[2][0] - matrix[0][2]) * factor,
            (matrix[0][1] - matrix[1][0]) * factor,
            0.5 * root,
        ]
    else:
        axis = 0
        if matrix[1][1] > matrix[0][0]:
            axis = 1
        if matrix[2][2] > matrix[axis][axis]:
            axis = 2
        following = (axis + 1) % 3
        remaining = (following + 1) % 3
        radicand = _scalar(1.0 + matrix[axis][axis]
                            - (matrix[following][following] + matrix[remaining][remaining]),
                            "matrix quaternion radicand")
        if radicand < 0.0:
            raise ParentTransformError("negative matrix quaternion radicand")
        root = math.sqrt(radicand)
        factor = 0.5 / root if root != 0.0 else 0.0
        quaternion = [0.0] * 4
        quaternion[axis] = 0.5 * root
        quaternion[following] = (matrix[axis][following] + matrix[following][axis]) * factor
        quaternion[remaining] = (matrix[axis][remaining] + matrix[remaining][axis]) * factor
        quaternion[3] = (matrix[following][remaining] - matrix[remaining][following]) * factor
    norm_squared = _scalar(sum(value * value for value in quaternion), "result quaternion norm")
    if norm_squared > RESULT_QUATERNION_MIN_NORM_SQUARED:
        norm = math.sqrt(norm_squared)
        quaternion = tuple(value / norm for value in quaternion)
    else:
        quaternion = (0.0, 0.0, 0.0, 1.0)
    return compact_to_mat44(CompactTransform(quaternion, matrix[3][:3], shape_scale))


@dataclass(frozen=True)
class Node:
    """Node identity and transform index are separate native table identities."""
    node_id: int
    transform_index: int
    parent_node_id: int
    name_token: int

    def __post_init__(self):
        _index(self.node_id, "node ID")
        _index(self.transform_index, "transform index")
        _index(self.parent_node_id, "parent node ID", PARENT_END)
        _index(self.name_token, "name token", 0xffffffff)


class Hierarchy:
    """Validate the entire hierarchy and cache immutable world matrices.

    Pass transform_entries as (index, CompactTransform) pairs rather than a
    pre-collapsed dictionary when duplicate source entries must be detected.
    Every node is validated, including nodes other than the selected query.
    """

    def __init__(self, nodes: Iterable[Node],
                 transform_entries: Iterable[tuple[int, CompactTransform]],
                 *, max_depth: int = DEFAULT_MAX_DEPTH):
        self.max_depth = _index(max_depth, "max depth", MAX_ENTRIES)
        if not self.max_depth:
            raise ParentTransformError("max depth must be positive")
        node_table = {}
        token_table = {}
        for node in nodes:
            if not isinstance(node, Node):
                raise ParentTransformError("expected Node entry")
            if node.node_id in node_table:
                raise ParentTransformError("duplicate node ID")
            if node.name_token in token_table:
                raise ParentTransformError("duplicate name token")
            if len(node_table) >= MAX_ENTRIES:
                raise ParentTransformError("node count exceeds 65535")
            node_table[node.node_id] = node
            token_table[node.name_token] = node.node_id
        if not node_table:
            raise ParentTransformError("empty hierarchy")
        transforms = {}
        for entry in transform_entries:
            try:
                index, transform = entry
            except (TypeError, ValueError) as error:
                raise ParentTransformError("expected indexed transform pair") from error
            _index(index, "transform index")
            if index in transforms:
                raise ParentTransformError("duplicate transform index")
            if not isinstance(transform, CompactTransform):
                raise ParentTransformError("expected CompactTransform entry")
            if len(transforms) >= MAX_ENTRIES:
                raise ParentTransformError("transform count exceeds 65535")
            transforms[index] = transform
        for node in node_table.values():
            if node.parent_node_id != PARENT_END and node.parent_node_id not in node_table:
                raise ParentTransformError("missing parent node")
            if node.transform_index not in transforms:
                raise ParentTransformError("missing node transform")
        self.nodes = MappingProxyType(node_table)
        self.transforms = MappingProxyType(transforms)
        self._tokens = MappingProxyType(token_table)
        matrices = {}
        depths = {}
        for start in node_table:
            path = []
            visited = set()
            current = start
            while current not in matrices:
                if current in visited:
                    raise ParentTransformError("parent cycle")
                visited.add(current)
                path.append(current)
                if len(path) > self.max_depth:
                    raise ParentTransformError("parent depth exceeds limit")
                parent = node_table[current].parent_node_id
                if parent == PARENT_END:
                    result, depth = None, 0
                    break
                current = parent
            else:
                result, depth = matrices[current], depths[current]
            while path:
                current = path.pop()
                depth += 1
                if depth > self.max_depth:
                    raise ParentTransformError("parent depth exceeds limit")
                local = compact_to_mat44(transforms[node_table[current].transform_index])
                result = local if result is None else mat44_multiply(local, result)
                matrices[current], depths[current] = result, depth
        self._matrices = MappingProxyType(matrices)
        self._depths = MappingProxyType(depths)

    def world_matrix(self, node_id: int) -> Matrix44:
        _index(node_id, "selected node ID")
        try:
            return self._matrices[node_id]
        except KeyError as error:
            raise ParentTransformError("unresolved selected node") from error

    def world_matrix_for_token(self, token: int) -> Matrix44:
        _index(token, "selected token", 0xffffffff)
        try:
            return self.world_matrix(self._tokens[token])
        except KeyError as error:
            raise ParentTransformError("unresolved name token") from error

    def transform_point_for_token(self, token: int, point_xyz) -> tuple[float, float, float]:
        return transform_point(point_xyz, self.world_matrix_for_token(token))

    def depth(self, node_id: int) -> int:
        self.world_matrix(node_id)
        return self._depths[node_id]
