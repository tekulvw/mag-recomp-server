#!/usr/bin/env python3
"""Bind verified Acquisition Structural entries, native FSP rows and authored script hooks."""
import argparse
from collections import defaultdict
import hashlib
import json
from pathlib import Path
import runpy
import struct

TOOLS = Path(__file__).resolve().parent
METADATA = '38703201deb85a4bf836164492658344b109eebb68274b5179b71b53bb93a9ab'
RETAIL = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
STRUCTURAL = 'ObjectiveDemolitionStructuralProperties'
SECURE = 'ObjectiveSecureAndHoldProperties'
TACTICAL = '/run/mag/ps3/scripts/objective_control/tactical_asset_control.lua'
TERTIARY = '/run/mag/ps3/scripts/objective_control/tertiary_objective_control.lua'
SCRIPTS = {
    TACTICAL: '62b59a815a6942a3f85847bae1251dd56c73ff6e146d87c3174c38f088e99c61',
    TERTIARY: 'f4fd90f9fa70fc6ebdbf993b6012d7e6fe714a089db1f7039436e51cfd7e2acc',
}
HOOKS = {'DeactivatedScript': 1, 'RepairedScript': 6, 'CompletedScript': 7}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(payload):
    return hashlib.sha256(payload).hexdigest()


def token(value):
    require(isinstance(value, str) and value.startswith('0x') and len(value) == 10,
            'Invalid authored token')
    return int(value[2:], 16)


def unique(rows, key):
    result = {row[key]: row for row in rows}
    require(len(result) == len(rows), f'Duplicate source {key}')
    return result


def enum(field):
    raw = bytes.fromhex(field['raw_hex'])
    require(len(raw) == 13 and raw[:9] == bytes.fromhex('deadbeef0000000100'),
            'Invalid source enum')
    return struct.unpack_from('>I', raw, 9)[0]


def script_hook(script, owner, platoons, destinations):
    require(script['immediate'] and not script['delayed'], 'Unqualified Structural script timing')
    row = script['immediate']
    require(row['delay_ms'] == 0 and row['member'] in SCRIPTS, 'Unqualified Structural script identity')
    require(all(arg.keys() == {'Object', 'Number'} and arg['Number'] == 0
                for arg in row['parameters']), 'Unqualified Structural script argument')
    args = [token(arg['Object']) for arg in row['parameters']]
    require(args and args[0] == owner, 'Script reads a different underlying objective')
    fn = row['function']
    if row['member'] == TACTICAL and fn == 'AntiaircraftControl':
        require(len(args) == 6 and platoons.get(args[5]) in (0, 1), 'Invalid anti-air script binding')
        require(len(set(args[1:5])) == 4, 'Duplicate anti-air spawn target')
        return {'function': 'antiaircraft', 'platoon': platoons[args[5]],
                'spawns': [destinations[arg] for arg in args[1:5]]}, args[1:5]
    if row['member'] == TACTICAL and fn in ('MortarControl', 'SensorControl'):
        require(len(args) == 2 and platoons.get(args[1]) in (4, 5), 'Invalid defensive script binding')
        return {'function': 'mortar' if fn == 'MortarControl' else 'sensor',
                'platoon': platoons[args[1]]}, []
    if row['member'] == TERTIARY and fn == 'MotorPoolControl':
        require(len(args) == 2, 'Invalid motor-pool script binding')
        return {'function': 'motor_pool', 'spawn': destinations[args[1]]}, args[1:]
    raise ValueError(f'Unsupported Structural script function {fn}')


def project(disc, metadata_path, objective_path, spawn_path, game_path):
    metadata_raw = metadata_path.read_bytes()
    require(sha(metadata_raw) == METADATA, 'Wrong original retail map metadata')
    metadata = json.loads(metadata_raw)
    objective = json.loads(objective_path.read_bytes())
    spawn = json.loads(spawn_path.read_bytes())
    require(objective['game_type'] == spawn['match_profile']['game_type'] == 1
            and len(spawn['maps']) == 1, 'Require Acquisition profile inputs')
    pool, = spawn['maps']
    source = objective['source']
    require(pool['name'] == objective['map_name'] and pool['runtime_indices_verified']
            and source['authored_catalog_sha256'] == METADATA
            and source['source_image_sha256'] == RETAIL
            and source['capture_qualification'] == 'native_source_gameplay',
            'Objective or spawn profile source mismatch')
    authored, = [row for row in metadata['maps'] if row['map']['map'] == pool['name']]
    require(authored['map']['mode_enum'] == 'GameType128', 'Wrong authored mode')
    level_source = authored['source']
    builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    original = builder['extractor']()
    script_helper = runpy.run_path(str(TOOLS / 'project-retail-sabotage-objectives.py'))
    build = runpy.run_path(str(TOOLS / 'extract-retail-domination.py'))
    game = game_path.read_bytes()
    require(sha(game) == builder['GAME_ARCHIVE_SHA256'], 'Wrong original game archive')
    archive = original.Psarc(game)
    hashes = {}
    scripts, _ = build['read_script_bindings'](disc, authored, original, script_helper, archive, hashes)
    require({member: hashes.get(member) for member in SCRIPTS} == SCRIPTS,
            'Structural script source changed')
    scripts = unique(scripts, 'instance_token')
    original_level = (disc / level_source['archive']).read_bytes()
    require(sha(original_level) == level_source['archive_sha256'], 'Wrong original level archive')
    level = original.Psarc(original_level)
    clv = level.extract(level.names.index(level_source['clv_member']) + 1)
    require(sha(clv) == level_source['clv_sha256'], 'Wrong original CLV')
    parsed = original.retail_clv(clv)
    script_helper['register'](parsed, 'PlatoonProperties')
    values = script_helper['values']
    platoon_defs = {table['name_token']: values(table['fields'])
                    for table in parsed.tables('PlatoonProperties')}
    placed_objects = list(parsed.tables('InstanceProperties'))
    placed_tokens = {values(row['fields'])['Name'] for row in placed_objects}
    platoons = {}
    for row in placed_objects:
        if row['preceding_token'] not in platoon_defs:
            continue
        data = platoon_defs[row['preceding_token']]
        index = 4 * data['Team'] + data['ID']
        key = token(values(row['fields'])['Name'])
        require(key not in platoons and index not in platoons.values(), 'Duplicate original platoon')
        platoons[key] = index
    require(set(platoons.values()) == {0, 1, 4, 5}, 'Incomplete original platoon roster')
    points = {token(row['name_token']): row for row in authored['points']}
    placed = unique(pool['locations'], 'token')
    require(placed.keys() == points.keys(), 'Original spawn point coverage mismatch')
    graph = authored['objectives']
    definitions = unique(graph['definitions'], 'definition_token')
    instances = {token(row['name_token']): definitions[row['definition_token']]
                 for row in graph['instances']}
    indices = {token(row['token']): row['object_index'] for row in objective['objects']}
    require(indices.keys() <= instances.keys() and len(set(indices.values())) == len(indices),
            'Original objective binding coverage mismatch')
    rows = {}
    passive = set()
    absent_hooks = 0
    for key, definition in instances.items():
        if definition['class_name'] != STRUCTURAL:
            continue
        fields = definition['fields']
        found = {}
        for field, value in fields.items():
            if not field.endswith('Script') or value['raw_hex'] == '00000000':
                continue
            require(field in HOOKS, 'An original Structural hook requires another state')
            script_token = '0x' + value['raw_hex']
            if script_token not in scripts:
                require(pool['name'] == 'mag_alaska_subpen_final'
                        and field == 'DeactivatedScript'
                        and script_token == '0xce51686a' and script_token not in placed_tokens,
                        'A Structural hook is missing its original ScriptObject')
                absent_hooks += 1
                continue
            hook, targets = script_hook(scripts[script_token], key, platoons, indices)
            passive.update(targets)
            found[HOOKS[field]] = hook
        require(fields['Active']['raw_hex'] == '01'
                and fields['InitialActiveState']['raw_hex'] == 'deadbeef000000010000000000',
                'Unqualified Structural MissionStart request')
        rows[key] = {'token': key, 'class': 'structural', 'hooks': found, 'start_deactivated': True}
    require(absent_hooks == (4 if pool['name'] == 'mag_alaska_subpen_final' else 0)
            and sum(len(row['hooks']) for row in rows.values()) + absent_hooks == 30
            and len(passive) == 12,
            'Incomplete original Structural callback coverage')
    for key in passive:
        definition = instances[key]
        fields = definition['fields']
        require(definition['class_name'] in (STRUCTURAL, SECURE)
                and enum(fields['SubType']) == 0
                and fields['Sequences']['raw_hex'] == '0000000100000000'
                and all(v['raw_hex'] == '00000000' for k,v in fields.items() if k.endswith('Script')),
                'A linked spawn has active scripts or sequences')
        if definition['class_name'] == STRUCTURAL:
            require(fields['DamageValue']['value'] == 0
                    and fields['CharacterDamageOverTimeValue']['value'] == 0,
                    'A passive motor-pool spawn objective deals damage')
        else:
            rows[key] = {'token': key, 'class': 'secure_hold', 'hooks': {}, 'start_deactivated': False}
    require(rows.keys() <= indices.keys(), "Missing Structural objective identity")

    for key, row in rows.items():
        fields = instances[key]['fields']
        require(fields['ActivationScoreValue']['value'] == fields['DeactivationScoreValue']['value'] == 0,
                'A sourced Structural entry has direct score effects')
        fsp = int(fields['ForwardSpawnPoint']['raw_hex'], 16)
        row['spawn'] = None
        if fsp:
            original_point = points[fsp]
            user = original_point['user_class']
            vehicle = user == 1
            require((user, original_point['spawn_type']) in ((0, 4), (0, 5), (0, 8), (1, 7)),
                    'Unqualified Structural linked vehicle source')
            row['spawn'] = {
                'token': fsp, 'index': placed[fsp]['index'], 'team': original_point['team'],
                'user_class': user, 'spawn_type': original_point['spawn_type'],
                'fsp_type': enum(fields['FSPType']), 'vehicle': vehicle,
            }
    squads = defaultdict(dict)
    names = ('Alpha', 'Bravo', 'Charlie', 'Delta')
    for key, original_point in points.items():
        for link in original_point['squad_links']:
            slot = link['team'] * 16 + link['platoon_id'] * 4 + names.index(link['squad'])
            require(link['slot'] not in squads[slot], 'Duplicate original squad spawn slot')
            squads[slot][link['slot']] = key
    require(set(squads) == {0,1,2,3,4,5,6,7,16,17,18,19,20,21,22,23},
            'Incomplete original squad membership')
    squad_rows = {}
    for slot, members in squads.items():
        keys = [key for _, key in sorted(members.items())]
        fallback = next((key for key in keys if points[key]['category'] == 0), None)
        squad_rows[slot] = {
            'points': [(placed[key]['index'], key) for key in keys],
            'category_zero': [placed[fallback]['index'], fallback] if fallback is not None else None,
        }
    return {
        'schema': 'mag_acquisition_structural_bindings_v1',
        'map': pool['name'], 'metadata_sha256': METADATA,
        'initial_objective_sha256': source['binding_capture_sha256'],
        'level_archive_sha256': level_source['archive_sha256'],
        'level_clv_sha256': level_source['clv_sha256'],
        'rows': {index: rows[key] for key, index in sorted(indices.items(), key=lambda row: row[1])
                 if key in rows},
        'squads': dict(sorted(squad_rows.items())),
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for arg in ('disc', 'metadata', 'objective-catalog', 'spawn-catalog', 'game', 'output'):
        parser.add_argument('--' + arg, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.output.resolve().is_relative_to(TOOLS.parents[1]):
        parser.error('Output must be outside the source checkout')
    result = project(args.disc, args.metadata, args.objective_catalog, args.spawn_catalog, args.game)
    raw = (json.dumps(result, indent=2, allow_nan=False) + '\n').encode()
    if args.check:
        require(args.output.read_bytes() == raw, 'Structural bindings differ from source reproduction')
    else:
        with args.output.open('xb') as file:
            file.write(raw)


if __name__ == '__main__':
    main()
