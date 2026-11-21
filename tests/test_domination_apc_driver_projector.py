"""Invented original-like archive members for the optional APC driver projection."""
import copy
import hashlib
import json
from pathlib import Path
import runpy
import unittest
import zlib

TOOLS = Path(__file__).resolve().parents[1] / 'tools/extraction'
API = runpy.run_path(str(TOOLS / 'project-retail-domination-ground.py'))
REGISTRY = '/run/mag/ps3/scripts/vehicles.zpr'
MEMBER = '/run/mag/ps3/scripts/vehicle/invented-driver.zpr'
TOKEN = zlib.crc32(b'invented driver')
ELF = API['APC']['SUPPORTED_NATIVE_ELF_SHA256']
ARCHIVE = API['APC']['GAME_ARCHIVE_SHA256']
REGISTRY_RAW = b'''VehicleTable = { Vehicle_Types = { Unit = { VehicleTypeData = {
 Type = "Invented Driver", Faction_Green = { Vehicle_Data = { Variant = { VehicleData = {
 ModifierLevel = "0", Class = "VehicleClass_Wheeled", PropertiesFileName = "invented-driver.zpr"
 } } } } } } } }'''
PROPERTY_RAW = b'''GroundVehicleProperties = {
 Placement_Y_Offset = "1", DamageProperties = { MaxHitpoints = "10" },
 Seats = { MaxOccupancy = "8", SeatPropertiesList = {
 SeatProperties0 = { SeatProperties = { SeatType = "VehicleOperatorSeat", occupantIsVisible = "false" } },
 SeatProperties1 = { SeatProperties = { SeatType = "TurretOperatorSeat", occupantIsVisible = "false" } },
 SeatProperties2 = { SeatProperties = { SeatType = "PassengerSeat", SeatCount = "8", occupantIsVisible = "false" } }
 } } }'''


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def fixture(apcs_last=False, registry=REGISTRY_RAW, properties=PROPERTY_RAW):
    source = dict(native_elf_sha256=ELF, original_game_sha256=ARCHIVE,
        original_registry_sha256=sha(registry), variants={'3:Valor':dict(
            faction=0, type=3, source_member=MEMBER, source_sha256=sha(properties),
            passenger_seats=list(range(2, 10)), maximum_occupancy=8,
            health_quarter_points=40, placement_y_offset=1.)})
    rows, bindings = [], []
    for index in range(16):
        apc = (index >= 8) == apcs_last
        rows.append(dict(index=index, vehicle_type_token=TOKEN if apc else TOKEN ^ 1,
            faction=0 if apc else 1, modifier=0, health_quarter_points=40,
            **(dict(passenger_seats=list(range(2, 10)), maximum_occupancy=8,
                    passenger_exit_candidates=[dict(ranking_offset=[1., 0., 0.],
                        position_offset=[1., 0., 0.])]) if apc else {})))
        bindings.append(dict(vehicle_index=index, spawn_type=3 if apc else 7))
    raw = json.dumps(source).encode()
    ground = dict(format='mag_ground_vehicles', schema_version=1,
        source=dict(native_elf_sha256=ELF, native_binding_sha256=sha(raw),
                    creation_scope='native_domination_vehicle_source_points'),
        vehicles=rows, vehicle_sources=bindings)
    return ground, source, {REGISTRY:registry, MEMBER:properties}


def apply(ground, source, members, archive=ARCHIVE):
    raw = json.dumps(source).encode()
    ground = copy.deepcopy(ground)
    ground['source']['native_binding_sha256'] = sha(raw)
    return API['apply_apc_driver_policy'](ground, raw, archive, members.__getitem__)


class DominationApcDriverProjectionTests(unittest.TestCase):
    def test_source_kind_selects_eight_apcs_in_either_allocation_range(self):
        for last in (False, True):
            with self.subTest(apcs_last=last):
                ground, source, members = fixture(last)
                before = copy.deepcopy(ground)
                result = apply(ground, source, members)
                expected = list(range(8, 16) if last else range(8))
                self.assertEqual(result['driver_seats'], [dict(vehicle_index=index,
                    seat=0, maximum_speed_mps=25.) for index in expected])
                self.assertEqual([row['vehicle_index'] for row in
                                  result['seat_visibility']['vehicles']], expected)
                for row in result['seat_visibility']['vehicles']:
                    self.assertEqual([seat['seat'] for seat in row['seats']], [0, *range(2, 10)])
                    self.assertTrue(all(not seat['occupant_visible'] for seat in row['seats']))
                for row, old in zip(result['vehicles'], before['vehicles']):
                    if row['index'] in expected:
                        self.assertEqual(row, old | dict(maximum_mount_distance_m=8.,
                            maximum_mount_position_age_ms=750))
                    else:
                        self.assertEqual(row, old)
                self.assertEqual(ground, before)

    def test_wrong_archive_registry_class_or_property_binding_rejects(self):
        ground, source, members = fixture()
        with self.assertRaisesRegex(ValueError, 'archive'):
            apply(ground, source, members, '0' * 64)
        for key in ('original_registry_sha256',):
            bad = copy.deepcopy(source)
            bad[key] = '0' * 64
            with self.assertRaises(ValueError):
                apply(ground, bad, members)
        for key, value in [('source_sha256', '0' * 64), ('faction', 1),
                           ('source_member', '/foreign.zpr')]:
            bad = copy.deepcopy(source)
            bad['variants']['3:Valor'][key] = value
            with self.assertRaises(ValueError):
                apply(ground, bad, members)
        registry = REGISTRY_RAW.replace(b'VehicleClass_Wheeled', b'VehicleClass_Other')
        ground, source, members = fixture(registry=registry)
        with self.assertRaisesRegex(ValueError, 'class'):
            apply(ground, source, members)

    def test_authored_roles_occupancy_and_inventory_are_required(self):
        for properties in (PROPERTY_RAW.replace(b'VehicleOperatorSeat', b'TurretOperatorSeat'),
                           PROPERTY_RAW.replace(b'MaxOccupancy = "8"', b'MaxOccupancy = "7"'),
                           PROPERTY_RAW.replace(b'occupantIsVisible = "false"', b'unknown = "false"')):
            ground, source, members = fixture(properties=properties)
            with self.assertRaises(ValueError):
                apply(ground, source, members)
        ground, source, members = fixture()
        for change in ('partial', 'duplicate', 'gunner'):
            bad = copy.deepcopy(ground)
            if change == 'partial':
                bad['vehicle_sources'][0]['spawn_type'] = 7
            elif change == 'duplicate':
                bad['vehicle_sources'][1]['vehicle_index'] = 0
            else:
                bad['vehicles'][0]['passenger_seats'] = [1, *range(2, 9)]
            with self.subTest(change=change), self.assertRaises(ValueError):
                apply(bad, source, members)

    def test_opt_in_requires_archive_and_archive_requires_opt_in(self):
        for kwargs in ({'enable_apc_driver': True}, {'game_raw': b'invented'}):
            with self.assertRaisesRegex(ValueError, 'opt-in'):
                API['project'](b'{}', b'{}', b'{}', **kwargs)


if __name__ == '__main__':
    unittest.main()
