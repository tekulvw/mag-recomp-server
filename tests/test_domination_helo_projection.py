import json
from pathlib import Path
import runpy
import unittest

TOOL = runpy.run_path(str(
    Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-domination-helo.py'
))
encode, project, sha = (TOOL[key] for key in ('encode', 'project', 'sha'))


def fixture():
    helo_points = [{
        'packed_family35_id': 0x23800000 | index,
        'candidate_family2_index': index,
        'spawn_type': 4,
        'point_token': 100 + index,
        'vehicle_type_token': 200,
    } for index in range(4)]
    spawns = {
        'format': 'mag_spawn_catalog',
        'match_profile': {'game_type': 2, 'auto_assign_teams': True, 'attacking_faction': 1},
        'active': {'map_name': 'invented_map'},
        'maps': [{
            'name': 'invented_map', 'runtime_indices_verified': True,
            'locations': [{'index': index, 'token': 100 + index, 'kind': 'unsupported', 'active': False}
                          for index in range(4)],
        }],
        'source': json.dumps({
            'game_archive_sha256': 'invented-game',
            'level_clv_sha256': 'invented-level',
            'native_spawn_bindings_sha256': 'invented-bindings',
            'unsupported_spawn_links': [{
                'index': index, 'spawn_type': 4, 'category': 2, 'user_class': 0, 'active': False,
                'squad_links': [[0, index, 0, 2], [0, index, 1, 2]],
            } for index in range(4)],
        }),
    }
    links = {
        'schema': 'mag_private_retail_domination_aircraft_links_v1',
        'retail_elf_sha256': 'invented-native',
        'initial_spawn_bindings_sha256': 'invented-bindings',
        'vehicle_registry_sha256': 'invented-registry',
        'maps': [{'map': 'invented_map', 'helo_points': helo_points}],
    }
    paths = {
        'schema': 'mag_private_retail_domination_helo_paths_v1',
        'retail_elf_sha256': 'invented-native',
        'aircraft_links_sha256': sha(encode(links)),
        'maps': [{
            'map': 'invented_map', 'level_clv_sha256': 'invented-level',
            'paths': [{
                'point_token': 100 + index, 'authored_drop_point_index': 1,
                'inbound_flight_seconds': 10, 'outbound_flight_seconds': 15,
                'knots': [{
                    'position_xyz': [index, 100, knot], 'tangent_xyz': [0, 0, 1],
                    'normalized_parameter': knot / 2,
                } for knot in range(3)],
            } for index in reversed(range(4))],
        }],
    }
    models = {
        'schema': 'mag_private_retail_domination_helo_models_v1',
        'retail_elf_sha256': 'invented-native',
        'game_archive_sha256': 'invented-game',
        'vehicle_registry_sha256': 'invented-registry',
        'variants': {
            'Raven': {
                'max_occupancy': 16, 'max_hitpoints': 100,
                'seat_groups': [
                    {'SeatType': 'TurretOperatorSeat'}, {'SeatType': 'TurretOperatorSeat'},
                    {'SeatType': 'PassengerSeat', 'Name': '', 'occupantIsVisible': 'false', 'SeatCount': '14'},
                ],
                'acceleration_seconds': 1.7, 'braking_seconds': 1.75,
                'flight_start_delay_max_seconds': 5, 'hover_delay_seconds': 0.5,
            },
        },
    }
    return spawns, links, paths, models


class HeloProjectionTests(unittest.TestCase):
    def test_joins_routes_to_native_slots_and_keeps_ordered_knots(self):
        output = json.loads(project(*map(encode, fixture())))
        carriers = output['carriers']
        self.assertEqual([carrier['object_index'] for carrier in carriers], list(range(4)))
        self.assertEqual([carrier['route']['knots'][0]['position_xyz'][0] for carrier in carriers],
                         list(range(4)))
        self.assertEqual(carriers[3]['squads'], [12, 13])
        self.assertEqual(carriers[3]['passenger_start'], 2)
        self.assertEqual(carriers[3]['health_quarter_points'], 400)

    def test_rejects_changed_hash_spline_order_and_unverified_seat_group(self):
        for index, change, message in (
            (1, lambda links: links.update(retail_elf_sha256='other-native'), 'Unbound Domination'),
            (2, lambda paths: paths['maps'][0]['paths'][0]['knots'].reverse(), 'spline order'),
            (3, lambda models: models['variants']['Raven']['seat_groups'].reverse(), 'passenger layout'),
        ):
            with self.subTest(index=index):
                input_documents = fixture()
                change(input_documents[index])
                with self.assertRaisesRegex(ValueError, message):
                    project(*map(encode, input_documents))


if __name__ == '__main__':
    unittest.main()
