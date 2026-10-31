#!/usr/bin/env python3
"""Build a runtime spawn catalog for retail Suppression or SVER Sabotage entry.

Reads the original retail disc dump and the decrypted retail game.psarc directly,
through the tracked extract-retail-spawns.py parser, and writes the Rust
SpawnCatalog schema. Never writes next to the game files. Positions are the
authored pre-terrain-snap coordinates; runtime pool indices are NOT verified for retail.
"""
import argparse
import hashlib
import importlib.util
import json
import runpy
from pathlib import Path

TOOLS = Path(__file__).resolve().parent
GAME_ARCHIVE_SHA256 = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
SETTINGS_SHA256 = 'ed3b62d496bb0f36d53ec9a2ec3b29cae930f9e714af41e8c828f3c2dd9ed922'

FACTIONS = {'Valor': 0, 'Raven': 1, 'SVER': 2}
KINDS = {0: 'insertion', 1: 'alternate'}
SQUADS = {'Alpha': 0, 'Bravo': 1, 'Charlie': 2, 'Delta': 3}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def extractor():
    spec = importlib.util.spec_from_file_location('extract_retail_spawns', TOOLS / 'extract-retail-spawns.py')
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def original_rules(settings, mode_enum):
    """Parse the original literal table; never execute authored Lua/ZPR."""
    if sha(settings) != SETTINGS_SHA256:
        raise SystemExit('Wrong retail gameproperties.zpr')
    parser = runpy.run_path(str(TOOLS / 'extract-item-catalog.py'))
    properties = parser['normalize'](parser['Parser'](settings.decode(), 'gameproperties.zpr').parse())
    types = properties['GameCreationPropertiesTable']['m_gameCreationTypes']
    matches = [(key, wrapper['GameCreationType']) for key, wrapper in types.items()
               if wrapper['GameCreationType']['m_gameType'] == mode_enum]
    if len(matches) != 1:
        raise SystemExit('Require one original selected mode rule table')
    key, creation = matches[0]
    rules = creation['m_gameRules']
    points = [rules[f'StartingPointsTeam{team}'] for team in (0, 1)]
    waves = [rules[f'SpawnIntervalTeam{team}'] for team in (0, 1)]
    initial = [rules[f'SpawnInitialDelayTeam{team}'] for team in (0, 1)]
    if (mode_enum not in ('GameTypeFactionExercise', 'GameType64')
            or any(type(value) is not int or not 0 <= value < 2**32 for value in points + waves + initial)
            or any(value <= 0 for value in points + waves)
            or type(creation.get('m_minPlayers')) is not int
            or type(creation.get('m_maxPlayers')) is not int
            or not 0 <= creation['m_minPlayers'] <= creation['m_maxPlayers'] <= 256):
        raise SystemExit('Unsupported or malformed authored mode rules')
    profile = {'maximum_points': points, 'spawn_wave_ms': waves}
    if mode_enum == 'GameType64':
        profile['initial_spawn_delay_ms'] = initial
    return profile, {
        'member': '/run/mag/ps3/settings/gameproperties.zpr',
        'sha256': sha(settings),
        'rules_json_pointer': f'/GameCreationPropertiesTable/m_gameCreationTypes/{key}/GameCreationType/m_gameRules',
        'authored_maximum_players': creation['m_maxPlayers'],
        'authored_minimum_players': creation['m_minPlayers'],
        'literal_parser': 'tools/extraction/extract-item-catalog.py',
    }


def faction_exercise_rules(settings):
    return original_rules(settings, 'GameTypeFactionExercise')


def extract(disc, game_archive, map_name):
    """Run the tracked retail extractor for one map straight from the original files."""
    x = extractor()
    game = game_archive.read_bytes()
    if sha(game) != GAME_ARCHIVE_SHA256:
        raise SystemExit('Wrong retail game.psarc')
    arc = x.Psarc(game)
    settings = arc.extract(arc.names.index('/run/mag/ps3/settings/gameproperties.zpr') + 1)
    info = {m['map']: m for m in x.catalog_settings(settings.decode())}.get(map_name)
    if info is None or not (info['mode_enum'] == 'GameTypeFactionExercise'
                            or (map_name == 'mag_cosmodrome_final' and info['mode_enum'] == 'GameType64')):
        raise SystemExit('Map is not a supported Suppression/SVER Sabotage entry level')
    match_rules, rules_source = original_rules(settings, info['mode_enum'])
    level = disc / 'PS3_GAME/USRDIR/run/mag/ps3/levels' / (map_name + '.psarc')
    points = x.extract_map(level, info)
    if info['mode_enum'] == 'GameType64':
        # Reuse the maintained objective parser to document disabled authored content.
        maps = runpy.run_path(str(TOOLS / 'extract-retail-maps.py'))
        helpers = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
        archive = x.Psarc(level.read_bytes())
        data = archive.extract(archive.names.index(points['source']['clv_member']) + 1)
        points['objectives'] = maps['objective_graph'](data, helpers)
    # Same grouping as extract-retail-spawns.py's squad-spawns.json for this map.
    grouped = {}
    for p in points['points']:
        for ref in p['squad_links']:
            grouped.setdefault((ref['team'], ref['platoon_id'], ref['squad']), []).append((ref['slot'], p))
    groups = [{'team': t, 'platoon_id': pl, 'squad': sq, 'points': [p for _, p in sorted(v, key=lambda e: e[0])]}
              for (t, pl, sq), v in sorted(grouped.items())]
    return points, groups, sha(game), match_rules, rules_source


def build(disc, game_archive, map_name, active_team, active_platoon, active_squad):
    points, groups, game_sha, match_rules, rules_source = extract(disc, game_archive, map_name)
    owner = points['map']['owner_faction']
    sabotage = points['map']['mode_enum'] == 'GameType64'
    if owner not in FACTIONS or any(p['team'] not in (0, 1)
            or p['faction'] != (None if sabotage and p['team'] == 0 else owner)
            for p in points['points']):
        raise SystemExit('Map has inconsistent authored side/faction bindings')
    owner = FACTIONS[owner]
    if sabotage and (map_name != 'mag_cosmodrome_final' or owner != FACTIONS['SVER']):
        raise SystemExit('Sabotage entry policy requires SVER Syr Darya')
    index = {}
    locations = []
    unsupported_links = []
    for i, p in enumerate(points['points']):
        if p['catalog_id'] != f'{map_name}:{i:03d}' or p['user_class'] != 0:
            raise SystemExit('Unsupported retail spawn identity ' + p['catalog_id'])
        basic = p['spawn_type_name'] == 'SpawnPointBasic' and p['category'] in KINDS
        inactive_lalo = (sabotage and p['spawn_type_name'] == 'SpawnPointLALODropZone'
                         and p['spawn_type'] == 5 and p['category'] == 2 and p['active'] is False)
        if not basic and not inactive_lalo:
            raise SystemExit('Unsupported retail spawn point ' + p['catalog_id'])
        if inactive_lalo:
            unsupported_links.append({'index': i, 'catalog_id': p['catalog_id'],
                                      'spawn_type': p['spawn_type'], 'category': p['category'],
                                      'active': False, 'authored_squad_links': p['squad_links']})
        index[p['catalog_id']] = i
        x, y, z = p['position_xyz']
        locations.append({'index': i, 'token': int(p['name_token'], 16),
                          'kind': KINDS[p['category']] if basic else 'unsupported',
                          'active': p['active'], 'position': {'x': x, 'y': y, 'z': z}})
    squads = []
    for g in groups:
        available = [index[p['catalog_id']] for p in g['points']
                     if p['active'] and locations[index[p['catalog_id']]]['kind'] != 'unsupported']
        defaults = [index[p['catalog_id']] for p in g['points']
                    if p['category'] == 0 and index[p['catalog_id']] in available]
        if len(defaults) != 1:
            raise SystemExit('Squad requires one supported active insertion default')
        default = defaults[0]
        squads.append({'index': g['team'] * 16 + g['platoon_id'] * 4 + SQUADS[g['squad']], 'team': g['team'],
                       'platoon': g['platoon_id'], 'squad': SQUADS[g['squad']],
                       'faction': owner if g['team'] == 1 else None, 'available': available, 'default_spawn': default})
    squads.sort(key=lambda s: s['index'])
    active = next(s for s in squads if (s['team'], s['platoon'], s['squad']) == (active_team, active_platoon, active_squad))
    source = {'extractor': 'tools/extraction/extract-retail-spawns.py', 'map': map_name, 'game_archive_sha256': game_sha,
              'level_archive_sha256': points['source']['archive_sha256'], 'level_clv_sha256': points['source']['clv_sha256'],
              'game_rules_source': rules_source,
              'note': 'Authored pre-terrain-snap coordinates; retail runtime pool indices unverified'}
    if sabotage:
        graph = points['objectives']
        if len(graph['definitions']) != 13 or len(graph['instances']) != 13:
            raise SystemExit('Original Syr Darya objective cardinality changed')
        source.update(unsupported_spawn_links=unsupported_links, authored_objectives={
            'definitions': len(graph['definitions']), 'direct_instances': len(graph['instances']),
            'catalog': 'gameplay/data/retail-maps.json', 'map': map_name,
            'runtime_policy': 'Disabled for entry foundation; authored objectives are present, runtime identities/control unresolved'})
    # Original-proven modes: Faction Exercise3, Sabotage0. No vehicle spawns
    # are supported here. Sabotage objectives are explicitly disabled above;
    # Faction Exercise objectives remain an unverified absence policy.
    # Same-faction Faction Exercise and Valor-versus-SVER Sabotage are distinct
    # local policies; map ownership alone never supplies Sabotage's attacker.
    # Admission limits/automatic balancing below are explicit local/user policy;
    # they are not new property fields recovered from the level archive. Keep
    # them in the generated runtime configuration rather than Rust constants.
    profile = {'game_type': 0 if sabotage else 3,
               'attacking_faction': FACTIONS['Valor'] if sabotage else owner,
               'auto_assign_teams': True, 'team_capacity': 32, 'squad_capacity': 8,
               'maximum_points': match_rules['maximum_points'],
               'spawn_wave_ms': match_rules['spawn_wave_ms'],
               'has_bunkers': False, 'has_objectives': False}
    if sabotage:
        profile['initial_spawn_delay_ms'] = match_rules['initial_spawn_delay_ms']
    if len(json.dumps(source, sort_keys=True)) >= 8192:
        raise SystemExit('Runtime spawn provenance exceeds source field bound')
    return {'format': 'mag_spawn_catalog', 'schema_version': 1, 'source': json.dumps(source, sort_keys=True),
            'match_profile': profile,
            'active': {'map_name': map_name, 'squad_index': active['index'], 'default_spawn': active['default_spawn'], 'available': active['available']},
            'maps': [{'name': map_name, 'spawn_count': len(locations), 'locations': locations, 'owner_faction': owner,
                      'runtime_indices_verified': False, 'squads': squads}]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--disc', type=Path, required=True, help='original retail disc dump directory containing PS3_GAME')
    parser.add_argument('--game-archive', type=Path, required=True, help='decrypted retail game.psarc (sha256 b49c9435...)')
    parser.add_argument('--map', required=True)
    parser.add_argument('--active', required=True, help='team/platoon/squad index for the local player')
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true', help='Fail unless OUTPUT already matches')
    args = parser.parse_args()
    if args.output.resolve() == args.game_archive.resolve() or args.output.resolve().is_relative_to(args.disc.resolve()):
        raise SystemExit('Output must not replace original assets')
    catalog = build(args.disc, args.game_archive, args.map, *map(int, args.active.split('/')))
    text = json.dumps(catalog, indent=2) + '\n'
    if args.check:
        if args.output.read_text() != text:
            raise SystemExit('Catalog differs from reproduction')
    else:
        args.output.write_text(text)


if __name__ == '__main__':
    main()
