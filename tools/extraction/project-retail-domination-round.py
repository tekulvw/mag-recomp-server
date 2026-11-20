#!/usr/bin/env python3
"""Bind original retail Domination score/end rules to explicit worker resources."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent


def require(condition, reason):
    if not condition:
        raise ValueError(reason)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def project_rules(rules, source, spawn, objectives, spawn_sha256):
    require(spawn['match_profile']['game_type'] == objectives['game_type'] == 2,
            'Require Domination worker resources')
    require(len(spawn['maps']) == 1 and spawn['maps'][0]['name'] == objectives['map_name'],
            'Spawn/objective map mismatch')
    provenance = json.loads(spawn['source'])['game_rules_source']
    require(all(provenance[k] == source[k] for k in ('sha256', 'rules_json_pointer')),
            'Spawn rules source mismatch')
    points = [rules[f'StartingPointsTeam{team}'] for team in (0, 1)]
    percent, delay = rules['MajorVictoryPercent'], rules['EndGameDelay']
    require(all(type(value) is int for value in points + [percent, delay]),
            'Rules require literal integer values')
    require(all(0 < value <= 32767 for value in points) and 0 <= percent <= 100
            and 0 < delay <= 60000, 'Rules exceed the supported round domain')
    require(points == spawn['match_profile']['maximum_points'], 'Starting score mismatch')
    return {
        'format': 'mag_domination_round_v1', 'schema_version': 1,
        'spawn_catalog_sha256': spawn_sha256,
        'authored_objective_catalog_sha256': objectives['source']['authored_catalog_sha256'],
        'game_rules_sha256': source['sha256'],
        'game_rules_json_pointer': source['rules_json_pointer'],
        'starting_points': points, 'major_victory_percent': percent,
        'end_game_delay_ms': delay,
    }


def project(game_archive, spawn_bytes, objective_bytes):
    builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    raw = game_archive.read_bytes()
    require(sha(raw) == builder['GAME_ARCHIVE_SHA256'], 'Wrong original retail game.psarc')
    archive = builder['extractor']().Psarc(raw)
    name = '/run/mag/ps3/settings/gameproperties.zpr'
    settings = archive.extract(archive.names.index(name) + 1)
    _, source = builder['original_rules'](settings, 'GameType256')
    parser = runpy.run_path(str(TOOLS / 'extract-item-catalog.py'))
    document = parser['normalize'](parser['Parser'](settings.decode(), name).parse())
    selected = document
    for part in source['rules_json_pointer'].split('/')[1:]:
        selected = selected[part]
    return project_rules(selected, source, json.loads(spawn_bytes), json.loads(objective_bytes),
                         sha(spawn_bytes))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('game-archive', 'spawn-catalog', 'objective-catalog', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if (args.output.resolve().is_relative_to(TOOLS.parents[1])
            or args.output.resolve() in {p.resolve() for p in
                (args.game_archive, args.spawn_catalog, args.objective_catalog)}):
        parser.error('Output must be outside the source checkout and inputs')
    try:
        raw = (json.dumps(project(args.game_archive, args.spawn_catalog.read_bytes(),
                                  args.objective_catalog.read_bytes()), indent=2) + '\n').encode()
        if args.check:
            require(args.output.read_bytes() == raw, 'Round rules differ from retail regeneration')
        else:
            with args.output.open('xb') as stream:
                stream.write(raw)
    except (KeyError, ValueError, OSError, TypeError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
