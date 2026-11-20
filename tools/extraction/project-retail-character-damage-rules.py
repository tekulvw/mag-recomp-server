#!/usr/bin/env python3
"""Project original selected-mode friendly fire into a source-bound worker profile."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy

TOOLS = Path(__file__).resolve().parent


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def project_rules(rules, source, spawn, spawn_sha256, game_sha256):
    require(len(spawn['maps']) == 1 and type(spawn['match_profile']['game_type']) is int
            and spawn['match_profile']['game_type'] in range(4), 'Invalid selected mode')
    provenance = json.loads(spawn['source'])['game_rules_source']
    require(all(provenance[key] == source[key] for key in ('sha256', 'rules_json_pointer')),
            'Rules source mismatch')
    value = rules['FriendlyFireDamageMultiplier']
    require(type(value) in (int, float) and math.isfinite(value) and 0 <= value <= 100,
            'Unsupported friendly fire multiplier')
    return dict(schema_version=1, application_id=20231, map_name=spawn['maps'][0]['name'],
                game_type=spawn['match_profile']['game_type'], friendly_fire_multiplier=value,
                provenance=dict(spawn_catalog_sha256=spawn_sha256,
                                game_archive_sha256=game_sha256, rules_member_sha256=source['sha256'],
                                json_pointer=source['rules_json_pointer'] + '/FriendlyFireDamageMultiplier'))


def project(game_path, spawn_raw):
    builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    original = game_path.read_bytes()
    require(sha(original) == builder['GAME_ARCHIVE_SHA256'], 'Wrong original retail game archive')
    archive = builder['extractor']().Psarc(original)
    member = '/run/mag/ps3/settings/gameproperties.zpr'
    raw = archive.extract(archive.names.index(member) + 1)
    spawn = json.loads(spawn_raw)
    mode = spawn['match_profile']['game_type']
    modes = {value: key for key, value in builder['MODES'].items()}
    require(type(mode) is int and mode in modes, 'Unsupported selected mode')
    _, source = builder['original_rules'](raw, modes[mode])
    literal = runpy.run_path(str(TOOLS / 'extract-item-catalog.py'))
    rules = literal['normalize'](literal['Parser'](raw.decode(), member).parse())
    for key in source['rules_json_pointer'].split('/')[1:]:
        rules = rules[key]
    return project_rules(rules, source, spawn, sha(spawn_raw), sha(original))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('game-archive', 'spawn-catalog', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        output = args.output.resolve()
        require(not output.is_relative_to(TOOLS.parents[1]) and output not in {
            path.resolve() for path in (args.game_archive, args.spawn_catalog)
        }, 'Output must be outside the source checkout and inputs')
        raw = args.spawn_catalog.read_bytes()
        require(len(raw) <= 1_048_576, 'Spawn input exceeds bound')
        encoded = (json.dumps(project(args.game_archive, raw), indent=2, allow_nan=False) + '\n').encode()
        if args.check:
            require(output.read_bytes() == encoded, 'Character damage rules differ')
        else:
            with output.open('xb') as stream:
                stream.write(encoded)
        print(json.dumps({'output_sha256': sha(encoded)}))
    except (KeyError, TypeError, ValueError, OSError) as error:
        parser.exit(1, f'{parser.prog}: {error}\n')


if __name__ == '__main__':
    main()
