"""Synthetic source identities for the retail Acquisition Escort projector."""
from copy import deepcopy
from pathlib import Path
import runpy
import unittest

from test_hospital_turret_projector import FACTIONS, encode, identity, node, sha, token, zpr


TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-escort-turrets.py'


class Fixture:
    def __init__(self):
        self.tool = runpy.run_path(str(TOOL))
        self.game, self.models = {}, {}
        self.items = {'items': {}, 'definitions': {'weapons': {}, 'hitscan_ammo': {'hmg_ammo': {}}}}
        self.ids = {}
        self.escort = token('Escort')
        self.owners = {f'escort_map_{faction}': faction for faction in range(3)}
        self.sources = {'initial_bindings_proof_sha256': '1' * 64, 'maps': []}
        self.pairs = []
        registry = {'Type': 'Escort'}
        self.properties = {}
        for faction, group in enumerate(('Green', 'Black', 'Red')):
            member = f'/run/mag/ps3/scripts/vehicle/test_escort_{faction}.zpr'
            model = f'ART/TestEscort{faction}.dae'
            registry[f'Faction_{group}'] = {'Vehicle_Data': {'VehicleData0': {'VehicleData': {
                'Class': 'VehicleClass_Wheeled', 'ModifierLevel': '0',
                'PropertiesFileName': Path(member).name, 'ModelFileName': model,
            }}}}
            weapon = f'invented_hmg_{faction}'
            ref = f'FactionItemList{faction + 1}:7'
            self.ids[ref] = (faction + 1) * 1000 + 7
            self.items['items'][ref] = {
                'name': weapon, 'authored_ref': ref, 'item_type': 'kWeapon',
                'definition': {'group': 'weapons', 'name': weapon},
            }
            self.items['definitions']['weapons'][weapon] = {'parameters': {
                'ammo_name': 'hmg_ammo', 'category': 'kWepCategoryVehicleWeapon',
                'initial_fire_mode': 'kAuto',
                'fire_mode': {'fire_modes': {'FireModeTypeClass0': {
                    'FireModeTypeClass': {'fire_mode_value': 'kAuto'},
                }}},
            }}
            seats = {'SeatPropertiesList': {
                'SeatProperties0': {'SeatProperties': {'SeatType': 'VehicleOperatorSeat',
                                                      'occupantIsVisible': 'false'}},
                'SeatProperties1': {'SeatProperties': {'SeatType': 'TurretOperatorSeat',
                                                      'Name': 'turret_01', 'TurretIndex': '0',
                                                      'occupantIsVisible': 'false'}},
                'SeatProperties2': {'SeatProperties': {'SeatType': 'PassengerSeat',
                                                      'SeatCount': '8', 'occupantIsVisible': 'false'}},
            }, 'MaxOccupancy': '8'}
            properties = {'Seats': seats, 'Placement_Y_Offset': '1.0',
                          'DamageProperties': {'MaxHitpoints': '200'},
                          'Turrets': {'TurretProps': {'TurretProperties0': {'TurretProperties': {
                              'Weapon1': weapon, 'Weapon2': '',
                          }}}}}
            self.game[member] = ('GroundVehicleProperties = ' + zpr(properties)).encode()
            self.properties[faction] = {
                'faction': FACTIONS[faction], 'properties_member': member,
                'properties_sha256': sha(self.game[member]), 'placement_y_offset': 1.,
                'health_quarter_points': 800,
            }
            seat = (9, 2, 3)[faction]
            yaw, pitch = seat + 1, seat + 2
            offsets = [identity() for _ in range(pitch + 1)]
            offsets[seat].translation_xyz = (0., 1.5, float(faction))
            self.models[token(model)] = (f'test_{faction}.zrb', sha(model.encode()), {
                'name_token': token(model), 'source_sha256': sha(member.encode()),
                'nodes': [node(0, 65535, 'root'), node(1, 0, 'chassis'),
                          node(seat, 1, 'turret_01'), node(yaw, seat, 'turret_yaw'),
                          node(pitch, yaw, 'turret_pitch')],
                'reference_transforms': offsets,
            })
        self.registry = '/run/mag/ps3/scripts/vehicles.zpr'
        self.game[self.registry] = ('VehicleTable = ' + zpr(
            {'Vehicle_Types': {'VehicleTypeData0': {'VehicleTypeData': registry}}})).encode()
        for name, owner in self.owners.items():
            self.add_profiles(name, owner)

    def add_profiles(self, name, owner):
        level = {'map': name, 'source': {'archive_sha256': sha(name.encode()),
                                         'clv_sha256': sha(name.upper().encode())}, 'descriptors': []}
        self.sources['maps'].append(level)
        for index in range(4, 16):
            escort = index < 8
            layout = [float(index), 10., 1.]
            variant_poses = [
                dict(row, initial_creation_position_xyz=[layout[0], 11., 1.],
                     respawn_position_xyz=[layout[0], 11., 1.])
                for row in self.properties.values()
            ]
            level['descriptors'].append({
                'owned_selected_vehicle_index': index, 'source_point_index': index + 16,
                'source_point_token': 900 + index, 'source_point_object_id': 0x23800000 + index + 16,
                'vehicle_type': self.escort if escort else token('MotorPool'),
                'spawn_type': 9 if escort else 7, 'team': int(not escort),
                'source_point_family': 35, 'authored_source_active': not escort,
                'initial_network_state': 0 if escort else 2, 'ground_snap': False,
                'radius_evidence': {'value': 0., 'raw_hex': '00000000'},
                'source_position_xyz': layout, 'rotation_xyzw_source_convention': [0., 0., 0., 1.],
                'variants': variant_poses,
            })
        for attacker in range(3):
            if attacker == owner:
                continue
            vehicles, bindings, drivers, passengers, visibility, points = [], [], [], [], [], []
            for index in range(16):
                escort = 4 <= index < 8
                source = level['descriptors'][index - 4] if index >= 4 else {
                    'source_point_index': index + 16, 'source_point_token': 900 + index,
                    'vehicle_type': token('APC'), 'team': 0, 'spawn_type': 3,
                    'source_position_xyz': [float(index), 10., 1.],
                    'authored_source_active': True, 'initial_network_state': 2,
                    'rotation_xyzw_source_convention': [0., 0., 0., 1.],
                }
                spawn = {'spawn_point_index': source['source_point_index'],
                         'spawn_point_token': source['source_point_token']}
                faction = [attacker, owner][source['team']]
                vehicles.append(dict(spawn, index=index, vehicle_type_token=source['vehicle_type'],
                                     faction=faction, team=source['team'], modifier=0,
                                     initial_state=source['initial_network_state'],
                                     position={'x': float(index), 'y': 11., 'z': 1.},
                                     rotation=source['rotation_xyzw_source_convention'],
                                     health_quarter_points=800))
                points.append({'index': source['source_point_index'], 'token': source['source_point_token'],
                               'kind': 'vehicle_passenger' if index < 4 else 'unsupported',
                               'active': source['authored_source_active'],
                               'position': dict(zip('xyz', source['source_position_xyz']))})
                bindings.append(dict(spawn, vehicle_index=index, vehicle_type_token=source['vehicle_type'],
                                     spawn_type=source['spawn_type'], team=source['team'],
                                     authored_active=source['authored_source_active'],
                                     source_family=35, user_class=int(index >= 4)))
                if index < 4:
                    passengers.append(dict(spawn, vehicle_index=index))
                if escort:
                    drivers.append({'vehicle_index': index, 'seat': 0})
                    vehicles[-1].update(passenger_seats=list(range(2, 10)), maximum_occupancy=8)
                if index < 8:
                    seats = ([0] if escort else []) + list(range(2, 10))
                    visibility.append({'vehicle_index': index,
                                       'properties_member_sha256': self.properties[faction]['properties_sha256'],
                                       'seats': [{'seat': seat, 'occupant_visible': False} for seat in seats]})
            spawns = {'format': 'mag_spawn_catalog', 'schema_version': 1, 'active': {'map_name': name},
                      'match_profile': {'game_type': 1, 'has_bunkers': False, 'has_objectives': True,
                                        'auto_assign_teams': True, 'attacking_faction': attacker},
                      'maps': [{'name': name, 'owner_faction': owner, 'runtime_indices_verified': True,
                                'locations': points}],
                      'source': encode({'map': name, 'game_archive_sha256': self.tool['GAME_ARCHIVE_SHA256'],
                                        'native_binding_sha256': self.sources['initial_bindings_proof_sha256'],
                                        'native_elf_sha256': self.tool['RETAIL_ELF_SHA256'],
                                        'level_archive_sha256': level['source']['archive_sha256'],
                                        'level_clv_sha256': level['source']['clv_sha256']}).decode()}
            ground = {'format': 'mag_ground_vehicles', 'schema_version': 1, 'vehicles': vehicles,
                      'vehicle_sources': bindings, 'driver_seats': drivers, 'passenger_spawns': passengers,
                      'seat_visibility': {'archive_sha256': self.tool['GAME_ARCHIVE_SHA256'],
                                          'registry_member_sha256': sha(self.game[self.registry]),
                                          'vehicles': visibility}}
            self.pairs.append((spawns, ground))

    def profiles(self, update=None, reverse=False):
        pairs = deepcopy(self.pairs)
        if update:
            update(pairs)
        final_pairs = []
        for spawns, ground in pairs:
            spawn_raw = encode(spawns)
            ground['source'] = {'creation_scope': 'native_vehicle_source_points',
                                'spawn_catalog_sha256': sha(spawn_raw), 'native_binding_sha256': '2' * 64,
                                'native_elf_sha256': self.tool['RETAIL_ELF_SHA256']}
            final_pairs.append((spawn_raw, encode(ground)))
        if reverse:
            final_pairs.reverse()
        return self.tool['project_profiles'](
            self.sources, final_pairs, self.owners, sha(self.game[self.registry]), '2' * 64)

    def variants(self):
        return self.tool['project_variants'](
            self.sources, self.items, self.ids, self.game.__getitem__,
            lambda tokens: {key: self.models[key] for key in tokens})


class EscortTurretProjectorTests(unittest.TestCase):
    def test_binds_only_the_twenty_four_source_type9_placements(self):
        fixture = Fixture()
        profiles = fixture.profiles()
        self.assertEqual(len(profiles), 6)
        self.assertEqual(profiles, fixture.profiles(reverse=True))
        for row in profiles:
            self.assertEqual([vehicle['vehicle_index'] for vehicle in row['vehicles']], [4, 5, 6, 7])

    def test_rejects_wrong_mode_type_pose_and_duplicate_pair(self):
        fixture = Fixture()
        for update, message in (
            (lambda pairs: pairs[0][0]['match_profile'].__setitem__('game_type', 2), 'spawn profile'),
            (lambda pairs: pairs[0][1]['vehicle_sources'][4].__setitem__('spawn_type', 7), 'source identity'),
            (lambda pairs: pairs[0][1]['vehicles'][4]['position'].__setitem__('x', 0.), 'source pose'),
            (lambda pairs: pairs.__setitem__(0, pairs[1]), 'duplicate or invalid'),
        ):
            with self.subTest(message=message), self.assertRaisesRegex(ValueError, message):
                fixture.profiles(update)

    def test_resolves_each_named_seat_and_attested_primary(self):
        variants, poses, equipment, _ = Fixture().variants()
        self.assertEqual([pose['seat_chain'][0]['node_id'] for pose in poses], [9, 2, 3])
        self.assertEqual([row['local_position_xyz'] for row in variants],
                         [[0., 1.5, 0.], [0., 1.5, 1.], [0., 1.5, 2.]])
        self.assertEqual([row['primary_item_id'] for row in variants], [1007, 2007, 3007])
        self.assertEqual([row['definition_slots'] for row in equipment],
                         [[1007, None, None], [2007, None, None], [3007, None, None]])

    def test_rejects_wrong_pose_and_item_mapping(self):
        fixture = Fixture()
        model = next(iter(fixture.models.values()))[2]
        model['reference_transforms'][1].translation_xyz = (1., 0., 0.)
        with self.assertRaisesRegex(ValueError, 'seat parent'):
            fixture.variants()
        model['reference_transforms'][1] = identity()
        fixture.ids['FactionItemList1:7'] = 0
        with self.assertRaisesRegex(ValueError, 'Escort item identity'):
            fixture.variants()


if __name__ == '__main__':
    unittest.main()
