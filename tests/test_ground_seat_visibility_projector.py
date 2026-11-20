"""Synthetic authored vehicle records, with no original resources or identifiers."""
import hashlib
from pathlib import Path
import runpy
import unittest
import zlib

TOOL = Path(__file__).resolve().parents[1] / 'tools/extraction/project-retail-ground-seat-visibility.py'
REGISTRY = '/run/mag/ps3/scripts/vehicles.zpr'
MEMBER = '/run/mag/ps3/scripts/vehicle/invented.zpr'
ARCHIVE = 'a' * 64
ELF = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
TOKEN = zlib.crc32(b'invented')
REGISTRY_RAW = b'''
VehicleTable = { Vehicle_Types = { Unit = { VehicleTypeData = {
    Type = "Invented", Faction_Green = { Vehicle_Data = { Variant = { VehicleData = {
        ModifierLevel = "0", PropertiesFileName = "invented.zpr"
    } } } }
} } } }
'''
VEHICLE_RAW = b'''
GroundVehicleProperties = { Seats = { SeatPropertiesList = {
    SeatProperties0 = { SeatProperties = { SeatType = "VehicleOperatorSeat", occupantIsVisible = "true" } },
    SeatProperties1 = { SeatProperties = { SeatType = "TurretOperatorSeat" } },
    SeatProperties2 = { SeatProperties = {
        SeatType = "PassengerSeat", SeatCount = "3", occupantIsVisible = "false"
    } }
} } }
'''


def catalog():
    return dict(format='mag_ground_vehicles', schema_version=1,
        source=dict(native_elf_sha256=ELF, creation_scope='native_vehicle_source_points'),
        vehicles=[dict(index=0, vehicle_type_token=TOKEN, faction=0, modifier=0, passenger_seats=[2, 4]),
                  dict(index=1, vehicle_type_token=TOKEN ^ 1, faction=0, modifier=0)],
        driver_seats=[dict(vehicle_index=0, seat=0)])


class GroundSeatVisibilityTests(unittest.TestCase):
    def api(self):
        return runpy.run_path(str(TOOL))

    def project(self, data, resource):
        members = {REGISTRY: REGISTRY_RAW, MEMBER: resource}
        return self.api()['project'](data, ARCHIVE, members.__getitem__)

    def test_expands_authored_roles_and_preserves_explicit_visibility_on_admitted_seats(self):
        result = self.project(catalog(), VEHICLE_RAW)
        source = result['seat_visibility']
        self.assertEqual(source['archive_sha256'], ARCHIVE)
        self.assertEqual(source['registry_member_sha256'], hashlib.sha256(REGISTRY_RAW).hexdigest())
        self.assertEqual(source['vehicles'], [
            dict(vehicle_index=0, properties_member_sha256=hashlib.sha256(VEHICLE_RAW).hexdigest(),
                 seats=[dict(seat=0, occupant_visible=True), dict(seat=2, occupant_visible=False),
                        dict(seat=4, occupant_visible=False)])
        ])

    def test_missing_visibility_and_runtime_seat_role_disagreement_fail_closed(self):
        changed = VEHICLE_RAW.replace(b', occupantIsVisible = "false"', b'')
        with self.assertRaisesRegex(ValueError, 'visibility'):
            self.project(catalog(), changed)
        data = catalog()
        data['vehicles'][0]['passenger_seats'] = [1, 2]
        with self.assertRaisesRegex(ValueError, 'seat role'):
            self.project(data, VEHICLE_RAW)

    def test_unknown_modified_or_wrong_game_archive_cannot_supply_visibility(self):
        data = catalog()
        data['vehicles'][0]['modifier'] = 1
        with self.assertRaisesRegex(ValueError, 'variant'):
            self.project(data, VEHICLE_RAW)
        data = catalog()
        data['vehicles'][0]['vehicle_type_token'] ^= 1
        with self.assertRaisesRegex(ValueError, 'variant'):
            self.project(data, VEHICLE_RAW)
        with self.assertRaisesRegex(ValueError, 'game.psarc'):
            self.api()['render'](catalog(), b'changed archive')


if __name__ == '__main__':
    unittest.main()
