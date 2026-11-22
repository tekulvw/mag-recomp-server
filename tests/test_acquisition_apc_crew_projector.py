import runpy
import unittest
from pathlib import Path


class AcquisitionApcCrewProjectorTest(unittest.TestCase):
    def test_projection_preserves_passenger_and_escort_policy(self):
        project = runpy.run_path(str(Path(__file__).resolve().parents[1] / "tools/extraction/project-acquisition-apc-crew.py"))["upgrade_ground"]
        roster = [{"index": i, "passenger_seats": list(range(2, 10)), "maximum_occupancy": 8,
                   "passenger_exit_candidates": [{"position_offset": [1, 0, 0], "ranking_offset": [1, 0, 0]}]}
                  for i in range(8)]
        ground = {"schema_version": 1, "vehicles": roster,
                  "vehicle_sources": [{"vehicle_index": i, "spawn_type": 3 if i < 4 else 9} for i in range(8)],
                  "driver_seats": [{"vehicle_index": i, "seat": 0, "maximum_speed_mps": 20} for i in range(4, 8)],
                  "seat_visibility": {"vehicles": [{"vehicle_index": i, "seats": [
                      {"seat": n, "occupant_visible": False} for n in range(2, 10)]} for i in range(4)]}}
        result = project(ground, set(range(4)))
        self.assertEqual(result["schema_version"], 2)
        self.assertEqual(result["driver_seats"][:4], ground["driver_seats"])
        self.assertEqual([d["vehicle_index"] for d in result["driver_seats"][4:]], list(range(4)))
        self.assertEqual(result["vehicles"][0]["passenger_seats"], list(range(2, 10)))
        self.assertEqual(result["vehicles"][0]["maximum_occupancy"], 8)
        self.assertEqual(result["seat_visibility"]["vehicles"][0]["seats"][0],
                         {"seat": 0, "occupant_visible": False})
        self.assertEqual(ground["schema_version"], 1)
        with self.assertRaises(ValueError):
            project(ground, {0, 1, 2, 4})
