"""Invented hospital definitions, body nodes and spawn/placement bindings."""
import hashlib
import importlib.util
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
import zlib


ROOT = Path(__file__).resolve().parents[1]
TOOL = ROOT / 'tools/extraction/project-retail-hospital-turrets.py'
FACTIONS = ('Valor', 'Raven', 'SVER')
ELF = '1' * 64
GAME = '2' * 64
MAIN = '3' * 64
SPAWN_BINDING = '4' * 64
ALLOCATION = '5' * 64
PASSENGER = '6' * 64


def sha(data):
    return hashlib.sha256(data).hexdigest()


def encode(value):
    return json.dumps(value, allow_nan=False).encode()


def zpr(value):
    if isinstance(value, str):
        return json.dumps(value)
    return '{' + ','.join(f'{key}={zpr(child)}' for key, child in value.items()) + '}'


def token(value):
    return zlib.crc32(value.lower().encode('ascii'))


def node(node_id, parent_id, name):
    return SimpleNamespace(node_id=node_id, transform_index=node_id,
                           parent_node_id=parent_id, name_token=token(name))


def identity():
    return SimpleNamespace(translation_xyz=(0., 0., 0.), quaternion_xyzw=(0., 0., 0., 1.),
                           uniform_scale=1.)


class Fixture:
    def __init__(self):
        spec = importlib.util.spec_from_file_location('synthetic_hospital_turrets', TOOL)
        self.tool = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(self.tool)
        self.tool.RETAIL_ELF_SHA256 = ELF
        self.tool.GAME_ARCHIVE_SHA256 = GAME
        self.tool.MAIN_ARCHIVE_SHA256 = MAIN
        self.native = {'retail_elf_sha256': ELF, 'pose': {'seat_trace': '8' * 64},
                       'wire': {'equipment_trace': '9' * 64}}
        self.game = {}
        self.main = {}
        self.model = {}
        self.profile_data = []
        self.structures = {'schema_version': 1, 'application_id': 20231,
                           'source_archive_sha256': GAME, 'source_sha256': {},
                           'native_rules': {'original_elf_sha256': ELF}, 'profiles': []}
        self.sources = {'format': 'mag_retail_domination_hospital_placer_inputs',
                        'schema_version': 1, 'retail_elf_sha256': ELF,
                        'retail_game_sha256': GAME, 'retail_main_sha256': MAIN,
                        'spawn_binding_sha256': SPAWN_BINDING, 'faction_sources': {}, 'maps': []}
        self.catalog = {'schema_version': 2, 'application_id': 20231,
                        'build': {'source_archive_sha256': GAME}, 'sources': {},
                        'items': {}, 'definitions': {'weapons': {}, 'hitscan_ammo': {}, 'projectile_ammo': {}}}
        self.bindings = {'validated': True, 'positions_compared': 6, 'mismatch_count': 0, 'mismatches': [],
                         'binding_configuration': {'application_id': 20231, 'original_retail_elf_sha256': ELF,
                                                   'by_authored_ref': {}}}
        registry_type = {'Type': 'DefensiveEmplacement'}
        for faction, (name, group) in enumerate(zip(FACTIONS, ('Green', 'Black', 'Red'), strict=True)):
            filename = f'test_property_{faction}.zpr'
            script = f'vehicle/{filename}'
            member = '/run/mag/ps3/scripts/' + script
            body = f'/test_body_{faction}.zrb'
            model_path = f'ART/TestModel{faction}.dae'
            weapon_names = [f'test_primary_{faction}', f'test_secondary_{faction}']
            variant = {'Class': 'VehicleClass_DefensiveEmplacement', 'ModifierLevel': '0',
                       'PropertiesFileName': filename, 'ModelFileName': model_path}
            field = f'Faction_{group}'
            registry_type[field] = {'Vehicle_Data': {'VehicleData0': {'VehicleData': variant}}}
            property_path = ('VehicleTable.Vehicle_Types.VehicleTypeData0.VehicleTypeData.'
                             f'{field}.Vehicle_Data.VehicleData0.VehicleData')
            authored_seat = f'test_missing_seat_{faction}'
            seats = {'SeatPropertiesList': {'SeatProperties0': {'SeatProperties': {
                'Name': authored_seat, 'SeatType': 'TurretOperatorSeat',
                'occupantIsVisible': 'false', 'TurretIndex': '0'}}}}
            turret = {'TurretProps': {'TurretProperties0': {'TurretProperties': {
                'Weapon1': weapon_names[0], 'Weapon2': weapon_names[1]}}}}
            self.game[member] = ('DefensiveEmplacementProperties = ' + zpr({'Seats': seats, 'Turrets': turret})).encode()
            self.main[body] = f'invented body member {faction}'.encode()
            self.model[self.main[body]] = {
                'name_token': token(model_path),
                'source_sha256': sha(f'invented model {faction}'.encode()),
                'nodes': [node(0, 65535, 'gunner_seat'), node(1, 0, 'turret_yaw'), node(2, 1, 'turret_pitch')],
                'reference_transforms': [identity(), identity(), identity()],
            }
            self.sources['faction_sources'][name] = {
                'source_member': member, 'sha256': sha(self.game[member]),
                'seats': seats, 'variant': dict(variant, faction=name, property_path=property_path),
                'neutral_body': {'member': body, 'member_sha256': sha(self.main[body])},
            }
            self.structures['source_sha256'][script] = sha(self.game[member])
            self.structures['profiles'].append({'faction': faction, 'vehicle_type': 'DefensiveEmplacement',
                                              'properties_source': script, 'model': model_path})
            for gear, weapon in enumerate(weapon_names):
                ref = f'FactionItemList{faction + 1}:{gear + 70}'
                self.catalog['items'][ref] = {'authored_ref': ref, 'name': weapon, 'item_type': 'kWeapon',
                                              'definition': {'group': 'weapons', 'name': weapon}}
                self.bindings['binding_configuration']['by_authored_ref'][ref] = 1301 + faction * 800 + gear * 17
                ammo = f'{weapon}_ammo'
                ammo_group = ('hitscan_ammo', 'projectile_ammo')[gear]
                self.catalog['definitions'][ammo_group][ammo] = {'parameters': {'damage': {'character': 12}}}
                self.catalog['definitions']['weapons'][weapon] = {'parameters': {
                    'category': ('kWepCategoryTurretGun', 'kWepCategoryTurretRocket')[gear],
                    'ammo_name': ammo, 'initial_fire_mode': 'kAuto', 'magazine_capacity': 20,
                    'fire_mode': {'fire_modes': {'FireModeTypeClass0': {'FireModeTypeClass': {'fire_mode_value': 'kAuto'}}}},
                }}
            self.add_map(faction, script)
        registry = ('VehicleTable = ' + zpr({'Vehicle_Types': {'VehicleTypeData0': {'VehicleTypeData': registry_type}}})).encode()
        self.game['/run/mag/ps3/scripts/vehicles.zpr'] = registry
        self.sources['vehicle_registry_sha256'] = sha(registry)
        self.structures['source_sha256']['vehicles.zpr'] = sha(registry)
        for filename in ('item_list.zpr', 'weapons.lua', 'hitscanammo.lua', 'projectileammo.lua'):
            raw = f'invented {filename}'.encode()
            self.game['/run/mag/ps3/scripts/' + filename] = raw
            self.catalog['sources'][filename] = {'sha256': sha(raw)}

    def add_map(self, owner, property_source):
        name = f'test_map_faction_{owner}'
        level = {'archive_sha256': sha(name.encode()), 'clv_sha256': sha(name.upper().encode())}
        locations, squads, source_rows, placements = [], [], [], []
        for allocation in range(16):
            index = 32 + allocation
            spawn_token = 50000 + owner * 100 + allocation
            platoon = allocation // 4
            squad = ('Alpha', 'Bravo', 'Charlie', 'Delta')[allocation % 4]
            link = {'platoon_id': platoon, 'squad': squad, 'slot': 2, 'team': 1}
            source_rows.append({'spawn_index': index, 'spawn_token': hex(spawn_token), 'team': 1,
                                'squad_link': link, 'vehicle_type_token': hex(token('DefensiveEmplacement')),
                                'candidate_type13_index_if_all_initial_creations_succeed': allocation})
            locations.append({'index': index, 'token': spawn_token, 'kind': 'bunker_passenger', 'active': True})
            squads.append({'index': 16 + allocation, 'team': 1, 'faction': owner, 'platoon': platoon,
                           'squad': allocation % 4, 'available': [0, 1, index]})
            placements.append({'object_index': allocation, 'spawn_index': index,
                               'spawn_token': spawn_token, 'properties_source': property_source})
        self.sources['maps'].append({'map': name, 'owner_faction_id': owner, 'owner_faction': FACTIONS[owner],
                                    'level_source': level, 'hospitals': source_rows})
        pool = {'name': name, 'owner_faction': owner, 'runtime_indices_verified': True,
                'locations': locations, 'squads': squads}
        for attacker in range(3):
            if attacker == owner:
                continue
            provenance = {'map': name, 'game_archive_sha256': GAME,
                          'level_archive_sha256': level['archive_sha256'], 'level_clv_sha256': level['clv_sha256'],
                          'native_spawn_bindings_sha256': SPAWN_BINDING, 'hospital_allocation_sha256': ALLOCATION}
            spawns = {'format': 'mag_spawn_catalog', 'schema_version': 1, 'source': json.dumps(provenance),
                      'active': {'map_name': name},
                      'match_profile': {'game_type': 2, 'attacking_faction': attacker,
                                        'auto_assign_teams': True, 'has_bunkers': True},
                      'maps': [json.loads(json.dumps(pool))]}
            hospitals = {'format': 'mag_domination_hospital_placements_v1', 'map_name': name,
                         'allocation_scope': 'successful_production_fresh_level_load',
                         'source_proof_sha256': ALLOCATION, 'passenger_pose_sha256': PASSENGER,
                         'passenger_pose_scope': 'retail_unresolved_passenger_node_zero_local_pose',
                         'vehicle_type_token': token('DefensiveEmplacement'),
                         'placements': json.loads(json.dumps(placements))}
            self.profile_data.append((spawns, hospitals))

    def read_model(self, raw, model_token):
        model = self.model[raw]
        if model['name_token'] != model_token:
            raise AssertionError('synthetic provider received wrong original-model token')
        return model

    def project(self, *, profile_data=None, bind_catalog=True):
        catalog = encode(self.catalog)
        structures = dict(self.structures, item_catalog_sha256=sha(catalog))
        if bind_catalog:
            self.bindings['catalog_sha256'] = sha(catalog)
            self.bindings['binding_configuration']['source_catalog_sha256'] = sha(catalog)
        report = encode(self.bindings)
        self.tool.BINDING_REPORT_SHA256 = sha(report)
        self.tool.BINDING_POSITIONS = 6
        structures['retail_item_binding_sha256'] = sha(report)
        structure_raw = encode(structures)
        pairs = []
        for spawns, hospitals in self.profile_data if profile_data is None else profile_data:
            spawn_raw = encode(spawns)
            pairs.append((spawn_raw, encode(dict(hospitals, spawn_catalog_sha256=sha(spawn_raw),
                                                structure_catalog_sha256=sha(structure_raw)))))
        return self.tool.project(encode(self.sources), catalog, report, structure_raw, pairs,
                                 self.game.__getitem__, self.main.__getitem__, self.read_model, self.native)


class HospitalTurretProjectorTests(unittest.TestCase):
    def test_projects_source_bound_rules_and_deterministic_proof_for_six_profiles(self):
        fixture = Fixture()
        # Two different native vehicle types may share an authored property asset.
        # Resolve the hospital by its type as well as its faction and asset.
        fixture.structures['profiles'].append(dict(fixture.structures['profiles'][0],
                                                   vehicle_type='DefensiveEmplacementNoTurret'))
        rules, proof = fixture.project()
        self.assertEqual(rules['schema'], 'mag_domination_hospital_turrets_v1')
        self.assertEqual(rules['source']['vehicle_type_token'], token('DefensiveEmplacement'))
        self.assertEqual(rules['variants'][0], {
            'faction': 0, 'primary_item_id': 1301, 'secondary_item_id': 1318,
            'properties_source': 'vehicle/test_property_0.zpr', 'seat_node': 'gunner_seat',
            'seat_index': 0, 'turret_index': 0, 'seat_pose_scope': 'retail_root0_identity_fallback',
            'occupant_visible': False,
        })
        self.assertEqual(len(rules['profiles']), 6)
        self.assertTrue(all(len(row['placements']) == 16 for row in rules['profiles']))
        seen = {item['map_name'] for item in rules['profiles']}
        self.assertEqual(seen, {'test_map_faction_0', 'test_map_faction_1', 'test_map_faction_2'})
        row = next(row for row in rules['profiles'] if row['map_name'] == 'test_map_faction_1')
        self.assertEqual(row['placements'][15], {
            'faction': 1, 'object_index': 15, 'spawn_index': 47, 'spawn_token': 50115,
        })
        self.assertEqual(proof['pose']['models'][0]['fallback_node']['node_id'], 0)
        self.assertEqual(proof['pose']['models'][0]['authored_seat_name'], 'test_missing_seat_0')
        self.assertEqual(proof['wire']['family'], 13)
        self.assertEqual(fixture.project(profile_data=reversed(fixture.profile_data)), (rules, proof))

    def test_rejects_authored_pose_lookup_and_displaced_fallback(self):
        for edit, error in [
            (lambda model: model['nodes'].append(node(3, 0, 'test_missing_seat_0')), 'authored seat already resolves'),
            (lambda model: setattr(model['reference_transforms'][0], 'uniform_scale', 2.0), 'identity root'),
        ]:
            fixture = Fixture()
            edit(fixture.model[fixture.main['/test_body_0.zrb']])
            with self.subTest(error=error), self.assertRaisesRegex(ValueError, error):
                fixture.project()

    def test_binds_registry_property_and_body_to_supported_sources(self):
        for member, table, error in [
            ('/run/mag/ps3/scripts/vehicles.zpr', 'game', 'vehicle registry'),
            ('/run/mag/ps3/scripts/vehicle/test_property_0.zpr', 'game', 'property member'),
            ('/test_body_0.zrb', 'main', 'body member'),
        ]:
            fixture = Fixture()
            getattr(fixture, table)[member] += b' '
            with self.subTest(member=member), self.assertRaisesRegex(ValueError, error):
                fixture.project()

    def test_rejects_item_rebinding_and_wrong_catalog_identity(self):
        fixture = Fixture()
        fixture.bindings['binding_configuration']['by_authored_ref']['FactionItemList1:70'] = 2101
        with self.assertRaisesRegex(ValueError, 'item binding'):
            fixture.project()
        fixture = Fixture()
        fixture.bindings['catalog_sha256'] = 'a' * 64
        with self.assertRaisesRegex(ValueError, 'item catalog'):
            fixture.project(bind_catalog=False)

    def test_rejects_changed_hospital_and_duplicate_attack_profile(self):
        fixture = Fixture()
        fixture.profile_data[0][1]['placements'][0]['spawn_token'] += 1
        with self.assertRaisesRegex(ValueError, 'placement identity'):
            fixture.project()
        fixture = Fixture()
        fixture.profile_data[1][0]['match_profile']['attacking_faction'] = 1
        with self.assertRaisesRegex(ValueError, 'profile coverage'):
            fixture.project()

    def test_writes_outside_checkout_once_and_checks_exact_bytes(self):
        fixture = Fixture()
        with tempfile.TemporaryDirectory() as directory:
            outside = Path(directory)
            source = outside / 'input.json'
            source.write_bytes(b'original input')
            rules, proof = (outside / 'rules.json', outside / 'proof.json')
            fixture.tool.write_outputs((source,), ((rules, b'rules'), (proof, b'proof')), check=False)
            fixture.tool.write_outputs((source,), ((rules, b'rules'), (proof, b'proof')), check=True)
            self.assertEqual(rules.read_bytes(), b'rules')
            self.assertEqual(proof.read_bytes(), b'proof')
            with self.assertRaisesRegex(ValueError, 'differs'):
                fixture.tool.write_outputs((source,), ((rules, b'changed'),), check=True)
            with self.assertRaisesRegex(ValueError, 'already exists'):
                fixture.tool.write_outputs((source,), ((rules, b'changed'),), check=False)
            with self.assertRaisesRegex(ValueError, 'outside'):
                fixture.tool.write_outputs((source,), ((ROOT / 'synthetic-local-forbidden.json', b'rules'),), check=False)
            with self.assertRaisesRegex(ValueError, 'distinct'):
                fixture.tool.write_outputs((source,), ((source, b'rules'),), check=False)
            self.assertEqual(source.read_bytes(), b'original input')


if __name__ == '__main__':
    unittest.main()
