#!/usr/bin/env python3
"""Project original seat visibility for the ground seats admitted by a profile."""
import argparse
import json
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent
COMMON = runpy.run_path(str(TOOLS / 'project-retail-apc-insertion.py'))
HELPERS = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
require, sha, encode, integer, property_integer = (
    COMMON[key] for key in ('require', 'sha', 'encode', 'integer', 'property_integer')
)
REGISTRY = HELPERS['VEHICLES_MEMBER']


def project(ground, archive_sha256, read_member):
    source = ground['source']
    require(ground['format'] == 'mag_ground_vehicles' and ground['schema_version'] == 1
            and source['native_elf_sha256'] == COMMON['SUPPORTED_NATIVE_ELF_SHA256']
            and source['creation_scope'] in ('native_vehicle_source_points',
                                            'native_domination_vehicle_source_points')
            and 'seat_visibility' not in ground, 'Require original qualified ground-vehicle inventory')
    registry_raw = read_member(REGISTRY)
    parse = HELPERS['parse_zpr']
    registry = HELPERS['vehicle_registry'](parse(registry_raw.decode()))
    vehicles = {v['index']: v for v in ground['vehicles']}
    require(len(vehicles) == len(ground['vehicles']), 'Duplicate ground vehicle index')
    driver_rows = ground.get('driver_seats', [])
    drivers = {d['vehicle_index']: d['seat'] for d in driver_rows}
    require(len(drivers) == len(driver_rows) and drivers.keys() <= vehicles.keys(),
            'Unbound or duplicate driver seat')

    rows = []
    for index, vehicle in sorted(vehicles.items()):
        passengers = vehicle.get('passenger_seats', [])
        seats = {integer(seat, 'passenger seat', 0, 16): 'PassengerSeat' for seat in passengers}
        require(len(seats) == len(passengers), 'Duplicate passenger seat')
        if index in drivers:
            seat = integer(drivers[index], 'driver seat', 0, 16)
            require(seat not in seats, 'Conflicting driver and passenger seat')
            seats[seat] = 'VehicleOperatorSeat'
        if not seats:
            continue

        vehicle_type = f"0x{integer(vehicle['vehicle_type_token'], 'vehicle type', 1, 2**32):08x}"
        faction = COMMON['FACTIONS'][integer(vehicle['faction'], 'vehicle faction', 0, 3)]
        modifier = integer(vehicle['modifier'], 'vehicle modifier', 0, 64)
        variants = [row for row in registry.get(vehicle_type, {}).get('variants', [])
                    if row['faction'] == faction and row['ModifierLevel'] == str(modifier)]
        require(len(variants) == 1, 'Require unique authored vehicle property variant')
        filename = variants[0]['PropertiesFileName']
        require(Path(filename).name == filename and filename.endswith('.zpr'), 'Invalid vehicle property member')
        raw = read_member('/run/mag/ps3/scripts/vehicle/' + filename)
        authored = parse(raw.decode())['GroundVehicleProperties']['Seats']['SeatPropertiesList']
        physical = []
        for ordinal, (key, record) in enumerate(authored.items()):
            require(key == f'SeatProperties{ordinal}', 'Unqualified authored physical seat order')
            seat = record['SeatProperties']
            count = property_integer(seat.get('SeatCount', '1'), 'SeatCount', 1, 17)
            require(len(physical) + count <= 16, 'Authored seat exceeds native inventory bound')
            physical.extend([seat] * count)
        projected = []
        for seat, role in sorted(seats.items()):
            require(seat < len(physical) and physical[seat]['SeatType'] == role,
                    'Admitted seat role differs from authored slot')
            visible = physical[seat].get('occupantIsVisible')
            require(visible in ('true', 'false'), 'Missing or invalid authored occupant visibility')
            projected.append(dict(seat=seat, occupant_visible=visible == 'true'))
        rows.append(dict(vehicle_index=index, properties_member_sha256=sha(raw), seats=projected))
    require(rows, 'No admitted ground seats have source visibility')
    return dict(ground, seat_visibility=dict(
        archive_sha256=archive_sha256, registry_member_sha256=sha(registry_raw), vehicles=rows))


def render(ground, game_raw):
    archive_sha256 = sha(game_raw)
    require(archive_sha256 == COMMON['GAME_ARCHIVE_SHA256'], 'Wrong original game.psarc')
    psarc = runpy.run_path(str(TOOLS / 'extract-retail-spawns.py'))['Psarc'](game_raw)

    def read_member(name):
        require(psarc.names.count(name) == 1, 'Missing or ambiguous original vehicle member ' + name)
        return psarc.extract(psarc.names.index(name) + 1)

    return project(ground, archive_sha256, read_member)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('ground-catalog', 'game-archive', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    output = args.output.resolve()
    try:
        require(not output.is_relative_to(TOOLS.parents[1])
                and output not in {args.ground_catalog.resolve(), args.game_archive.resolve()},
                'Output must be outside the source checkout and inputs')
        result = encode(render(json.loads(args.ground_catalog.read_bytes()), args.game_archive.read_bytes()))
        if args.check:
            require(output.read_bytes() == result, 'Ground seat visibility differs from reproduction')
        else:
            with output.open('xb') as out:
                out.write(result)
    except (KeyError, OSError, TypeError, ValueError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
