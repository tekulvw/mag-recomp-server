"""Independent synthetic arithmetic; contains no map, game or capture data."""
import math
from pathlib import Path
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT / "tools/collision"))

import parent_transforms as transforms

from parent_transforms import (
    CompactTransform, Hierarchy, Node, ParentTransformError,
    compact_to_mat44, mat44_multiply, transform_point,
)


IDENTITY_Q = (0.0, 0.0, 0.0, 1.0)
HALF = math.sqrt(0.5)
ROOT = CompactTransform((0.0, 0.0, HALF, HALF), (10.0, 20.0, 30.0), 2.0)
CHILD = CompactTransform((HALF, 0.0, 0.0, HALF), (1.0, 2.0, 3.0), 3.0)


class ParentTransformTests(unittest.TestCase):
    def assertPoint(self, actual, expected):
        self.assertEqual(len(actual), 3)
        for a, e in zip(actual, expected):
            self.assertAlmostEqual(a, e, places=10)

    def test_compact_scales_basis_and_preserves_its_own_translation(self):
        matrix = compact_to_mat44(ROOT)
        # Positive Z quarter-turn maps +X to +Y; translation is not scaled.
        for actual, expected in zip(matrix, ((0., 2., 0., 0.), (-2., 0., 0., 0.),
                                            (0., 0., 2., 0.), (10., 20., 30., 1.))):
            for a, e in zip(actual, expected):
                self.assertAlmostEqual(a, e, places=10)
        self.assertPoint(transform_point((1., 0., 0.), matrix), (10., 22., 30.))

    def test_noncommuting_child_then_parent_rotation_translation_and_scale(self):
        hierarchy = Hierarchy([Node(0, 40, 0xffff, 100), Node(7, 2, 0, 200)],
                              [(40, ROOT), (2, CHILD)])
        # (1,2,3) -> child scale/Rx90/translation -> (4,-7,9)
        # -> root scale/Rz90/translation -> (24,28,48).
        self.assertPoint(hierarchy.transform_point_for_token(200, (1., 2., 3.)),
                         (24., 28., 48.))
        self.assertPoint(transform_point((0., 0., 0.), hierarchy.world_matrix(7)),
                         (6., 22., 36.))
        # Reversed root*child produces (19,-106,69), a separate literal oracle.
        reversed_matrix = mat44_multiply(compact_to_mat44(ROOT), compact_to_mat44(CHILD))
        self.assertPoint(transform_point((1., 2., 3.), reversed_matrix), (19., -106., 69.))

    def test_two_ancestor_chain_uses_node_links_and_distinct_transform_indices(self):
        middle = CompactTransform((0., HALF, 0., HALF), (4., 5., 6.), 0.5)
        hierarchy = Hierarchy([Node(9, 3, 0xffff, 100), Node(2, 80, 9, 200),
                               Node(50, 7, 2, 300)],
                              [(7, CHILD), (3, ROOT), (80, middle)])
        # Child (4,-7,9) -> middle .5/Ry90/(4,5,6) -> (8.5,1.5,4)
        # -> root -> (7,37,38). Three scales multiply to 3, not 6 or 1.5.
        self.assertPoint(hierarchy.transform_point_for_token(300, (1., 2., 3.)),
                         (7., 37., 38.))
        self.assertPoint(transform_point((0., 0., 0.), hierarchy.world_matrix(50)),
                         (-2., 31., 41.))

    def test_root_is_applied_once_with_no_second_compiled_root_step(self):
        hierarchy = Hierarchy([Node(0, 0, 0xffff, 100), Node(1, 5, 0, 200)],
                              [(0, ROOT), (5, CHILD)])
        self.assertPoint(hierarchy.transform_point_for_token(100, (0., 0., 0.)),
                         (10., 20., 30.))
        self.assertPoint(hierarchy.transform_point_for_token(200, (1., 2., 3.)),
                         (24., 28., 48.))
        # Applying ROOT again would instead give (-46,68,126).
        self.assertPoint(transform_point((24., 28., 48.), compact_to_mat44(ROOT)),
                         (-46., 68., 126.))

    def test_antipodal_quaternion_has_same_pose_without_normalizing_input(self):
        negative = CompactTransform(tuple(-v for v in ROOT.quaternion_xyzw),
                                    ROOT.translation_xyz, ROOT.uniform_scale)
        self.assertEqual(compact_to_mat44(ROOT), compact_to_mat44(negative))

    def test_all_quaternion_components_match_independent_cross_product_oracle(self):
        q = tuple(value / math.sqrt(30.) for value in (1., 2., 3., 4.))
        matrix = compact_to_mat44(CompactTransform(q, (1., 2., 3.), 2.5))
        # For v=(3,5,7), qv cross v = (-1,2,-1)/sqrt(30).
        # v + 2*w*(qv cross v) + 2*qv cross (qv cross v)
        # = (11/5,27/5,7), then scale 5/2 and add translation.
        self.assertPoint(transform_point((3., 5., 7.), matrix), (6.5, 15.5, 20.5))

    def test_zero_and_negative_scales_preserve_translation(self):
        zero = compact_to_mat44(CompactTransform(IDENTITY_Q, (2., 3., 4.), 0.))
        self.assertPoint(transform_point((5., 6., 7.), zero), (2., 3., 4.))
        mirror = compact_to_mat44(CompactTransform(IDENTITY_Q, (2., 3., 4.), -2.))
        self.assertPoint(transform_point((5., 6., 7.), mirror), (-8., -9., -10.))

    def test_invalid_hierarchies_fail_before_any_world_pose_is_returned(self):
        cases = [
            ([Node(0, 0, 0, 100)], [(0, ROOT)]),  # self-cycle
            ([Node(0, 0, 1, 100), Node(1, 0, 0, 200)], [(0, ROOT)]),
            ([Node(0, 0, 4, 100)], [(0, ROOT)]),  # missing parent
            ([Node(0, 3, 0xffff, 100)], [(0, ROOT)]),  # missing transform
            ([Node(0, 0, 0xffff, 100), Node(0, 0, 0xffff, 200)], [(0, ROOT)]),
            ([Node(0, 0, 0xffff, 100), Node(1, 0, 0xffff, 100)], [(0, ROOT)]),
            ([Node(0, 0, 0xffff, 100)], [(0, ROOT), (0, CHILD)]),
            ([], [(0, ROOT)]),
        ]
        for nodes, transforms in cases:
            with self.subTest(nodes=nodes, transforms=transforms):
                with self.assertRaises(ParentTransformError):
                    Hierarchy(nodes, transforms)

    def test_missing_token_or_selected_node_is_not_an_identity_fallback(self):
        hierarchy = Hierarchy([Node(0, 0, 0xffff, 100)], [(0, ROOT)])
        for action in [lambda: hierarchy.world_matrix(9),
                       lambda: hierarchy.world_matrix_for_token(999),
                       lambda: hierarchy.transform_point_for_token(999, (0., 0., 0.))]:
            with self.assertRaises(ParentTransformError):
                action()

    def test_depth_limit_accounts_for_cached_ancestor_depth(self):
        nodes = [Node(0, 0, 0xffff, 100), Node(1, 0, 0, 200), Node(2, 0, 1, 300)]
        with self.assertRaises(ParentTransformError):
            Hierarchy(nodes, [(0, ROOT)], max_depth=2)
        self.assertEqual(Hierarchy(nodes, [(0, ROOT)], max_depth=3).depth(2), 3)

    def test_numeric_shape_domain_and_overflow_fail_closed(self):
        for quaternion, position, scale in [
            ((0., 0., 0., 0.), (0., 0., 0.), 1.),
            ((0., 0., 0., 2.), (0., 0., 0.), 1.),
            ((0., 0., 0.), (0., 0., 0.), 1.),
            (IDENTITY_Q, (0., float('nan'), 0.), 1.),
            (IDENTITY_Q, (0., 0., 0.), float('inf')),
        ]:
            with self.assertRaises(ParentTransformError):
                compact_to_mat44(CompactTransform(quaternion, position, scale))
        for node in [lambda: Node(True, 0, 0xffff, 100),
                     lambda: Node(0xffff, 0, 0xffff, 100),
                     lambda: Node(0, -1, 0xffff, 100),
                     lambda: Node(0, 0, 0xffff, 2**32)]:
            with self.assertRaises(ParentTransformError):
                node()
        with self.assertRaises(ParentTransformError):
            transform_point((float('inf'), 0., 0.), compact_to_mat44(ROOT))
        huge = compact_to_mat44(CompactTransform(IDENTITY_Q, (0., 0., 0.), 1e308))
        with self.assertRaises(ParentTransformError):
            mat44_multiply(huge, huge)

    def test_caches_are_not_mutable_after_validation(self):
        hierarchy = Hierarchy([Node(0, 0, 0xffff, 100)], [(0, ROOT)])
        with self.assertRaises(TypeError):
            hierarchy.nodes[1] = Node(1, 0, 0xffff, 200)

    def test_collision_scale_w_or_projective_matrix_cannot_be_an_affine_pose(self):
        identity = compact_to_mat44(CompactTransform(IDENTITY_Q, (0., 0., 0.), 1.))
        for row, column, value in [(3, 3, 0.5), (0, 3, 1.)]:
            matrix = [list(row) for row in identity]
            matrix[row][column] = value
            with self.assertRaises(ParentTransformError):
                transform_point((0., 0., 0.), matrix)


class CollisionReferenceTests(unittest.TestCase):
    def assertMatrix(self, actual, expected):
        self.assertEqual(len(actual), 4)
        for actual_row, expected_row in zip(actual, expected):
            self.assertEqual(len(actual_row), 4)
            for a, e in zip(actual_row, expected_row):
                self.assertAlmostEqual(a, e, places=12)

    def test_scaled_basis_changes_extracted_rotation_before_result_normalization(self):
        world = ((0., .75, 0., 0.), (-.75, 0., 0., 0.),
                 (0., 0., .75, 0.), (1., 2., 3., 1.))
        result = transforms.collision_reference_matrix(world, 2.)
        # Raw matrix gives qz/qw=6/7. Normalized q=(0,0,6,7)/sqrt(85),
        # hence cos(theta)=13/85 and sin(theta)=84/85, not theta=90 degrees.
        expected = ((26/85, 168/85, 0., 0.), (-168/85, 26/85, 0., 0.),
                    (0., 0., 2., 0.), (1., 2., 3., 1.))
        self.assertMatrix(result, expected)
        self.assertAlmostEqual(math.degrees(math.atan2(result[0][1], result[0][0])),
                               81.20258929000894, places=12)

    def test_unit_mixed_axis_rotation_roundtrips_with_translation(self):
        world = ((2/15, 14/15, -1/3, 0.), (-2/3, 1/3, 2/3, 0.),
                 (11/15, 2/15, 2/3, 0.), (4., -3., 2., 1.))
        self.assertMatrix(transforms.collision_reference_matrix(world, 1.), world)

    def test_largest_diagonal_branches_roundtrip_half_turns(self):
        matrices = [
            ((1., 0., 0., 0.), (0., -1., 0., 0.), (0., 0., -1., 0.), (5., -2., 4., 1.)),
            ((-1., 0., 0., 0.), (0., 1., 0., 0.), (0., 0., -1., 0.), (5., -2., 4., 1.)),
            ((-1., 0., 0., 0.), (0., -1., 0., 0.), (0., 0., 1., 0.), (5., -2., 4., 1.)),
            ((-6/7, 2/7, 3/7, 0.), (2/7, -3/7, 6/7, 0.),
             (3/7, 6/7, 2/7, 0.), (5., -2., 4., 1.)),
        ]
        for matrix in matrices:
            with self.subTest(matrix=matrix):
                self.assertMatrix(transforms.collision_reference_matrix(matrix, 1.), matrix)

    def test_largest_diagonal_ties_retain_x_before_y_before_z(self):
        # Invented nonsingular raw matrices expose tie selection; these are
        # algebraic branch tests, not claims about authored collision inputs.
        x_tie = ((0., 1., 0., 0.), (.5, 0., 0., 0.),
                 (0., 0., -1., 0.), (0., 0., 0., 1.))
        expected_x = ((7/25, 24/25, 0., 0.), (24/25, -7/25, 0., 0.),
                      (0., 0., -1., 0.), (0., 0., 0., 1.))
        self.assertMatrix(transforms.collision_reference_matrix(x_tie, 1.), expected_x)
        y_tie = ((-1., 0., 0., 0.), (0., 0., 1., 0.),
                 (0., .5, 0., 0.), (0., 0., 0., 1.))
        expected_y = ((-1., 0., 0., 0.), (0., 7/25, 24/25, 0.),
                      (0., 24/25, -7/25, 0.), (0., 0., 0., 1.))
        self.assertMatrix(transforms.collision_reference_matrix(y_tie, 1.), expected_y)

    def test_zero_trace_cyclic_rotation_is_supported(self):
        world = ((0., 1., 0., 0.), (0., 0., 1., 0.),
                 (1., 0., 0., 0.), (0., 0., 0., 1.))
        self.assertMatrix(transforms.collision_reference_matrix(world, 1.), world)

    def test_exact_zero_trace_uses_largest_diagonal_branch(self):
        world = ((0., 2., 0., 0.), (0., 0., 1., 0.),
                 (1., 0., 0., 0.), (0., 0., 0., 1.))
        # The fallback's extracted quaternion is (1,2,1,1)/sqrt(7).
        # A trace>=0 implementation instead picks (1,1,2,1)/sqrt(7).
        expected = ((-3/7, 6/7, -2/7, 0.), (2/7, 3/7, 6/7, 0.),
                    (6/7, 2/7, -3/7, 0.), (0., 0., 0., 1.))
        self.assertMatrix(transforms.collision_reference_matrix(world, 1.), expected)

    def test_degenerate_nonfinite_or_nonaffine_inputs_are_rejected(self):
        valid = ((1., 0., 0., 0.), (0., 1., 0., 0.),
                 (0., 0., 1., 0.), (0., 0., 0., 1.))
        bad_matrices = [
            ((0., 0., 0., 0.),)*3 + ((0., 0., 0., 1.),),
            ((1., 0., 0., 0.), (1., 0., 0., 0.), (0., 0., 1., 0.), (0., 0., 0., 1.)),
        ]
        for row, column, value in [(0, 0, float('nan')), (1, 1, float('inf')), (3, 3, .5)]:
            matrix = [list(row) for row in valid]
            matrix[row][column] = value
            bad_matrices.append(matrix)
        for matrix in bad_matrices:
            with self.subTest(matrix=matrix):
                with self.assertRaises(ParentTransformError):
                    transforms.collision_reference_matrix(matrix, 1.)
        for scale in [0., -1., float('inf'), float('nan')]:
            with self.subTest(scale=scale):
                with self.assertRaises(ParentTransformError):
                    transforms.collision_reference_matrix(valid, scale)


if __name__ == '__main__':
    unittest.main()
