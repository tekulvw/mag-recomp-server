#!/usr/bin/env python3
"""Project external Acquisition controller bindings without executing Lua.

The restricted parser accepts ordered state-condition groups and explicit
objective/damage-volume calls. Trailing calls remain unbound; unsupported grammar
is an error. The supplied proof is externally qualified evidence, not certified
by this projector's hash/coherence checks.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy

TOOLS = Path(__file__).resolve().parent
GAME_SHA = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
IDENT = r'[A-Za-z_][A-Za-z_0-9]*'
CONDITION = re.compile(r'(if|elseif)\s*\(\s*ZScript\.Mission_GetObjectiveState\(\s*('
                       + IDENT + r')\s*\)\s*==\s*ZScript\.ObjectiveState\.kObjectiveState'
                       r'(Completed|Deactivated)\s*\)\s*then')
OPERATIONS = {'Mission_LockObjective': 'lock', 'Mission_EnableObjective': 'enable',
              'Mission_DisableObjective': 'disable'}


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def digest(value):
    require(isinstance(value, str) and re.fullmatch('[0-9a-f]{64}', value) is not None,
            'Invalid source fingerprint')
    return value


def token(value):
    if isinstance(value, str):
        require(re.fullmatch('0x[0-9a-fA-F]{1,8}', value) is not None, 'Invalid token')
        value = int(value, 16)
    require(type(value) is int and 0 < value < 2**32, 'Invalid token')
    return value


def integer(value):
    require(re.fullmatch(r'-?\d+', value) is not None, 'Expected literal integer argument')
    result = int(value)
    require(-2**31 <= result < 2**31, 'Argument outside i32')
    return result


def parse_script(raw):
    """Parse a small explicit grammar, never evaluate or translate arbitrary Lua."""
    require(isinstance(raw, bytes) and len(raw) <= 65_536, 'Script exceeds bound')
    parameters, function, groups, unbound = None, None, [], []
    group, branch, finished = None, None, False
    for original in raw.decode('utf-8').splitlines():
        # Preserve quoted debug text while removing comments outside strings.
        line = re.sub(r'"(?:[^"\\]|\\.)*"|--.*',
                      lambda m: '' if m[0].startswith('--') else m[0], original).strip()
        if not line:
            continue
        require(not finished, 'Content after function end')
        header = re.fullmatch(r'function\s+(' + IDENT + r')\(([^()]*)\)', line)
        if header:
            require(parameters is None, 'Only one function is supported')
            function = header[1]
            parameters = [v.strip() for v in header[2].split(',')]
            require(len(parameters) == 12 and len(set(parameters)) == 12
                    and all(re.fullmatch(IDENT, v) for v in parameters), 'Expected twelve distinct parameters')
            continue
        require(parameters is not None, 'Statement outside function')
        condition = CONDITION.fullmatch(line)
        if condition:
            require(not unbound and condition[2] in parameters, 'Invalid condition parameter/order')
            if condition[1] == 'if':
                require(group is None, 'Nested condition unsupported')
                group = dict(branches=[])
                groups.append(group)
            else:
                require(group is not None, 'elseif outside group')
            branch = dict(condition=dict(parameter=condition[2], state=condition[3].lower()), commands=[])
            group['branches'].append(branch)
            continue
        if line == 'end':
            if group is not None:
                group, branch = None, None
            else:
                finished = True
            continue
        if re.fullmatch(r'ZScript\.DebugPrint\("(?:[^"\\]|\\.)*"\)', line):
            continue
        call = re.fullmatch(r'ZScript\.(' + IDENT + r')\(([^()]*)\)', line)
        require(call is not None, 'Unsupported script statement: ' + line[:100])
        name, arguments = call[1], [v.strip() for v in call[2].split(',')]
        if group is None:
            require(len(groups) == 4, 'Only trailing unbound calls are supported')
            unbound.append(dict(symbol=name, arguments=[integer(v) for v in arguments]))
        elif name in OPERATIONS:
            require(len(arguments) == 1 and arguments[0] in parameters, 'Invalid objective call')
            branch['commands'].append(dict(kind='objective', parameter=arguments[0], operation=OPERATIONS[name]))
        elif name == 'DamageVolume':
            require(len(arguments) == 7 and arguments[0] in parameters, 'Invalid volume call')
            require(all(re.fullmatch(r'ZScript\.' + IDENT + r'(?:\.' + IDENT + r')+', v)
                        for v in arguments[5:]), 'Invalid damage symbol')
            branch['commands'].append(dict(kind='damage_volume', parameter=arguments[0],
                arguments=[integer(v) for v in arguments[1:5]],
                damage_type_symbol=arguments[5][8:], damage_source_symbol=arguments[6][8:]))
        else:
            raise ValueError('Unsupported conditional call: ' + name)
    require(finished and group is None and len(groups) == 4, 'Incomplete four-group function')
    require(len(unbound) <= 8 and all(len(v['arguments']) <= 8 for v in unbound), 'Unbound call limit')
    for item in groups:
        require(0 < len(item['branches']) <= 3, 'Branch count limit')
        predicates = [(v['condition']['parameter'], v['condition']['state']) for v in item['branches']]
        require(len(set(predicates)) == len(predicates), 'Duplicate condition')
        require(all(0 < len(v['commands']) <= 8 for v in item['branches']), 'Command count limit')
    return dict(function=function, parameters=parameters, groups=groups, unbound_calls=unbound)


def project(spawn_bytes, proof_bytes, script_bytes):
    require(len(spawn_bytes) <= 1_048_576 and len(proof_bytes) <= 2_097_152, 'Input exceeds bound')
    spawn, proof = json.loads(spawn_bytes), json.loads(proof_bytes)
    require(proof['schema'] == 'mag_acquisition_arrival_adjacent_frontier_v1'
            and proof['status'] == 'ALL_THREE_MAP_SCRIPT_AND_ESCORT_LINKS_BOUND_RETAIL_ARRIVAL_GUARDS_PARTIAL',
            'Unsupported graph proof')
    require(spawn['format'] == 'mag_spawn_catalog' and spawn['schema_version'] == 1
            and len(spawn['maps']) == 1 and spawn['match_profile']['game_type'] == 1, 'Expected Acquisition spawn catalog')
    source = json.loads(spawn['source'])
    map_name = spawn['maps'][0]['name']
    require(spawn['active']['map_name'] == source['map'] == map_name, 'Spawn map mismatch')
    selected = [m for m in proof['maps'] if m['map']['map'] == map_name]
    require(len(selected) == 1, 'Require one selected proof map')
    selected = selected[0]
    require(selected['map']['mode_enum'] == 'GameType128', 'Wrong authored mode')
    require(source['game_archive_sha256'] == digest(proof['original_game_sha256'])
            and source['level_archive_sha256'] == digest(selected['source']['archive_sha256'])
            and source['level_clv_sha256'] == digest(selected['source']['clv_sha256']), 'Original source mismatch')
    script_hash = digest(proof['scripts']['gametype_128.lua']['sha256'])
    require(sha(script_bytes) == script_hash, 'Script fingerprint mismatch')
    parsed = parse_script(script_bytes)
    parameters = selected['parameters']
    require(parsed['function'] == selected['script_function'] and len(parameters) == 12
            and [p['index'] for p in parameters] == list(range(12))
            and parsed['parameters'] == [p['label'] for p in parameters], 'Script parameter mismatch')
    symbols = {p['label']: token(p['object_token']) for p in parameters}
    controller = token(selected['script_instance_name_token'])
    chains, objective_tokens, volume_tokens = [], set(), set()
    require(len(selected['chains']) == 4, 'Expected four chains')
    point_tokens = {token(p['token']) for p in spawn['maps'][0]['locations']}
    all_tokens = {controller}
    for i, chain in enumerate(selected['chains']):
        require(chain['chain'] == str(i//2+1) + ('a' if i%2 == 0 else 'b'), 'Invalid chain order/address')
        capture, arrival, vehicle, volume = [token(chain[k]) for k in
            ('spawn_objective_token', 'arrival_objective_token', 'vehicle_spawn_token', 'kill_volume_token')]
        require([capture, arrival] == [token(p['object_token']) for p in parameters[i*2:i*2+2]]
                and volume == token(parameters[8+i]['object_token']), 'Chain parameter mismatch')
        require(vehicle in point_tokens, 'Missing escort spawn point')
        require(chain['spawn_definition']['class_name'] == 'ObjectiveSecureAndHoldProperties'
                and chain['arrival_definition']['class_name'] == 'ObjectiveArrivalProperties', 'Wrong objective classes')
        sf, af = chain['spawn_definition']['fields'], chain['arrival_definition']['fields']
        require(token(sf['ForwardSpawnPoint']['token']) == vehicle == token(af['VehicleSpawnPoint']['token']),
                'Capture/Arrival vehicle join mismatch')
        require(all(token(fields[hook]['token']) == controller for fields in (sf, af)
                    for hook in ('CompletedScript', 'DeactivatedScript')), 'Controller event hook mismatch')
        require(all(t not in all_tokens for t in (capture, arrival, vehicle, volume))
                and len({capture, arrival, vehicle, volume}) == 4, 'Duplicate graph identity')
        all_tokens.update((capture, arrival, vehicle, volume))
        objective_tokens.update((capture, arrival)); volume_tokens.add(volume)
        chains.append(dict(platoon=i//2, stage='a' if i%2 == 0 else 'b', capture_token=capture,
                           arrival_token=arrival, vehicle_spawn_token=vehicle, volume_token=volume))
    groups = []
    for group in parsed['groups']:
        branches = []
        for branch in group['branches']:
            condition = branch['condition']
            target = symbols[condition['parameter']]
            require(target in objective_tokens, 'Condition addresses a nonobjective')
            commands = []
            for command in branch['commands']:
                command = command.copy(); target = symbols[command.pop('parameter')]
                require(target in (objective_tokens if command['kind'] == 'objective' else volume_tokens),
                        'Command target family mismatch')
                commands.append(dict(command, target=target))
            branches.append(dict(condition=dict(objective_token=symbols[condition['parameter']], state=condition['state']), commands=commands))
        groups.append(dict(branches=branches))
    return dict(format='mag_acquisition_control', schema_version=1, controller_token=controller,
        source=dict(map_name=map_name, spawn_catalog_sha256=sha(spawn_bytes),
                    game_archive_sha256=proof['original_game_sha256'], script_sha256=script_hash, graph_proof_sha256=sha(proof_bytes)),
        chains=chains, groups=groups, unbound_calls=parsed['unbound_calls'])


def render(spawns, proof, game):
    raw = game.read_bytes()
    require(sha(raw) == GAME_SHA, 'Wrong retail game archive')
    proof_bytes = proof.read_bytes()
    evidence = json.loads(proof_bytes)
    require(evidence['original_game_sha256'] == GAME_SHA, 'Graph game archive mismatch')
    builder = runpy.run_path(str(TOOLS/'build-retail-spawn-catalog.py'))
    archive = builder['extractor']().Psarc(raw)
    member = evidence['scripts']['gametype_128.lua']['member']
    require(archive.names.count(member) == 1, 'Missing/ambiguous original script member')
    script = archive.extract(archive.names.index(member)+1)
    return project(spawns.read_bytes(), proof_bytes, script)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('spawn-catalog', 'graph-proof', 'game-archive', 'output'):
        parser.add_argument('--'+name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    require(args.output.resolve() not in {p.resolve() for p in
            (args.spawn_catalog, args.graph_proof, args.game_archive)}, 'Output must not overwrite an input')
    try:
        raw = encode(render(args.spawn_catalog, args.graph_proof, args.game_archive))
        if args.check:
            require(args.output.read_bytes() == raw, 'Controller differs from source reproduction')
        else:
            args.output.write_bytes(raw)
    except (KeyError, TypeError, ValueError) as error:
        parser.exit(1, str(error)+'\n')


if __name__ == '__main__':
    main()
