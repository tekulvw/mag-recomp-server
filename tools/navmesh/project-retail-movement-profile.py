#!/usr/bin/env python3
"""Project the ordinary authored ascent profile from supported retail sources."""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy
import sys

TOOLS = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(TOOLS / 'collision'))
from archive import Psarc

MEMBER = '/run/mag/ps3/scripts/character.zpr'
SOURCES = {
    'retail_elf_sha256': '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c',
    'game_archive_sha256': 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f',
    'character_zpr_sha256': '87c76a863bbe869891ade8719eab12f451b38b22f094cfbc86a294b0bd8b9f98',
}


def verify_hash(raw, key):
    digest = hashlib.sha256(raw).hexdigest()
    if digest != SOURCES[key]:
        raise ValueError(f'{key}: SHA-256 mismatch')
    return digest


def read_source(path, limit):
    if not path.is_file():
        raise ValueError(f'{path}: expected a regular source file')
    with path.open('rb') as source:
        raw = source.read(limit + 1)
    if len(raw) > limit:
        raise ValueError(f'{path}: source exceeds {limit} bytes')
    return raw


def authored_angles(raw):
    parser = runpy.run_path(str(TOOLS / 'extraction/extract-item-catalog.py'))
    resource = parser['Parser'](raw.decode('utf-8-sig'), MEMBER).parse()

    def unique(table, name):
        matches = [value for key, value in table.entries if key == name]
        if len(matches) != 1:
            raise ValueError(f'{MEMBER}: expected exactly one {name}')
        return matches[0]

    settings = unique(resource, 'CharacterSettings')
    if not isinstance(settings, parser['Table']):
        raise ValueError(f'{MEMBER}: expected a CharacterSettings table')

    def angle(name):
        value = parser['normalize'](unique(settings, name))
        if type(value) not in (float, int) or not 0 < value < 90 or not math.isfinite(value):
            raise ValueError(f'{MEMBER}: invalid {name}')
        return float(value)

    ascent = angle('fMaxAscentAngle')
    no_slide = angle('fMaxNoSlideAngle')
    if ascent != no_slide:
        raise ValueError(f'{MEMBER}: ascent and no-slide angles must agree')
    return ascent, no_slide


def produce(elf_path, archive_path):
    # These exact digests bind the separately verified native reflection and consumer.
    elf_sha = verify_hash(read_source(elf_path, 64 * 1024**2), 'retail_elf_sha256')
    raw = read_source(archive_path, 32 * 1024**2)
    archive_sha = verify_hash(raw, 'game_archive_sha256')
    archive = Psarc(raw)
    if MEMBER not in archive.names:
        raise ValueError(f'missing archive member {MEMBER}')
    member = archive.extract(archive.names.index(MEMBER) + 1)
    if len(member) > 64 * 1024:
        raise ValueError(f'{MEMBER}: character settings exceed size limit')
    member_sha = verify_hash(member, 'character_zpr_sha256')
    ascent, no_slide = authored_angles(member)
    return {
        'schema_version': 1,
        'application_id': 20231,
        'source': {
            'retail_elf_sha256': elf_sha,
            'game_archive_sha256': archive_sha,
            'character_zpr_sha256': member_sha,
        },
        'max_ascent_degrees': ascent,
        'max_no_slide_degrees': no_slide,
    }


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--elf', required=True, type=Path)
    cli.add_argument('--game-archive', required=True, type=Path)
    cli.add_argument('--output', required=True, type=Path)
    cli.add_argument('--check', action='store_true', help='verify the existing output byte for byte')
    args = cli.parse_args()
    try:
        if args.output.resolve() in (args.elf.resolve(), args.game_archive.resolve()):
            raise ValueError('output must differ from both source paths')
        if args.output.exists() and any(args.output.samefile(source)
                                        for source in (args.elf, args.game_archive)):
            raise ValueError('output must differ from both source files')
        output = (json.dumps(produce(args.elf, args.game_archive), indent=2) + '\n').encode()
        if args.check:
            if read_source(args.output, 64 * 1024) != output:
                raise ValueError('movement profile differs from projected output')
        else:
            args.output.write_bytes(output)
    except (ValueError, OSError) as error:
        cli.exit(1, f'{error}\n')


if __name__ == '__main__':
    main()
