"""Synthetic source and wire identities for the retail APC turret projector."""
import importlib.util
from pathlib import Path
import unittest

from test_hospital_turret_projector import FACTIONS, encode, identity, node, sha, token, zpr


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-apc-turrets.py'


class Fixture:
    def __init__(self):
        spec = importlib.util.spec_from_file_location('synthetic_apc_turret_projector', TOOL)
        self.tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.tool)
        self.game = {}
        self.main = {}
        self.models = {}
        self.equipment = {}
        self.profile_pairs = []
        self.sources = {
            'format': 'mag_domination_ground_sources_v1',
            'allocation_scope': 'successful_production_fresh_level_load_type1',
            'native_elf_sha256': self.tool.RETAIL_ELF_SHA256,
            'original_game_sha256': self.tool.GAME_ARCHIVE_SHA256,
            'original_main_sha256': self.tool.MAIN_ARCHIVE_SHA256,
            'spawn_bindings_sha256': '1' * 64,
            'maps': [], 'variants': {},
        }
        registry_type = {'Type': 'APC'}
        for faction, (name, group) in enumerate(zip(FACTIONS, ('Green', 'Black', 'Red'), strict=True)):
            member = f'/run/mag/ps3/scripts/vehicle/test_apc_{faction}.zpr'
            model_path = f'ART/TestApc{faction}.dae'
            registry_type[f'Faction_{group}'] = {'Vehicle_Data': {'VehicleData0': {'VehicleData': {
                'Class': 'VehicleClass_Wheeled', 'ModifierLevel': '0',
                'PropertiesFileName': Path(member).name, 'ModelFileName': model_path,
            }}}}
            weapon = f'invented_primary_{faction}'
            self.equipment[weapon] = {'wire_id': 1101 + 700 * faction}
            properties = {'Seats': {'MaxOccupancy': '8', 'SeatPropertiesList': {
                'SeatProperties0': {'SeatProperties': {'SeatType': 'VehicleOperatorSeat'}},
                'SeatProperties1': {'SeatProperties': {'SeatType': 'TurretOperatorSeat',
                                                      'Name': 'turret_01', 'TurretIndex': '0',
                                                      'occupantIsVisible': 'false'}},
                'SeatProperties2': {'SeatProperties': {'SeatType': 'PassengerSeat', 'SeatCount': '8'}},
            }}, 'Turrets': {'TurretProps': {'TurretProperties0': {'TurretProperties': {
                'Weapon1': weapon, 'Weapon2': '',
            }}}}}
            self.game[member] = ('GroundVehicleProperties = ' + zpr(properties)).encode()
            body = f'/test_apc_{faction}.zrb'
            self.main[body] = f'invented model payload {faction}'.encode()
            transform = identity()
            transform.translation_xyz = (-0.5 + faction, 1.5, 0.75)
            self.models[self.main[body]] = {
                'name_token': token(model_path), 'source_sha256': sha(body.encode()),
                'nodes': [node(0, 65535, 'root'), node(1, 0, 'chassis'),
                          node(2, 1, 'turret_01'), node(3, 2, 'turret_yaw'),
                          node(4, 3, 'turret_pitch')],
                'reference_transforms': [identity(), identity(), transform, identity(), identity()],
            }
            self.sources['variants'][f'3:{name}'] = {
                'type': 3, 'faction': faction, 'source_member': member,
                'source_sha256': sha(self.game[member]), 'passenger_seats': list(range(2, 10)),
                'maximum_occupancy': 8, 'placement_y_offset': 0.5,
                'body': {'member': body, 'member_sha256': sha(self.main[body])},
            }
        registry_member = '/run/mag/ps3/scripts/vehicles.zpr'
        self.game[registry_member] = ('VehicleTable = ' + zpr(
            {'Vehicle_Types': {'VehicleTypeData0': {'VehicleTypeData': registry_type}}})).encode()
        self.sources['original_registry_sha256'] = sha(self.game[registry_member])
        for owner in range(3):
            self.add_profiles(owner)

    def add_profiles(self, owner):
        name = f'invented_apc_map_{owner}'
        original_level = {'archive_sha256': sha(name.encode()), 'clv_sha256': sha(name.upper().encode())}
        level = {'map': name, 'owner_faction': FACTIONS[owner], 'source': original_level, 'rows': []}
        self.sources['maps'].append(level)
        for index in range(16):
            apc = index < 8 if owner != 2 else index >= 8
            level['rows'].append({
                'object_index': index, 'spawn_point_index': index, 'spawn_point_token': 500 + index,
                'vehicle_type_token': token('APC' if apc else 'invented_motor_pool'),
                'spawn_type': 3 if apc else 7, 'team': int(not apc),
                'user_class': int(not apc), 'source_family': 35, 'authored_active': True,
                'squad_links': [{'platoon_id': (index % 8) // 4, 'squad': ('Alpha', 'Bravo', 'Charlie', 'Delta')[index % 4],
                                 'team': 0, 'slot': 0}] if apc else [],
                'source_transform': {'position_xyz': [index, 2., 0.], 'rotation_xyzw': [0., 0., 0., 1.]},
            })
        for attacker, faction in enumerate(FACTIONS):
            if attacker == owner:
                continue
            vehicles, bindings, passengers, drivers, visibility, points, squads = [], [], [], [], [], [], []
            for row in level['rows']:
                index = row['object_index']
                apc = row['spawn_type'] == 3
                spawn = {'spawn_point_index': index, 'spawn_point_token': row['spawn_point_token']}
                vehicle = dict(spawn, index=index, vehicle_type_token=row['vehicle_type_token'], modifier=0,
                               team=int(not apc), faction=attacker if apc else owner,
                               position={'x': index, 'y': 2.5, 'z': 0.}, rotation=[0., 0., 0., 1.])
                if apc:
                    vehicle.update(passenger_seats=list(range(2, 10)), maximum_occupancy=8)
                    passengers.append(dict(spawn, vehicle_index=index))
                    drivers.append({'vehicle_index': index, 'seat': 0})
                    visibility.append({'vehicle_index': index,
                                       'properties_member_sha256': self.sources['variants'][f'3:{faction}']['source_sha256'],
                                       'seats': [{'seat': seat, 'occupant_visible': False} for seat in [0, *range(2, 10)]]})
                    squads.append({'index': index % 8, 'team': 0, 'default_spawn': index, 'available': [index]})
                vehicles.append(vehicle)
                bindings.append(dict(spawn, vehicle_index=index, **{key: row[key] for key in (
                    'vehicle_type_token', 'spawn_type', 'team', 'user_class', 'source_family', 'authored_active')}))
                points.append({'index': index, 'token': row['spawn_point_token'], 'active': True,
                               'kind': 'vehicle_passenger' if apc else 'unsupported'})
            layout = {'name': name, 'owner_faction': owner, 'runtime_indices_verified': True,
                      'locations': points, 'squads': squads}
            spawns = {'format': 'mag_spawn_catalog', 'schema_version': 1, 'active': {'map_name': name},
                      'match_profile': {'game_type': 2, 'auto_assign_teams': True, 'has_bunkers': True,
                                        'attacking_faction': attacker}, 'maps': [layout]}
            ground = {'format': 'mag_ground_vehicles', 'schema_version': 1, 'vehicles': vehicles,
                      'vehicle_sources': bindings, 'passenger_spawns': passengers, 'driver_seats': drivers,
                      'seat_visibility': {'archive_sha256': self.tool.GAME_ARCHIVE_SHA256,
                                          'registry_member_sha256': self.sources['original_registry_sha256'],
                                          'vehicles': visibility}}
            self.profile_pairs.append((spawns, ground, original_level))

    def profiles(self, mutate=lambda _pairs: None, reverse=False):
        mutate(self.profile_pairs)
        source_hash = sha(encode(self.sources))
        pairs = []
        for spawns, ground, level in self.profile_pairs:
            spawns['source'] = encode({
                'map': spawns['active']['map_name'], 'game_archive_sha256': self.tool.GAME_ARCHIVE_SHA256,
                'native_spawn_bindings_sha256': self.sources['spawn_bindings_sha256'],
                'domination_ground_sources_sha256': source_hash,
                'level_archive_sha256': level['archive_sha256'], 'level_clv_sha256': level['clv_sha256'],
            }).decode()
            spawn_raw = encode(spawns)
            ground['source'] = {'creation_scope': self.tool.GROUND['SCHEMA'], 'spawn_catalog_sha256': sha(spawn_raw),
                                'native_elf_sha256': self.tool.RETAIL_ELF_SHA256, 'native_binding_sha256': source_hash}
            pairs.append((spawn_raw, encode(ground)))
        if reverse:
            pairs.reverse()
        return self.tool.project_profiles(self.sources, source_hash, pairs, token('APC'))

    def variants(self):
        def binding(_catalog, _ids, faction, weapon):
            return dict(self.equipment[weapon], name=weapon, faction=faction)

        return self.tool.project_variants(
            self.sources, {}, {}, self.game.__getitem__, self.main.__getitem__,
            lambda body, _token: self.models[body], binding,
        )


class ApcTurretProjectorTests(unittest.TestCase):
    def test_preserves_48_exact_ground_bindings_and_excludes_motor_pool(self):
        fixture = Fixture()
        profiles = fixture.profiles()
        self.assertEqual(len(profiles), 6)
        self.assertEqual(sum(len(profile['vehicles']) for profile in profiles), 48)
        self.assertEqual(list(range(8, 16)), [row['vehicle_index'] for row in profiles[-1]['vehicles']])
        self.assertEqual(profiles, fixture.profiles(reverse=True))

    def test_rejects_unbound_ground_pose_or_duplicate_profile(self):
        fixture = Fixture()

        def shift(pairs):
            pairs[0][1]['vehicles'][0]['position']['x'] += 1.

        with self.assertRaisesRegex(ValueError, 'spawn identity differs at allocation 0'):
            fixture.profiles(shift)
        fixture = Fixture()
        with self.assertRaisesRegex(ValueError, 'duplicate or invalid Domination APC profile'):
            fixture.profiles(lambda pairs: pairs.__setitem__(1, pairs[0]))

    def test_selects_original_named_seat_and_three_attested_primaries(self):
        fixture = Fixture()
        variants, proof, equipment, vehicle_type = fixture.variants()
        self.assertEqual(vehicle_type, token('APC'))
        self.assertEqual([row['primary_item_id'] for row in variants], [1101, 1801, 2501])
        self.assertEqual(variants[0], {
            'faction': 0, 'primary_item_id': 1101, 'seat_node': 'turret_01',
            'seat_index': 1, 'turret_index': 0, 'occupant_visible': False,
            'local_position_xyz': [-0.5, 1.5, 0.75],
        })
        self.assertEqual(proof[0]['seat_chain'][0]['node_id'], 2)
        self.assertEqual(proof[0]['aim_nodes'][1]['parent_node_id'], 3)
        self.assertEqual(equipment[0]['name'], 'invented_primary_0')
        self.assertEqual(fixture.variants(), (variants, proof, equipment, vehicle_type))

    def test_requires_a_unique_original_named_seat_and_identity_parent_chain(self):
        fixture = Fixture()
        first = next(iter(fixture.models.values()))
        first['nodes'][2].name_token = token('unresolved_seat')
        with self.assertRaisesRegex(ValueError, 'named APC turret seat'):
            fixture.variants()
        first['nodes'][2].name_token = token('turret_01')
        first['reference_transforms'][1].translation_xyz = (1., 0., 0.)
        with self.assertRaisesRegex(ValueError, 'APC seat parent'):
            fixture.variants()

    def test_rejects_open_secondary_or_wrong_original_property_digest(self):
        fixture = Fixture()
        variant = fixture.sources['variants']['3:Valor']
        member = variant['source_member']
        fixture.game[member] = fixture.game[member].replace(b'Weapon2=""', b'Weapon2="another"')
        with self.assertRaisesRegex(ValueError, 'APC property digest'):
            fixture.variants()
        variant['source_sha256'] = sha(fixture.game[member])
        with self.assertRaisesRegex(ValueError, 'absent secondary'):
            fixture.variants()


if __name__ == '__main__':
    unittest.main()
