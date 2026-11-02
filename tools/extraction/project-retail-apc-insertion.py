#!/usr/bin/env python3
"""Project qualified original Acquisition APC bindings into explicit server inputs.

Original resources stay external. Initial-load proof is source evidence, not an
observation of live occupancy or live-match compatibility. Exit qualification is
an explicit trusted external assertion, not inferred from a hash or coordinates.
"""
import argparse
import copy
import hashlib
import json
import math
from pathlib import Path
import re
import runpy
import struct

TOOLS = Path(__file__).resolve().parent
GAME_ARCHIVE_SHA256 = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
SUPPORTED_NATIVE_ELF_SHA256 = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
FACTIONS = ('Valor', 'Raven', 'SVER')
SQUADS = {'Alpha': 0, 'Bravo': 1, 'Charlie': 2, 'Delta': 3}
BASIS = 'Verified source initial/reset compiled load before dynamic creation; no live occupancy claim'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    """The exact bytes used by the Rust loader's spawn SHA binding."""
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def require(condition, message):
    if not condition:
        raise ValueError(message)


def digest(value, label):
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None,
            'Invalid SHA256: ' + label)
    return value


def integer(value, label, lower, upper):
    require(type(value) is int and lower <= value < upper, 'Invalid integer: ' + label)
    return value


def number(value, label):
    require(not isinstance(value, bool) and isinstance(value, (str, int, float)), 'Invalid number: ' + label)
    result = float(value)
    require(math.isfinite(result), 'Nonfinite number: ' + label)
    return result


def f32(value):
    try:
        result = struct.unpack('<f', struct.pack('<f', value))[0]
    except (OverflowError, struct.error) as error:
        raise ValueError('Coordinate outside finite f32') from error
    require(math.isfinite(result), 'Nonfinite f32')
    return result


def vector(value, length, label):
    require(isinstance(value, (list, tuple)) and len(value) == length, 'Invalid vector: ' + label)
    return [number(v, label) for v in value]


def quaternion(value):
    result = vector(value, 4, 'rotation_xyzw')
    require(abs(sum(v*v for v in result)-1.) <= .001, 'Nonunit quaternion')
    # The target loader reads f32. Also check the actual encoded quaternion.
    require(abs(sum(f32(v)**2 for v in result)-1.) <= .001, 'Nonunit f32 quaternion')
    return result


def token(value):
    if isinstance(value, str):
        require(re.fullmatch('0x[0-9a-fA-F]{1,8}', value) is not None, 'Invalid hexadecimal token')
        value = int(value, 16)
    return integer(value, 'token', 1, 2**32)


def position(value):
    xyz = [f32(v) for v in vector(value, 3, 'position')]
    require(all(abs(v) < 2048 for v in xyz) and xyz[0] >= 0 and xyz[2] >= 0,
            'Position outside server wire bounds')
    return dict(zip('xyz', xyz))


def property_integer(value, label, lower, upper):
    result = number(value, label)
    require(result.is_integer(), 'Nonintegral property: ' + label)
    return integer(int(result), label, lower, upper)


def vehicle_properties(tree):
    """Expand every physical seat in authored list order, including nonpassengers."""
    props = tree['GroundVehicleProperties']
    seats = props['Seats']
    physical, passengers = 0, []
    for ordinal, (name, wrapper) in enumerate(seats['SeatPropertiesList'].items()):
        require(name == f'SeatProperties{ordinal}', 'Seat properties must be contiguous in authored order')
        row = wrapper['SeatProperties']
        count = property_integer(row.get('SeatCount', 1), 'SeatCount', 1, 17)
        require(physical + count <= 16, 'Physical seat index exceeds supported bounds')
        if row['SeatType'] == 'PassengerSeat':
            passengers.extend(range(physical, physical + count))
        physical += count
    occupancy = property_integer(seats['MaxOccupancy'], 'MaxOccupancy', 1, 17)
    require(passengers and occupancy <= len(passengers), 'Invalid authored passenger capacity')
    health = number(props['DamageProperties']['MaxHitpoints'], 'MaxHitpoints') * 4
    require(health.is_integer() and 1 <= health < 4096, 'Health exceeds quarter-point wire bounds')
    return dict(passenger_seats=passengers, maximum_occupancy=occupancy,
                health_quarter_points=int(health)), f32(number(props['Placement_Y_Offset'], 'Placement_Y_Offset'))


def inverse_rotate(rotation, value):
    """Conjugate of the supplied unit quaternion; never infer Euler conventions."""
    x, y, z, w = rotation
    x, y, z = -x, -y, -z
    vx, vy, vz = value
    tx, ty, tz = 2*(y*vz-z*vy), 2*(z*vx-x*vz), 2*(x*vy-y*vx)
    return [vx+w*tx+y*tz-z*ty, vy+w*ty+z*tx-x*tz, vz+w*tz+x*ty-y*tx]


def exit_policy(row, map_name, point_token, faction, origin, rotation):
    require(row['map'] == map_name and token(row['spawn_token']) == point_token
            and type(row['faction']) is int and row['faction'] == faction, 'Exit map/token/faction mismatch')
    require(row.get('qualification') == 'neutral_static_ground_and_clearance', 'Unqualified exits')
    require(isinstance(row.get('provenance'), dict) and row['provenance'], 'Missing exit provenance')
    require(vector(row['vehicle_world_position'], 3, 'exit origin') == origin,
            'Exit vehicle origin mismatch (placement offset must be applied exactly once)')
    require(quaternion(row['rotation_xyzw']) == rotation, 'Exit vehicle quaternion mismatch')
    candidates = row['candidates']
    require(isinstance(candidates, list) and 0 < len(candidates) <= 32, 'Missing or excessive exit candidates')
    result = []
    for candidate in candidates:
        feet = vector(candidate['position_world'], 3, 'settled world feet')
        position(feet)
        ranking = vector(candidate['ranking_offset_model'], 3, 'ranking offset')
        offset = inverse_rotate(rotation, [v-o for v, o in zip(feet, origin)])
        require(all(abs(v) < 2048 for v in ranking + offset)
                and sum(v*v for v in ranking) > 1.1920928955078125e-7, 'Invalid exit candidate')
        result.append(dict(ranking_offset=[f32(v) for v in ranking],
                           position_offset=[f32(v) for v in offset]))
    return result


def project(spawn_catalog, authored_map, bindings, registry, property_trees, exits,
            *, property_hashes, binding_proof):
    """Return (remapped SpawnCatalog, ground catalog) without changing inputs.

    Registry is retail_map_helpers.vehicle_registry output. Property trees are
    keyed by the exact variant PropertiesFileName; property_hashes is the original
    member-name -> raw-member SHA256 manifest (including vehicles.zpr). The
    caller supplies original extraction and independently qualified binding and
    exit evidence; this API validates their joins and does not certify provenance.
    """
    require(spawn_catalog.get('format') == 'mag_spawn_catalog'
            and spawn_catalog.get('schema_version') == 1, 'Unsupported SpawnCatalog')
    require(bindings.get('format') == 'mag_spawn_bindings' and bindings.get('schema_version') == 1
            and bindings.get('qualification') == 'native_initial_load', 'Unqualified native bindings')
    require(isinstance(binding_proof, bytes) and sha(binding_proof) == digest(bindings['proof_sha256'], 'proof'),
            'Binding proof SHA256 mismatch')
    digest(bindings['native_elf_sha256'], 'native ELF')
    result = copy.deepcopy(spawn_catalog)
    require(len(result['maps']) == 1, 'Require exactly one selected map')
    pool = result['maps'][0]
    map_name = pool['name']
    source = json.loads(result['source'])
    require(map_name == bindings['map_name'] == authored_map['map']['map']
            == result['active']['map_name'] == source['map'], 'Map mismatch')
    require(authored_map['map']['mode_enum'] == 'GameType128'
            and result['match_profile']['game_type'] == 1, 'APC insertion requires Acquisition')
    faction = integer(result['match_profile']['attacking_faction'], 'attacking faction', 0, 3)
    require(pool['owner_faction'] != faction, 'Attacker must oppose map owner')
    for field, authored_field in [('level_archive_sha256', 'archive_sha256'), ('level_clv_sha256', 'clv_sha256')]:
        expected = digest(bindings[field], field)
        require(source[field] == authored_map['source'][authored_field] == expected,
                'Source level hash mismatch: ' + field)
    count = len(pool['locations'])
    require(0 < count <= 128 and count == pool['spawn_count'] == len(authored_map['points'])
            == len(bindings['points']), 'Spawn pool cardinality mismatch')
    old_locations, authored, bound, new_ids = {}, {}, {}, set()
    old_ids = set()
    for row in pool['locations']:
        t = token(row['token']); i = integer(row['index'], 'old spawn index', 0, count)
        require(t not in old_locations and i not in old_ids, 'Duplicate SpawnCatalog identity')
        old_locations[t] = row; old_ids.add(i)
    for row in authored_map['points']:
        t = token(row['name_token'])
        require(t not in authored, 'Duplicate authored token')
        authored[t] = row
    for row in bindings['points']:
        t = integer(row['token'], 'binding token', 1, 2**32)
        i = integer(row['index'], 'native spawn index', 0, count)
        require(t not in bound and i not in new_ids, 'Duplicate native token/index')
        position(row['position']); quaternion(row['rotation_xyzw'])
        bound[t] = row; new_ids.add(i)
    require(old_locations.keys() == authored.keys() == bound.keys(), 'Native token bijection mismatch')
    remap = {row['index']: bound[t]['index'] for t, row in old_locations.items()}
    enabled = {t for t, p in authored.items() if p['active'] is True and p['spawn_type'] == 3
               and p['user_class'] == 0 and p['category'] == 0}
    for t, row in old_locations.items():
        p = authored[t]
        require(type(row['active']) is bool and type(p['active']) is bool
                and row['active'] == p['active'], 'Authored active flag mismatch')
        basic = p['spawn_type'] == 0 and p['user_class'] == 0 and p['category'] in (0, 1)
        expected_kind = ('insertion' if p['category'] == 0 else 'alternate') if basic else 'unsupported'
        require(row['kind'] == expected_kind, 'Require original pre-APC authored spawn kinds')
        row['index'] = bound[t]['index']
        row['position'] = position(bound[t]['position'])
        if t in enabled:
            require(row['kind'] == 'unsupported', 'APC insertion must replace an unsupported authored point')
            row['kind'] = 'vehicle_passenger'
    restored = set()
    for squad in pool['squads']:
        # Reconstruct original slot order, preserving all supported authored links.
        linked = []
        for t, p in authored.items():
            for link in p['squad_links']:
                if (link['team'], link['platoon_id'], SQUADS[link['squad']]) == (squad['team'], squad['platoon'], squad['squad']):
                    linked.append((link['slot'], t))
        require(len({slot for slot, _ in linked}) == len(linked), 'Duplicate authored squad slot')
        available = [bound[t]['index'] for _, t in sorted(linked)
                     if old_locations[t]['active'] and old_locations[t]['kind'] != 'unsupported']
        apc_defaults = [bound[t]['index'] for _, t in linked if t in enabled]
        require(len(apc_defaults) <= 1, 'Ambiguous authored APC squad insertion')
        squad['default_spawn'] = apc_defaults[0] if apc_defaults else remap[squad['default_spawn']]
        if apc_defaults:
            restored.add(squad['index'])
        require(squad['default_spawn'] in available, 'Squad default is unavailable')
        squad['available'] = available
    active = next((s for s in pool['squads'] if s['index'] == result['active']['squad_index']), None)
    require(active is not None, 'Active squad missing')
    result['active'].update(default_spawn=active['default_spawn'], available=list(active['available']))
    for row in pool.get('forward_spawns', []):
        row['spawn_point_index'] = remap[row['spawn_point_index']]
    for row in source.get('unsupported_spawn_links', []):
        row['index'] = remap[row['index']]
    source['default_spawn_fallbacks'] = [dict(row, default_spawn=remap[row['default_spawn']])
        for row in source.get('default_spawn_fallbacks', []) if row['squad_index'] not in restored]
    pool.update(runtime_indices_verified=True, runtime_indices_basis=BASIS)
    pool['locations'].sort(key=lambda p: p['index'])
    require(enabled, 'No active authored player APC insertion points')
    rows = exits['rows'] if isinstance(exits, dict) else exits
    require(isinstance(rows, list), 'Expected qualified exit rows')
    exit_rows = {}
    for row in rows:
        key = (row['map'], token(row['spawn_token']), integer(row['faction'], 'exit faction', 0, 3))
        require(key not in exit_rows, 'Duplicate qualified exit identity')
        exit_rows[key] = row
    vehicles, passengers, used_properties, used_exits = [], [], {}, []
    registry_member = '/run/mag/ps3/scripts/vehicles.zpr'
    used_properties[registry_member] = digest(property_hashes[registry_member], 'registry member')
    for t in sorted(enabled, key=lambda t: bound[t]['index']):
        p, b = authored[t], bound[t]
        require(p['team'] == 0, 'Only attacking APC player insertion is supported')
        require(vector(p['scale_xyz'], 3, 'authored scale') == [1, 1, 1], 'Nonunit APC scale')
        vehicle_token = token(p['vehicle_type_token'])
        variants = [v for v in registry[f'0x{vehicle_token:08x}']['variants']
                    if v['faction'] == FACTIONS[faction] and str(v['ModifierLevel']) == '0']
        require(len(variants) == 1, 'Require unique selected-faction modifier-zero vehicle variant')
        variant = variants[0]; filename = variant['PropertiesFileName']
        require(isinstance(filename, str) and Path(filename).name == filename, 'Invalid vehicle property filename')
        props, offset = vehicle_properties(property_trees[filename])
        member = '/run/mag/ps3/scripts/vehicle/' + filename
        used_properties[member] = digest(property_hashes[member], 'vehicle properties member')
        base = [f32(v) for v in vector(b['position'], 3, 'bound position')]
        origin = [base[0], f32(base[1]+offset), base[2]]
        rotation = quaternion(b['rotation_xyzw'])
        row = exit_rows.get((map_name, t, faction))
        require(row is not None, 'Missing qualified APC exits')
        candidates = exit_policy(row, map_name, t, faction, origin, rotation)
        vehicle_index = len(vehicles)
        require(vehicle_index < 32, 'Too many ground vehicles')
        vehicles.append(dict(index=vehicle_index, spawn_point_index=b['index'], spawn_point_token=t,
            vehicle_type_token=vehicle_token, faction=faction, team=p['team'], modifier=0,
            position=position(origin), rotation=[f32(v) for v in rotation],
            **props, passenger_exit_candidates=candidates))
        passengers.append(dict(spawn_point_index=b['index'], spawn_point_token=t, vehicle_index=vehicle_index))
        used_exits.append(row)
    property_manifest = dict(sorted(used_properties.items()))
    source.update(note=BASIS, native_binding_sha256=bindings['proof_sha256'],
                  native_elf_sha256=bindings['native_elf_sha256'],
                  native_bindings_input_sha256=sha(encode(bindings)),
                  vehicle_properties=property_manifest,
                  qualified_exits_sha256=sha(encode(used_exits)),
                  vehicle_network_index_policy='Local sequential type1 IDs in native spawn-point order; not original world-pool identity',
                  apc_exit_policy='External neutral static ground/clearance qualification; no dynamic or native live acceptance claim')
    result['source'] = json.dumps(source, sort_keys=True, allow_nan=False)
    require(len(result['source'].encode()) <= 8192, 'Spawn provenance exceeds runtime source bound')
    ground = dict(format='mag_ground_vehicles', schema_version=1,
        source=dict(spawn_catalog_sha256=sha(encode(result)), native_elf_sha256=bindings['native_elf_sha256'],
                    native_binding_sha256=bindings['proof_sha256'], vehicle_properties_sha256=sha(encode(property_manifest))),
        vehicles=vehicles, passenger_spawns=passengers)
    return result, ground


def render(disc, game_archive, spawn_catalog, bindings, exits, binding_proof, native_elf):
    """Read pinned original archives with the maintained parsers, then project."""
    elf_sha = sha(native_elf.read_bytes())
    require(elf_sha == SUPPORTED_NATIVE_ELF_SHA256, 'Wrong retail native ELF')
    normalized = json.loads(bindings.read_bytes())
    require(normalized['native_elf_sha256'] == elf_sha, 'Binding native ELF SHA256 mismatch')
    builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    helpers = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
    extractor = builder['extractor']()
    raw = game_archive.read_bytes()
    require(sha(raw) == GAME_ARCHIVE_SHA256, 'Wrong retail game.psarc')
    archive = extractor.Psarc(raw)
    def member(name):
        require(archive.names.count(name) == 1, 'Missing/ambiguous original archive member: ' + name)
        return archive.extract(archive.names.index(name)+1)
    catalog = json.loads(spawn_catalog.read_bytes())
    map_name = normalized['map_name']
    require(re.fullmatch('[A-Za-z0-9_]+', map_name) is not None, 'Invalid map name')
    settings = member('/run/mag/ps3/settings/gameproperties.zpr')
    infos = [m for m in extractor.catalog_settings(settings.decode()) if m['map'] == map_name]
    require(len(infos) == 1, 'Require one original configured map')
    authored = extractor.extract_map(disc/'PS3_GAME/USRDIR/run/mag/ps3/levels'/(map_name+'.psarc'), infos[0])
    registry_name = helpers['VEHICLES_MEMBER']
    registry_raw = member(registry_name)
    registry = helpers['vehicle_registry'](helpers['parse_zpr'](registry_raw.decode()))
    faction = catalog['match_profile']['attacking_faction']
    require(type(faction) is int and 0 <= faction < 3, 'Invalid attacking faction')
    hashes, trees = {registry_name: sha(registry_raw)}, {}
    for p in authored['points']:
        if p['active'] is True and (p['spawn_type'], p['user_class'], p['category']) == (3, 0, 0):
            variants = [v for v in registry[p['vehicle_type_token']]['variants']
                        if v['faction'] == FACTIONS[faction] and str(v['ModifierLevel']) == '0']
            require(len(variants) == 1, 'Require unique selected-faction modifier-zero vehicle variant')
            filename = variants[0]['PropertiesFileName']
            require(Path(filename).name == filename, 'Invalid vehicle property filename')
            name = '/run/mag/ps3/scripts/vehicle/' + filename
            raw_props = member(name)
            hashes[name] = sha(raw_props)
            trees[filename] = helpers['parse_zpr'](raw_props.decode())
    require(json.loads(catalog['source'])['game_archive_sha256'] == sha(raw), 'Spawn game archive hash mismatch')
    return project(catalog, authored, normalized, registry, trees, json.loads(exits.read_bytes()),
                   property_hashes=hashes, binding_proof=binding_proof.read_bytes())


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('disc', 'game-archive', 'spawn-catalog', 'bindings', 'exits', 'binding-proof', 'native-elf', 'spawn-output', 'ground-output'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Require both outputs already reproduce byte-for-byte')
    args = parser.parse_args()
    inputs = [args.game_archive, args.spawn_catalog, args.bindings, args.exits, args.binding_proof, args.native_elf]
    outputs = [args.spawn_output, args.ground_output]
    resolved = {p.resolve() for p in inputs}
    if not (len({p.resolve() for p in outputs}) == 2 and all(p.resolve() not in resolved
            and not p.resolve().is_relative_to(args.disc.resolve()) for p in outputs)):
        parser.error('Outputs must be distinct, separate from every input, and outside the original disc')
    try:
        spawn, ground = render(args.disc, args.game_archive, args.spawn_catalog, args.bindings, args.exits, args.binding_proof, args.native_elf)
        for path, value in zip(outputs, (spawn, ground)):
            raw = encode(value)
            if args.check:
                require(path.is_file() and path.read_bytes() == raw, 'Catalog differs from reproduction: ' + str(path))
            else:
                path.write_bytes(raw)
    except (KeyError, TypeError, ValueError) as error:
        parser.exit(1, str(error)+'\n')


if __name__ == '__main__':
    main()
