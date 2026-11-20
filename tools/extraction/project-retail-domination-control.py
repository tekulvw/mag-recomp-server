#!/usr/bin/env python3
"""Project the retail Domination callback graph into private controller data."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy

TOOLS = Path(__file__).resolve().parent
IDENT = r'[A-Za-z_]\w*'
STATES = {'Completed': 'completed', 'Locked': 'locked', 'Deactivated': 'deactivated'}
ABILITIES = {'ClusterBomb': 'cluster_bomb', 'GasBombardment': 'gas_bombardment',
             'GuidedArtillery': 'guided_artillery', 'Sensor': 'sensor',
             'PrecisionStrike': 'precision_strike', 'StrafingRun': 'strafing_run'}
EVENTS = {'CompletedScript': 'completed', 'DeactivatedScript': 'deactivated',
          'RepairedScript': 'repaired'}
HASH = re.compile(r'[0-9a-f]{64}')


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def token(value):
    require(re.fullmatch(r'0x[0-9a-fA-F]{8}', value) is not None, 'Invalid authored token')
    result = int(value, 16)
    require(result != 0, 'Unexpected null authored token')
    return result


def parse_conditions(line, params):
    prefix = 'if '
    require(line.startswith(prefix) and line.endswith('then'), 'Unsupported conditional')
    conditions = []
    for part in line[len(prefix):-4].strip().split(' and '):
        match = re.fullmatch(
            r'\(ZScript\.Mission_GetObjectiveState\((' + IDENT +
            r')\)\s*(==|~=)\s*ZScript\.ObjectiveState\.kObjectiveState(\w+)\)', part)
        require(match is not None and match[1] in params and match[3] in STATES,
                'Unsupported objective predicate')
        conditions.append(dict(parameter=match[1], state=STATES[match[3]], equal=match[2] == '=='))
    return conditions


def parse_call(line, params):
    match = re.fullmatch(
        r'ZScript\.Mission_SetObjectiveState\((' + IDENT +
        r'),\s*(ZScript\.Caller|nil),\s*ZScript\.ObjectiveState\.kObjectiveState(\w+)\)', line)
    if match is not None:
        require(match[1] in params and match[3] in STATES, 'Unsupported objective setter')
        return dict(kind='set_state', parameter=match[1], state=STATES[match[3]],
                    caller='original' if match[2] == 'ZScript.Caller' else 'none')
    match = re.fullmatch(r'ZScript\.CommandAbility_(Enable|Disable)_(\w+)\((' + IDENT + r')\)', line)
    if match is not None:
        require(match[2] in ABILITIES and match[3] in params, 'Unsupported ability setter')
        return dict(kind='ability', parameter=match[3], ability=ABILITIES[match[2]], enabled=match[1] == 'Enable')
    match = re.fullmatch(r'ZScript\.Game_SetScoreBleed\((\d+),\s*(\d+),\s*([01])\)', line)
    require(match is not None, 'Unsupported callback statement')
    return dict(kind='score_rate', frequency_ms=int(match[1]) * 1000, amount=int(match[2]),
                team=int(match[3]), objectives=[])


def count_rate(body, params):
    """Recognize the bounded integer count grammar, including parameter order."""
    if not body or not body[0].startswith('local '):
        return None
    joined = '\n'.join(body)
    pattern = (
        r'local (?P<count>' + IDENT + r') = 0\nlocal (?P<rate>' + IDENT + r') = 0\n'
        r'(?P<array>' + IDENT + r') = \{(?P<args>[^{}]+)\}\n'
        r'for ' + IDENT + r', (?P<item>' + IDENT + r') in ipairs\((?P=array)\) do\n'
        r'if ZScript\.Mission_IsObjectiveCompleted\((?P=item)\) then\n'
        r'(?P=count) = (?P=count) \+ 1\nend\nend\n'
        r'(?P=rate) = \((?P=count) \* (?P<factor>\d+)\)\n'
        r'ZScript\.Game_SetScoreBleed\((?P<seconds>\d+),\s*tonumber\((?P=rate)\),\s*(?P<team>[01])\)'
    )
    match = re.fullmatch(pattern, joined)
    require(match is not None, 'Unsupported objective count grammar')
    counted = [s.strip() for s in match['args'].split(',')]
    require(counted == params, 'Unexpected counted parameters')
    return dict(kind='score_rate', frequency_ms=int(match['seconds']) * 1000,
                amount=int(match['factor']), team=int(match['team']), objectives=counted)


def parse_body(body, params):
    score = count_rate(body, params)
    if score is not None:
        return [dict(conditions=[], commands=[score])]
    rules = []
    active = []
    for line in body:
        if line.startswith('if '):
            require(len(active) < 2, 'Conditional nesting outside admitted grammar')
            active.append(parse_conditions(line, params))
        elif line == 'else':
            require(active and len(active[-1]) == 1 and active[-1][0]['equal'], 'Unsupported else')
            active[-1] = [dict(active[-1][0], equal=False)]
        elif line == 'end':
            require(active, 'Unbalanced conditional')
            active.pop()
        else:
            conditions = [condition for group in active for condition in group]
            command = parse_call(line, params)
            if rules and rules[-1]['conditions'] == conditions:
                rules[-1]['commands'].append(command)
            else:
                rules.append(dict(conditions=conditions, commands=[command]))
    require(not active and rules, 'Incomplete callback body')
    return rules


def parse_script(raw):
    require(len(raw) <= 65536, 'Script exceeds parser bound')
    functions = {}
    name, body = None, []
    for original in raw.decode('utf-8').splitlines():
        line = re.sub(r'"(?:[^"\\]|\\.)*"|--.*',
                      lambda m: '' if m[0].startswith('--') else m[0], original).strip()
        if not line or re.fullmatch(r'ZScript\.DebugPrint\("(?:[^"\\]|\\.)*"(?:\s*\.\.\s*'
                + IDENT + r'\s*\.\.\s*"(?:[^"\\]|\\.)*")?\)', line):
            continue
        header = re.fullmatch(r'function\s+(' + IDENT + r')\(([^()]*)\)', line)
        if header:
            if name is not None:
                require(body and body[-1] == 'end', 'Missing callback terminator')
                functions[name] = (params, parse_body(body[:-1], params))
            name, body = header[1], []
            params = [part.strip() for part in header[2].split(',')]
            require(name not in functions and len(params) == len(set(params))
                    and all(re.fullmatch(IDENT, p) for p in params), 'Invalid callback signature')
        else:
            require(name is not None, 'Statement outside callback')
            body.append(line)
    require(name is not None and body and body[-1] == 'end', 'Missing callback terminator')
    functions[name] = (params, parse_body(body[:-1], params))
    return functions


def project_entry(entry, scripts, objectives):
    params, rules = scripts[entry['member']][entry['function']]
    arguments = entry['parameters']
    require(len(arguments) == len(params)
            and all(p['Number'] == 0 for p in arguments), 'Invalid callback arguments')
    bound = dict(zip(params, (token(p['Object']) if p['Object'] else 0 for p in arguments)))

    def objective(parameter):
        value = bound[parameter]
        require(value in objectives, 'Script references an unplaced objective')
        return value

    def command(row):
        data = row.copy()
        if row['kind'] == 'score_rate':
            data['objectives'] = [objective(p) for p in row['objectives']]
        elif row['kind'] == 'set_state':
            data['objective_token'] = objective(data.pop('parameter'))
        else:
            data['platoon_token'] = bound[data.pop('parameter')]
        return data

    delay = entry['delay_ms']
    require(type(delay) is int and 0 <= delay <= 60000, 'Invalid callback delay')
    return dict(delay_ms=delay, rules=[
        dict(conditions=[dict(objective_token=objective(c['parameter']), state=c['state'], equal=c['equal'])
                         for c in rule['conditions']],
             commands=[command(c) for c in rule['commands']])
        for rule in rules])


def project_map(row, scripts):
    objectives = {token(o['name_token']) for o in row['objectives']['instances']}
    require(len(objectives) == len(row['objectives']['instances']), 'Duplicate objective token')
    bindings = {}
    for b in row['script_bindings']:
        key = token(b['instance_token'])
        require(key not in bindings, 'Duplicate binding token')
        immediate = [b['immediate']] if b['immediate'] is not None else []
        require(all(e['delay_ms'] == 0 for e in immediate)
                and all(e['delay_ms'] > 0 for e in b['delayed']), 'Invalid registration delay')
        entries = [project_entry(e, scripts, objectives) for e in immediate + b['delayed']]
        require(entries, 'Empty controller binding')
        bindings[key] = dict(binding_token=key, entries=entries)

    definitions = {d['definition_token']: d for d in row['objectives']['definitions']}
    expected_hooks = {(token(i['name_token']), EVENTS[name], token(field['token']))
                      for i in row['objectives']['instances']
                      for name, field in definitions[i['definition_token']]['fields'].items()
                      if name.endswith('Script') and field['token'] != '0x00000000'}
    hooks = {}
    for hook in row['objective_script_hooks']:
        key = (token(hook['objective_token']), EVENTS[hook['property']])
        target = token(hook['script_instance_token'])
        resolved = target in bindings
        require(key[0] in objectives and key not in hooks
                and hook['binding_present'] is resolved, 'Invalid objective hook')
        hooks[key] = dict(objective_token=key[0], event=key[1], binding_token=target, resolved=resolved)
    require({(*key, hook['binding_token']) for key, hook in hooks.items()} == expected_hooks,
            'Objective hook does not match authored fields')
    require({h['binding_token'] for h in hooks.values() if h['resolved']} == set(bindings),
            'Unclassified controller registration')
    return dict(name=row['map']['map'], level_archive_sha256=row['source']['archive_sha256'],
                level_clv_sha256=row['source']['clv_sha256'], bindings=list(bindings.values()),
                hooks=list(hooks.values()))


def parse_members(archive, fingerprints):
    require(0 < len(fingerprints) <= 16, 'Invalid script source count')
    scripts = {}
    for member, digest in fingerprints.items():
        require(archive.names.count(member) == 1 and HASH.fullmatch(digest), 'Invalid script member')
        script = archive.extract(archive.names.index(member) + 1)
        require(sha(script) == digest, 'Retail callback fingerprint mismatch')
        scripts[member] = parse_script(script)
    return scripts


def render(metadata_path, game_path):
    metadata_bytes = metadata_path.read_bytes()
    require(len(metadata_bytes) <= 16 * 1024 * 1024, 'Metadata exceeds parser bound')
    metadata = json.loads(metadata_bytes)
    require(metadata['format'] == 'mag_retail_domination_map_metadata'
            and metadata['schema_version'] == 1 and metadata['mode_enum'] == 'GameType256',
            'Require retail Domination metadata')
    builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    raw = game_path.read_bytes()
    game_hash = sha(raw)
    require(game_hash == builder['GAME_ARCHIVE_SHA256'] == metadata['source_archive_sha256'],
            'Retail game archive fingerprint mismatch')
    archive = builder['extractor']().Psarc(raw)
    fingerprints = metadata['script_sources_sha256']
    scripts = parse_members(archive, fingerprints)
    maps = [project_map(row, scripts) for row in metadata['maps']]
    require(len(maps) == 3 and len({m['name'] for m in maps}) == 3
            and all(HASH.fullmatch(m[k]) for m in maps
                    for k in ('level_archive_sha256', 'level_clv_sha256')), 'Invalid retail map set')
    return dict(format='mag_domination_control', schema_version=1,
                game_archive_sha256=game_hash, metadata_sha256=sha(metadata_bytes),
                script_sha256=list(fingerprints.values()), maps=maps)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('metadata', 'game-archive', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if (args.output.resolve().is_relative_to(TOOLS.parents[1])
            or args.output.resolve() in {p.resolve() for p in (args.metadata, args.game_archive)}):
        parser.error('Output must be private and outside the source checkout and inputs')
    try:
        raw = (json.dumps(render(args.metadata, args.game_archive), indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            require(args.output.read_bytes() == raw, 'Output differs from source reproduction')
        else:
            with args.output.open('xb') as output:
                output.write(raw)
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.exit(1, str(error) + '\n')


if __name__ == '__main__':
    main()
