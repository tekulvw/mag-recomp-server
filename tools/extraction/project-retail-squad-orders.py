#!/usr/bin/env python3
"""Project the authored squad order cooldown from the supported retail game archive."""
import argparse
import hashlib
import json
from pathlib import Path
import runpy
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'collision'))
from archive import Psarc

ARCHIVE_SHA256 = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
MEMBER = '/run/mag/ps3/scripts/character_gameplay.zpr'
PROPERTY = 'SquadLeaderOrderCooldown_ms'


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def project(member, archive_sha256):
    parser = runpy.run_path(str(Path(__file__).with_name('extract-item-catalog.py')))
    settings = parser['Parser'](member.decode('utf-8-sig'), MEMBER).parse()
    matches = [
        value for _, section in settings.entries if isinstance(section, parser['Table'])
        for name, value in section.entries if name == PROPERTY
    ]
    error = f'require one authored unsigned 32-bit {PROPERTY}'
    if len(matches) != 1 or not isinstance(matches[0], parser['Scalar']):
        raise ValueError(error)
    scalar = matches[0]
    value = scalar.value
    if scalar.kind == 'string' and value.isascii() and value.isdecimal():
        value = int(value)
    if type(value) not in (int, float) or not 0 <= value <= 0xffffffff or value % 1 != 0:
        raise ValueError(error)
    return {
        'schema_version': 1,
        'application_id': 20231,
        'source': {'archive_sha256': archive_sha256, 'member': MEMBER, 'member_sha256': sha(member)},
        'settings': {'squad_order_cooldown_ms': int(value)},
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.output.resolve() == args.archive.resolve():
        parser.error('output must not replace the source archive')
    raw = args.archive.read_bytes()
    archive_sha256 = sha(raw)
    if archive_sha256 != ARCHIVE_SHA256:
        parser.error('unsupported retail game archive SHA-256')
    archive = Psarc(raw)
    if archive.names.count(MEMBER) != 1:
        parser.error('require one character gameplay member')
    member = archive.extract(archive.names.index(MEMBER) + 1)
    result = (json.dumps(project(member, archive_sha256), indent=2) + '\n').encode()
    if args.check:
        if args.output.read_bytes() != result:
            raise ValueError('squad order projection differs')
    else:
        with args.output.open('xb') as output:
            output.write(result)


if __name__ == '__main__':
    main()
