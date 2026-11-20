#!/usr/bin/env python3
"""Project authored Domination metadata from retail files you are entitled to use."""

import argparse
import json
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent
MODE = 'GameType256'
SCRIPT_ROOT = '/run/mag/ps3/scripts/'
OBJECTIVE_FIELDS = frozenset(    """Active InitialActiveState AdvanceOnLockOnly CanDeactivate CanRepair Category
    SubType OwnerTeam DisplayID DisplayName NameTag FRAGOBonusRange FSPType ForwardSpawnPoint
    ActivationScoreValue DeactivationScoreValue TimeToActivateMs TimeToCompleteMs
    TimeToDeactivateMs MinTimeToRepairMs TimeToRepairMs TimeToArmMs TimeToDefuseMs
    TimeToDetonateMs DamageValue FullDamageRadius HalfDamageRadius MaterialPenetration
    CharacterDamageOverTimeRadius CharacterDamageOverTimeValue DamageThreshold RepairValue
    ObjectiveID ObjectiveTriggerVolume TriggerType StartActive UsesNonUniformScale""".split()
)


def tool(name):
    return runpy.run_path(str(TOOLS / name))


def index(rows, field):
    result = {}
    for row in rows:
        key = row[field]
        if key in result:
            raise ValueError(f'Duplicate {field}: {key}')
        result[key] = row
    return result


def squad_links(points):
    squads = {}
    labels = ('Alpha', 'Bravo', 'Charlie', 'Delta')
    for token, point in points.items():
        if point['team'] not in (0, 1):
            raise ValueError(f'Invalid spawn team for {token}')
        for link in point['squad_links']:
            team, platoon, squad, slot = (link[k] for k in ('team', 'platoon_id', 'squad', 'slot'))
            if team != point['team'] or not 0 <= platoon < 4 or squad not in labels or slot < 0:
                raise ValueError(f'Invalid squad link for {token}')
            slots = squads.setdefault((team, platoon, labels.index(squad)), {})
            if slot in slots:
                raise ValueError(f'Duplicate squad slot for {token}')
            slots[slot] = token
    result = []
    for (team, platoon, squad), slots in sorted(squads.items()):
        if sorted(slots) != list(range(len(slots))):
            raise ValueError(f'Missing squad slot for team {team}, platoon {platoon}, squad {squad}')
        result.append({'team': team, 'platoon': platoon, 'squad': squad,
                       'spawns': [slots[slot] for slot in range(len(slots))]})
    return result


def project_map(row, volumes):
    source = row['source']
    metadata = row['map']
    if (metadata['mode_enum'] != MODE or metadata['map'] != volumes['map']
            or any(source[k] != volumes['source'][k]
                   for k in ('archive_sha256', 'clv_member', 'clv_sha256'))):
        raise ValueError('Map and trigger source mismatch')
    if row['unresolved_squad_references']:
        raise ValueError(f'Unresolved authored squad references on {metadata["map"]}')

    points = index(row['points'], 'name_token')
    graph = row['objectives']
    definitions = index(graph['definitions'], 'definition_token')
    instances = index(graph['instances'], 'name_token')
    forward_spawns = []
    placed_definitions = {}
    for token, instance in instances.items():
        definition_token = instance['definition_token']
        definition = definitions[definition_token]
        if definition_token not in placed_definitions:
            placed_definitions[definition_token] = {
                **definition,
                'fields': {name: value for name, value in definition['fields'].items()
                           if name in OBJECTIVE_FIELDS or name.endswith('Script')},
            }
        forward = definition['fields'].get('ForwardSpawnPoint')
        if forward is None or forward['token'] == '0x00000000':
            continue
        spawn = forward['token']
        if spawn not in points:
            raise ValueError(f'Objective {token} names missing forward spawn {spawn}')
        forward_spawns.append({'objective_token': token, 'spawn_token': spawn})

    return {
        'map': metadata,
        'source': source,
        'spawns': list(points.values()),
        'squads': squad_links(points),
        'bunker_candidates': row['bunkers'],
        'objectives': {'definitions': list(placed_definitions.values()),
                       'instances': list(instances.values())},
        'forward_spawns': forward_spawns,
        'objective_trigger_volumes': volumes['objectives'],
        'runtime_pool_indices_verified': False,
    }


def script_bindings(clv, helpers, archive, digests):
    """Extract placed callback registrations and hash the referenced script members."""
    helpers['register'](clv, 'ScriptObjectProperties')
    values = helpers['values']
    instances = {}
    for row in clv.tables('InstanceProperties'):
        instances.setdefault(row['preceding_token'], []).append(row)

    def entry(script, function, params, delay_ms):
        member = SCRIPT_ROOT + script
        if not function or member not in archive.names:
            raise ValueError(f'Unresolved server script entry: {member}')
        if member not in digests:
            raw = archive.extract(archive.names.index(member) + 1)
            digests[member] = helpers['sha'](raw)
        return {'member': member, 'function': function, 'delay_ms': delay_ms,
                'parameters': [values(p['fields']) for p in params]}

    bindings = []
    for table in clv.tables('ScriptObjectProperties'):
        placed = instances.get(table['name_token'], [])
        if not placed:
            continue
        fields = values(helpers['decoded_fields'](clv, table['fields']))
        immediate = None
        if fields['ServerScript'] or fields['ServerFunction']:
            immediate = entry(fields['ServerScript'], fields['ServerFunction'],
                              fields['ServerParams'], 0)
        delayed = []
        for record in fields['ServerEntryPoints']:
            field = values(record['fields'])
            delayed.append(entry(field['Script'], field['Function'],
                                 field['Params'], field['TimeDelay']))
        if immediate is None and not delayed:
            continue
        for instance in placed:
            bindings.append({'instance_token': values(instance['fields'])['Name'],
                             'definition_token': f"0x{table['name_token']:08x}",
                             'definition_sha256': table['source_sha256'],
                             'immediate': immediate, 'delayed': delayed})
    index(bindings, 'instance_token')
    return bindings


def read_script_bindings(disc, row, spawns, helpers, archive, digests):
    source = row['source']
    raw = (disc / source['archive']).read_bytes()
    if helpers['sha'](raw) != source['archive_sha256']:
        raise ValueError('Script binding level archive source mismatch')
    level = spawns.Psarc(raw)
    data = level.extract(level.names.index(source['clv_member']) + 1)
    if helpers['sha'](data) != source['clv_sha256']:
        raise ValueError('Script binding CLV source mismatch')
    bindings = script_bindings(spawns.retail_clv(data), helpers, archive, digests)
    by_token = index(bindings, 'instance_token')
    hooks = []
    definitions = index(row['objectives']['definitions'], 'definition_token')
    for instance in row['objectives']['instances']:
        definition = definitions[instance['definition_token']]
        for name, field in definition['fields'].items():
            if not name.endswith('Script') or field['token'] == '0x00000000':
                continue
            target = field['token']
            hooks.append({'objective_token': instance['name_token'], 'property': name,
                          'script_instance_token': target, 'binding_present': target in by_token})
    return bindings, hooks


def extract(disc, game_archive):
    builder = tool('build-retail-spawn-catalog.py')
    game = game_archive.read_bytes()
    game_hash = builder['sha'](game)
    if game_hash != builder['GAME_ARCHIVE_SHA256']:
        raise ValueError('Wrong original retail game.psarc')
    spawns = builder['extractor']()
    archive = spawns.Psarc(game)
    settings_member = '/run/mag/ps3/settings/gameproperties.zpr'
    settings = archive.extract(archive.names.index(settings_member) + 1)
    rules, provenance = builder['original_rules'](settings, MODE)

    map_data = tool('extract-retail-maps.py')['extract'](disc, game_archive, mode=MODE)
    maps = map_data['maps']
    owners = sorted(row['map']['owner_faction'] for row in maps)
    if owners != sorted(builder['FACTIONS']):
        raise ValueError('Require exactly one authored Domination map per faction')
    names = [row['map']['map'] for row in maps]
    volume_data = tool('extract-retail-objective-volumes.py')['extract'](disc, only=names)
    volumes = index(volume_data['maps'], 'map')
    script_helper = tool('project-retail-sabotage-objectives.py')
    script_sources = {}
    projected = []
    for row in maps:
        level = project_map(row, volumes[row['map']['map']])
        level['script_bindings'], level['objective_script_hooks'] = read_script_bindings(
            disc, row, spawns, script_helper, archive, script_sources,
        )
        projected.append(level)
    return {
        'format': 'mag_retail_domination_map_metadata',
        'schema_version': 1,
        'source_archive_sha256': game_hash,
        'mode_enum': MODE,
        'authored_rules': rules,
        'rules_source': provenance,
        'bunker_property_sources': map_data['property_sources'],
        'script_sources_sha256': script_sources,
        'maps': projected,
        'limitations': [
            'Authored metadata. Squad slots and object tokens are not runtime pool IDs.',
            'Authored player bounds are not server admission limits.',
            'Bunker exits are candidates without runtime floor or clearance validation.',
            'Trigger box order does not establish native trigger indices.',
            'Script registrations describe authored calls; runtime invocation and effects are unverified.',
            'This format does not enable a worker mode or implement match logic.',
        ],
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--disc', type=Path, required=True, help='Retail dump containing PS3_GAME')
    parser.add_argument('--game-archive', type=Path, required=True, help='Decrypted retail game.psarc')
    parser.add_argument('--output', type=Path, required=True, help='Private JSON outside the source checkout')
    parser.add_argument('--check', action='store_true', help='Compare with existing output without writing')
    args = parser.parse_args()

    output = args.output.resolve()
    if (output == args.game_archive.resolve() or output.is_relative_to(args.disc.resolve())
            or output.is_relative_to(TOOLS.parents[1])):
        parser.error('Output must be outside the original inputs and source checkout')
    if not args.check and args.output.exists():
        parser.error('Refusing existing output. Use --check to reproduce it.')
    try:
        data = extract(args.disc, args.game_archive)
        raw = (json.dumps(data, indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            if args.output.read_bytes() != raw:
                raise ValueError('Domination metadata differs from retail regeneration')
        else:
            with args.output.open('xb') as stream:
                stream.write(raw)
    except (KeyError, ValueError, OSError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')
    print(f'{"Verified" if args.check else "Extracted"} {len(data["maps"])} Domination maps')


if __name__ == '__main__':
    main()
