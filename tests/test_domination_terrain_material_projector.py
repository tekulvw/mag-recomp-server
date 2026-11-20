import runpy
import struct
import unittest
from pathlib import Path


PROJECTOR = runpy.run_path(str(
    Path(__file__).resolve().parents[1]
    / 'tools/extraction/project-retail-domination-terrain-materials.py'
))


def block(alpha, color0, color1, color_indices=0, alpha_indices=0):
    a0, a1 = alpha
    return (bytes([a0, a1]) + alpha_indices.to_bytes(6, 'little')
            + struct.pack('<HHI', color0, color1, color_indices))


class TerrainMaterialProjectorTests(unittest.TestCase):
    def test_little_endian_bc3_selectors_and_channel_ties(self):
        color_pattern = sum((index % 4) << (2 * index) for index in range(16))
        red_green = block((0, 0), 0xF800, 0x07E0, color_pattern)
        self.assertEqual(PROJECTOR['selected_channels'](red_green), [0, 1, 0, 1] * 4)
        self.assertEqual(PROJECTOR['selected_channels'](block((255, 0), 0xFFFF, 0)), [3] * 16)
        self.assertEqual(PROJECTOR['selected_channels'](block((0, 0), 0xFFFF, 0)), [2] * 16)

    def test_uses_both_bc3_alpha_palettes(self):
        # DXT5 selector 7 interpolates when alpha0 > alpha1 and is 255 otherwise.
        selectors = sum(7 << (3 * index) for index in range(16))
        gray = 0x8410
        self.assertEqual(
            PROJECTOR['selected_channels'](block((245, 0), gray, 0, alpha_indices=selectors)),
            [2] * 16,
        )
        self.assertEqual(
            PROJECTOR['selected_channels'](block((0, 245), gray, 0, alpha_indices=selectors)),
            [3] * 16,
        )

    def test_maps_tiles_into_pixel_rows_without_swapping_block_axes(self):
        red = block((0, 0), 0xF800, 0)
        green = block((0, 0), 0x07E0, 0)
        blue = block((0, 0), 0x001F, 0)
        alpha = block((255, 0), 0, 0)
        result = PROJECTOR['project_texture'](red + green + blue + alpha, 8, [4, 5, 6, 7])
        self.assertEqual(result, bytes([4] * 4 + [5] * 4) * 4 + bytes([6] * 4 + [7] * 4) * 4)

    def test_rejects_an_incomplete_texture_and_invalid_slots(self):
        valid = block((0, 0), 0xF800, 0)
        project = PROJECTOR['project_texture']
        for raw, size, slots in [(valid[:-1], 4, [0, 1, 2, 3]),
                                 (valid, 3, [0, 1, 2, 3]),
                                 (valid, 4, [0, 1, 2, 256])]:
            with self.subTest(size=size, slots=slots), self.assertRaises(ValueError):
                project(raw, size, slots)
