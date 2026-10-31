#!/usr/bin/env python3
"""Project original Syr Darya objective/control metadata; never execute original Lua.

No native resources, runtime object identities or general Lua interpretation are
supplied. Typed CLV fields retain raw bytes/offsets for unresolved semantics.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy
import struct
import sys

TOOLS = Path(__file__).resolve().parent
GAME_SHA = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
MAP = 'mag_cosmodrome_final'
LEVEL = 'PS3_GAME/USRDIR/run/mag/ps3/levels/' + MAP + '.psarc'
LEVEL_SHA = '8157f5abc55ddd2ca9eaad3379e87ae909d0b8f7c9b08fa04f1f1e2d8018d96e'
CLV = '/run/mag/ps3/levels/' + MAP + '/' + MAP + '.clv'
CLV_SHA = 'd336caea88351dda5f3204a34e64fdde8eabdf0532b3133e2338cdd150488f4b'
MEMBERS = {
    'scripts/objective_control/gametype_64.lua': '39ad009271454fec6352822201aa58dac14e4c2900c82ac2a3a8a4f2d71b16b9',
    'scripts/objective_control/gametype_timer_control.lua': '1a1f6cf1af260faeadac6e76f9b917ee51b96b0627b5d615cd14f0f9b9430faf',
    'scripts/game_objective_descriptions/gametype_64_descriptions.zpr': '31f8dfd74957f2855d11509510055c659c56ad778cdf7e626a27b1d70375ba87',
    'scripts/game_objective_descriptions/gametype_description_table_locations.zpr': '7316233becc911e5782aaccc2ca8d60e2416c734609130833601b406983fe93c',
}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def checked(raw, expected, label):
    require(sha(raw) == expected, 'Original resource hash mismatch: ' + label)
    return raw


def register(clv, classname):
    marker = struct.pack('>I', len(classname) + 1) + classname.encode() + b'\0'
    tokens = set()
    for match in re.finditer(re.escape(marker), clv.b):
        p = match.start()
        if (p >= 24 and clv.b[p - 8:p] == bytes.fromhex('deadbeef00000003')
                and clv.u32(p - 24) == 7):
            tokens.add(clv.u32(p - 20))
    require(len(tokens) == 1, 'Missing/ambiguous property class: ' + classname)
    token = tokens.pop()
    clv.tokens[token] = classname
    clv.byname[classname] = token


def decoded_fields(clv, fields, depth=0):
    """Decode only string and inline-property vectors; retain original typed data."""
    require(depth <= 4, 'Nested property vector depth exceeded')
    result = []
    for field in fields:
        row = dict(field)
        raw = bytes.fromhex(field['raw_hex'])
        if field['type'] == 'ztl::string':
            require(len(raw) >= 4 and int.from_bytes(raw[:4], 'big') == len(raw) - 4,
                    'Invalid authored string boundary')
            row['decoded_value'] = raw[4:].rstrip(b'\0').decode('utf-8')
        elif field['type'] == 'ztl::vector':
            require(len(raw) >= 8, 'Invalid property vector header')
            version, count = struct.unpack_from('>II', raw)
            require(version == 1 and count <= 128, 'Unsupported property vector')
            cursor = int(field['offset'], 16) + 8
            end = int(field['offset'], 16) + field['bytes']
            values = []
            for _ in range(count):
                require(clv.b[cursor:cursor + 8] == bytes.fromhex('deadbeef00000003'),
                        'Unsupported inline property encoding')
                classname, start = clv.string(cursor + 8)
                nested, finish = clv.fields(start, end)
                values.append({'class_name': classname,
                               'fields': decoded_fields(clv, nested, depth + 1),
                               'source': {'offset': hex(cursor), 'end': hex(finish + 1),
                                          'sha256': sha(clv.b[cursor:finish + 1])}})
                cursor = finish + 1
            require(cursor == end, 'Invalid inline property vector boundary')
            row['decoded_value'] = values
        result.append(row)
    return result


def values(fields):
    return {f['name']: f.get('decoded_value', f['value']) for f in fields}


def instance_rows(instances, token):
    rows = [dict(i, runtime_pool_id=None) for i in instances if i['preceding_token'] == token]
    require(len(rows) == 1, 'Expected one original property instance')
    return rows


def script_facts(raw):
    """Convert a hash-pinned literal Lua subset to data; never emit/execute code.

    Source order and conditions are recorded, but branch scopes/general Lua
    semantics are deliberately not reconstructed by this inventory projector.
    """
    text = raw.decode('utf-8')
    matches = list(re.finditer(r'^function\s+(\w+)\(([^)]*)\)', text, re.M))
    functions = []
    def argument(token):
        token = token.strip()
        if token in ('true', 'false'):
            return {'kind': 'bool', 'value': token == 'true'}
        if re.fullmatch(r'-?\d+', token):
            return {'kind': 'integer', 'value': int(token)}
        if token.startswith('"'):
            return {'kind': 'string', 'value': json.loads(token)}
        require(bool(re.fullmatch(r'[A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*', token)),
                'Unsupported script argument expression')
        return {'kind': 'symbol', 'name': token}
    for index, match in enumerate(matches):
        end = matches[index + 1].start() if index + 1 < len(matches) else len(text)
        body = text[match.end():end]
        calls, conditions = [], []
        for line, content in enumerate(body.splitlines(), text[:match.end()].count('\n') + 1):
            call = re.fullmatch(r'\s*ZScript\.(\w+)\((.*)\)\s*', content)
            if call:
                # This pinned subset has no commas inside string literals.
                calls.append({'source_line': line, 'api': call[1],
                              'arguments': [argument(t) for t in call[2].split(',')] if call[2] else []})
            if content.lstrip().startswith('if '):
                comparisons = re.findall(r'ZScript\.Mission_GetObjectiveState\((\w+)\)\s*(~=|==)\s*(ZScript\.ObjectiveState\.\w+)', content)
                require(comparisons and ' and ' in content, 'Unsupported script condition')
                conditions.append({'source_line': line, 'operator': 'all',
                                   'comparisons': [{'objective_parameter': obj,
                                                    'operator': 'not_equal' if op == '~=' else 'equal',
                                                    'state_symbol': state}
                                                   for obj, op, state in comparisons]})
        parameters = [p.strip() for p in match[2].split(',') if p.strip()]
        require(all(re.fullmatch(r'\w+', p) for p in parameters), 'Invalid script signature')
        functions.append({'name': match[1], 'parameters': parameters,
                          'conditions_in_source_order': conditions,
                          'calls_in_source_order': calls})
    require(functions, 'Missing original script functions')
    return {'functions': functions,
            'interpretation_scope': 'Literal signatures, comparisons and standalone calls only. Call order is source order; branch scopes and generalized Lua semantics are not implemented.'}


def project(disc, game_archive):
    x = runpy.run_path(str(TOOLS / 'extract-retail-spawns.py'))
    maps = runpy.run_path(str(TOOLS / 'extract-retail-maps.py'))
    helpers = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
    game = x['Psarc'](checked(game_archive.read_bytes(), GAME_SHA, 'game.psarc'))
    def member(relative):
        path = '/run/mag/ps3/' + relative
        return game.extract(game.names.index(path) + 1)
    settings = member('settings/gameproperties.zpr')
    configured = [m for m in x['catalog_settings'](settings.decode()) if m['map'] == MAP]
    require(len(configured) == 1 and configured[0]['mode_enum'] == 'GameType64'
            and configured[0]['owner_faction_enum'] == 'FactionTypeRed',
            'Unexpected configured Sabotage map identity')
    archive = x['Psarc'](checked((disc / LEVEL).read_bytes(), LEVEL_SHA, LEVEL))
    raw = checked(archive.extract(archive.names.index(CLV) + 1), CLV_SHA, CLV)
    graph = maps['objective_graph'](raw, helpers)
    require(len(graph['definitions']) == len(graph['instances']) == 13,
            'Unexpected objective cardinality')
    clv = x['retail_clv'](raw)
    instances = clv.tables('InstanceProperties')
    records = {}
    for classname in ('ScriptObjectProperties', 'SquadObjectiveListPropTable', 'PlatoonProperties'):
        register(clv, classname)
        rows = []
        for table in clv.tables(classname):
            row = {'definition_token': f"0x{table['name_token']:08x}",
                   'class_name': classname, 'fields': table['fields'],
                   'instances': instance_rows(instances, table['name_token']),
                   'source': x['evidence'](table), 'runtime_pool_id': None}
            if classname != 'PlatoonProperties':
                row['fields'] = decoded_fields(clv, table['fields'])
            else:
                row['squad_spawn_references'] = clv.squad_references(table)
            rows.append(row)
        records[classname] = rows
    require([len(records[c]) for c in records] == [4, 8, 2], 'Unexpected control/list/platoon counts')
    resources = []
    for path, expected in MEMBERS.items():
        data = checked(member(path), expected, path)
        resources.append({'member': '/run/mag/ps3/' + path, 'bytes': len(data),
                          'sha256': sha(data),
                          'converted_properties': script_facts(data) if path.endswith('.lua')
                              else helpers['parse_zpr'](data.decode('utf-8'))})
    controls = []
    for row in records['ScriptObjectProperties']:
        field = values(row['fields'])
        if field['ServerFunction'] != 'ObjectiveControl':
            continue
        script = next(r for r in resources if r['member'].endswith('/' + field['ServerScript']))
        signatures = [f for f in script['converted_properties']['functions']
                      if f['name'] == field['ServerFunction']]
        require(len(signatures) == 1, 'Expected one original ObjectiveControl signature')
        names = signatures[0]['parameters']
        params = field['ServerParams']
        require(len(names) == len(params) and all(re.fullmatch(r'\w+', n) for n in names),
                'Script signature/argument mismatch')
        controls.append({'script_instance_token': values(row['instances'][0]['fields'])['Name'],
                         'script_member': script['member'], 'function': field['ServerFunction'],
                         'bindings': [{'parameter': name, **values(param['fields'])}
                                      for name, param in zip(names, params)],
                         'basis': 'Original function parameter names joined to ordered authored CLV arguments; no Lua execution or generalized state-machine interpretation.'})
    require(len(controls) == 1, 'Missing original control argument binding')
    triggers = []
    forward = []
    spawn = x['extract_map'](disc / LEVEL, configured[0])
    by_token = {p['name_token']: p for p in spawn['points']}
    for definition in graph['definitions']:
        field = definition['fields']
        inst = next(i for i in graph['instances'] if i['definition_token'] == definition['definition_token'])
        if definition['class_name'] == 'ObjectiveTriggerProperties':
            triggers.append({'trigger_instance_token': inst['name_token'],
                             'objective_instance_token': field['ObjectiveID']['token'],
                             'volume_instance_token': field['ObjectiveTriggerVolume']['token']})
        point = field.get('ForwardSpawnPoint', {}).get('token')
        if point and point != '0x00000000':
            require(point in by_token, 'Unresolved authored forward spawn token')
            p = by_token[point]
            require(p['active'] is False and p['spawn_type_name'] == 'SpawnPointLALODropZone',
                    'Unexpected forward-spawn policy')
            forward.append({'objective_instance_token': inst['name_token'], 'spawn': p,
                            'runtime_policy': 'Unsupported inactive authored LALO; metadata only, not selectable or activated by this projection.'})
    require(len(triggers) == 3 and len(forward) == 4, 'Unexpected authored objective links')
    return {'schema_version': 1, 'format': 'mag_retail_sabotage_objective_metadata',
            'application_id': 20231, 'map': configured[0], 'runtime_indices_verified': False,
            'source': {'game_archive_sha256': GAME_SHA, 'level_archive': LEVEL,
                       'level_archive_sha256': LEVEL_SHA, 'clv_member': CLV,
                       'clv_sha256': CLV_SHA, 'settings_member': '/run/mag/ps3/settings/gameproperties.zpr',
                       'settings_sha256': sha(settings),
                       'projector': 'tools/extraction/project-retail-sabotage-objectives.py',
                       'readers': ['tools/extraction/extract-retail-spawns.py',
                                   'tools/extraction/extract-retail-maps.py',
                                   'tools/extraction/retail_map_helpers.py', 'tools/collision/archive.py']},
            'objectives': graph, 'trigger_links': triggers,
            'script_objects': records['ScriptObjectProperties'],
            'control_argument_bindings': controls,
            'squad_objective_lists': records['SquadObjectiveListPropTable'],
            'platoons': records['PlatoonProperties'],
            'forward_spawn_links': forward, 'original_resources': resources,
            'limitations': ['Static original resource data only; no live gameplay acceptance.',
                'Objective enums remain typed raw original values; no numeric wire-state mapping is supplied.',
                'Authored tokens are not runtime replication pool indices.',
                'Script references/argument bindings do not implement the original Lua or sequence system.',
                'No win/end condition, damage authority, objective activation or LALO support is inferred.',
                'Converted script call/condition lists preserve source order but not branch scopes; original Lua text is not emitted.',
                'Localized-string tokens are retained; displayed translations are not resolved.']}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--disc', type=Path, required=True)
    parser.add_argument('--game-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Require byte-identical existing output; write nothing')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(output != args.game_archive.resolve() and not output.is_relative_to(args.disc.resolve()),
                'Output cannot replace original assets')
        require(args.check or not args.output.exists(), 'Output already exists; use --check')
        result = project(args.disc, args.game_archive)
        encoded = (json.dumps(result, indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            require(args.output.read_bytes() == encoded, 'Objective projection differs')
        else:
            fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(encoded)
        print(json.dumps({'projected': True, 'checked': args.check,
                          'bytes': len(encoded), 'sha256': sha(encoded),
                          'definitions': len(result['objectives']['definitions']),
                          'instances': len(result['objectives']['instances'])}))
    except (ValueError, OSError, KeyError, AssertionError, UnicodeError) as error:
        print(json.dumps({'projected': False, 'failure': str(error)}), file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
