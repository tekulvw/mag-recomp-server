#!/usr/bin/env python3
"""Preserve complete authored prototype XP/progression resources as disk configuration.

Reads literal Lua/ZPR with the existing nonexecuting parser. Raw Experience values
are retained; cumulative/scaled/rounded runtime meaning needs native evidence.
Default writes the catalog and provenance. --check regenerates entirely in memory
and requires byte-identical outputs, including unchanged source evidence copies.
"""
import argparse
import hashlib
import json
from pathlib import Path
import runpy

ROOT = Path(__file__).resolve().parents[2]
PARSER = ROOT/'tools/extraction/extract-item-catalog.py'
SOURCES = (
    'scripts/experience.zpr',
    'scripts/character_awards.zpr',
    'scripts/character_stats.zpr',
    'scripts/shadowwar_contract_bonuses.zpr',
    'scripts/data_scripts.zpr',
    'settings/magrootsettings.lua',
    'settings/maglocalsettings.lua',
)
TABLES = {
    'scripts/experience.zpr': 'ExperienceTable',
    'scripts/character_awards.zpr': 'CharacterAwardTable',
    'scripts/character_stats.zpr': 'CharacterStatsInfoTable',
    'scripts/shadowwar_contract_bonuses.zpr': 'ShadowWarContractBonusTable',
}


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2)+'\n').encode()


def render(ASSETS):
    parser = runpy.run_path(str(PARSER))
    source_bytes = {name: (ASSETS/name).read_bytes() for name in SOURCES}
    parsed = {name: parser['Parser'](raw.decode('utf-8'), name).parse() for name, raw in source_bytes.items()}
    literals = {}; normalized = {}
    for name, node in parsed.items():
        literals[name] = {}
        normalized[name] = parser['normalize'](node, literals=literals[name])
    tables = {key: normalized[name][key] for name, key in TABLES.items()}
    experience = tables['ExperienceTable']
    levels = []
    for name, wrapper in experience['Levels'].items():
        prefix = 'ExperienceLevel'
        if not name.startswith(prefix) or not name[len(prefix):].isdigit():
            raise ValueError('Unexpected authored level identifier: '+name)
        ordinal = int(name[len(prefix):]); data = wrapper['ExperienceLevel']
        if not data: continue  # Preserve unused rows in the complete table.
        required = ('Experience', 'SkillPointsGained', 'LoadoutBudget', 'Rank')
        if any(type(data.get(key)) is not int or data[key] < 0 for key in required):
            raise ValueError('Malformed authored level definition: '+name)
        levels.append({'level': ordinal, **data})
    levels.sort(key=lambda row: row['level'])
    if not levels or len({row['level'] for row in levels}) != len(levels):
        raise ValueError('Missing or duplicated authored level rows')
    # Settings may contain duplicate unrelated top-level Lua assignments. Select
    # the exact named creation/stat tables from the parser's ordered entries.
    def setting(name, key):
        matches = [node for entry, node in parsed[name].entries if entry == key]
        if len(matches) != 1: raise ValueError('Expected exactly one '+key+' in '+name)
        return parser['normalize'](matches[0])
    settings = {
        'root_defaults': setting('settings/magrootsettings.lua', 'MAGCharacterRecordCreationSettings'),
        'installed_local_overrides': setting('settings/maglocalsettings.lua', 'MAGCharacterRecordCreationSettings'),
        'character_stats': setting('settings/magrootsettings.lua', 'MAGCharacterStats'),
    }
    source_sha = {name: sha(raw) for name, raw in source_bytes.items()}
    tools = {str(path.relative_to(ROOT)): sha(path.read_bytes()) for path in (Path(__file__).resolve(), PARSER)}
    catalog = {
        'schema_version': 1,
        'application_id': 20230,
        'status': 'authored prototype progression resources; native award and threshold interpretation documented separately',
        'source_sha256': source_sha,
        'tool_sha256': tools,
        'table': experience,
        'related_tables': {key: value for key, value in tables.items() if key != 'ExperienceTable'},
        'creation_settings': settings,
        'source_literals': literals,
    }
    stats = tables['CharacterStatsInfoTable']['Stats']
    summary = {
        'application_id': catalog['application_id'],
        'maximum_authored_level': max(row['level'] for row in levels),
        'defined_level_count': len(levels),
        'raw_levels': levels,
        'experience_required_scale': experience['ExperienceRequiredScale'],
        'experience_values': 'raw authored values; threshold interpretation and integer rounding are native questions',
        'awards': experience['Awards'],
        'creation_settings': settings,
        'stat_row_count': len(stats),
        'stat_tokens': [row['Stat'] for row in stats],
        'stat_token_policy': 'authored tokens; array positions do not establish native enum ordinals',
        'related_table_names': list(catalog['related_tables']),
    }
    return encode(catalog)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--assets', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    inputs = [args.assets]
    if args.output.resolve() in {v.resolve() for v in inputs}:
        parser.error('output must not replace source inputs')
    payload = render(*inputs)
    if args.check:
        if args.output.read_bytes() != payload:
            raise ValueError('catalog differs from source regeneration')
    else:
        with args.output.open('xb') as stream:
            stream.write(payload)

if __name__ == '__main__':
    main()
