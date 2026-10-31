#!/usr/bin/env python3
"""Extract retail progression/Training configuration from pinned original data.

Uses the existing literal Lua parser and PSARC reader without executing game
code. Completion mask provenance is the separately verified original witness.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re
import runpy
import sys

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / 'tools/collision'))
from archive import Psarc


def extract(archive_path, witness_path):
    witness = json.loads(witness_path.read_text())
    raw = archive_path.read_bytes()
    sha = lambda value: hashlib.sha256(value).hexdigest()
    if sha(raw) != witness['sources']['decrypted_game_archive_sha256']:
        raise ValueError('Original retail archive hash mismatch')
    arc = Psarc(raw)
    def member(name):
        return arc.extract(arc.names.index('/run/mag/ps3/' + name) + 1)
    experience = member('scripts/experience.zpr')
    settings = member('settings/magrootsettings.lua')
    if sha(experience) != witness['authored_reward']['experience_zpr_sha256']:
        raise ValueError('Original retail experience member mismatch')
    parser = runpy.run_path(str(ROOT / 'tools/extraction/extract-item-catalog.py'))
    def parse(text, name):
        return parser['normalize'](parser['Parser'](text, name).parse())
    table = parse(experience.decode(), 'experience.zpr')['ExperienceTable']
    creation = re.search(r'MAGCharacterRecordCreationSettings\s*=\s*\{[^}]*\}', settings.decode())
    if creation is None:
        raise ValueError('Missing authored character creation defaults')
    defaults = parse(creation[0], 'magrootsettings.lua')['MAGCharacterRecordCreationSettings']
    if table['Training_ExperienceBonus'] != witness['authored_reward']['experience_bonus'] or table['ExperienceRequiredScale'] != witness['authored_reward']['scale']:
        raise ValueError('Reward/scale disagrees with original witness')
    return {'schema_version': 1, 'application_id': 20231,
            'status': 'retail authored progression; one-time Training reward policy, not recovered production MLS',
            'source_sha256': {'game.psarc': sha(raw), 'experience.zpr': sha(experience), 'magrootsettings.lua': sha(settings)},
            'training_completion': {'completed_flag_mask': witness['native_flag']['mask'],
                                    'evidence': witness_path.name},
            'table': table, 'creation_settings': {'root_defaults': defaults}}


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument('--game-archive', type=Path, required=True)
    p.add_argument('--output', type=Path, required=True)
    p.add_argument('--witness', type=Path, required=True, help='external independent training-completion witness JSON')
    p.add_argument('--check', action='store_true')
    a = p.parse_args()
    if a.output.resolve() in {a.game_archive.resolve(), a.witness.resolve()}:
        p.error('output must not replace inputs')
    report = extract(a.game_archive, a.witness)
    raw = (json.dumps(report, indent=2) + '\n').encode()
    if a.check:
        if a.output.read_bytes() != raw:
            raise SystemExit('Catalog differs from original extraction')
    else:
        with a.output.open('xb') as output:
            output.write(raw)
    print(json.dumps({'verified': True, 'level_rows': len(report['table']['Levels']), 'training_xp': report['table']['Training_ExperienceBonus']}))


if __name__ == '__main__':
    main()
