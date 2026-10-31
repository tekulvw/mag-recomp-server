#!/usr/bin/env python3
# Adapted project-owned extractor; precursor source SHA256: 4e21d4e9c015b8985ba0964d9909269d5fe605ff5e0da075c19899988cb613ed
"""Read falling-damage scalars from exact hash-qualified retail ELF and archive.

No disassembly, DWARF generation, game execution or runtime capture is performed.
The fixed addresses are supported-build layout metadata, not embedded values.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy
import struct
import sys

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'collision'))
from archive import Psarc

SOURCES = {
    'retail': '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c',
    'archive': 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f',
}


def sha(data):
    return hashlib.sha256(data).hexdigest()


class Elf:
    def __init__(self, path, build):
        self.raw = path.read_bytes()
        if sha(self.raw) != SOURCES[build]:
            raise ValueError(f'{build} original ELF SHA-256 mismatch')
        if self.raw[:6] != b'\x7fELF\x02\x02':
            raise ValueError('Expected ELF64 big-endian')
        if len(self.raw) < 64 or struct.unpack_from('>H', self.raw, 18)[0] != 21:
            raise ValueError('Expected complete PPC64 ELF header')
        start = struct.unpack_from('>Q', self.raw, 32)[0]
        size, count = struct.unpack_from('>HH', self.raw, 54)
        if size < 56 or count == 0 or start + size * count > len(self.raw):
            raise ValueError('Invalid program header bounds')
        self.segments = []
        for i in range(count):
            kind, flags, offset, address, _, length, memory_length, _ = struct.unpack_from(
                '>IIQQQQQQ', self.raw, start + size * i)
            if kind == 1:
                if length > memory_length or offset + length > len(self.raw):
                    raise ValueError('Invalid file-backed segment bounds')
                self.segments.append((offset, address, length, flags))

    def read(self, address, size):
        candidates = [(offset + address - start, flags)
                      for offset, start, length, flags in self.segments
                      if start <= address and address + size <= start + length]
        if len(candidates) != 1 or not 0 < size <= 16384:
            raise ValueError('Require bounded unique file-backed ELF range')
        offset, flags = candidates[0]
        return self.raw[offset:offset + size], offset, flags

    def word(self, address):
        return int.from_bytes(self.read(address, 4)[0], 'big')



def project(retail, zpr):
    parser = runpy.run_path(str(Path(__file__).with_name('extract-item-catalog.py')))
    resource = parser['normalize'](parser['Parser'](zpr.decode('utf-8-sig'), 'server.zpr').parse())

    def authored(name):
        matches = []
        def walk(value):
            if isinstance(value, dict):
                for key, child in value.items():
                    if key == name:
                        matches.append(child)
                    walk(child)
        walk(resource)
        if (len(matches) != 1 or type(matches[0]) not in (int, float)
                or not math.isfinite(matches[0])):
            raise ValueError('Require one finite authored scalar ' + name)
        return matches[0]

    # Existing supported-build TOC interpretation; no copied instruction verifier.
    resolver_toc = 0x1B35B20
    getter_toc = resolver_toc + 0x10000 - 0x1C
    fall_toc = getter_toc + 0x10000 - 0x294
    threshold = struct.unpack('>f', retail.read(fall_toc - 0x8000, 4)[0])[0]
    config = {
        'schema_version': 1, 'source_build': 'retail', 'application_id': 20231,
        'min_height': authored('FallingDamageMinHeight'),
        'damage_factor': authored('FallingDamageFactor'),
        'minimum_damage_exclusive': threshold,
        'jump_fall_state': retail.word(0x7CBD3C) & 0xFFFF,
        'damage_type': retail.word(0x7CBA6C) & 0xFFFF,
        'damage_source_type': retail.word(0x7CBA7C) & 0xFFFF,
    }
    if (config['min_height'] < 0 or config['damage_factor'] <= 0
            or not math.isfinite(threshold) or threshold < 0
            or config['jump_fall_state'] >= 64 or config['damage_type'] != 5
            or config['damage_source_type'] != 1):
        raise ValueError('Invalid falling-damage rules')
    return config


def produce(elf_path, archive_path):
    retail = Elf(elf_path, 'retail')
    raw = archive_path.read_bytes()
    if sha(raw) != SOURCES['archive']:
        raise ValueError('Original retail archive SHA256 mismatch')
    archive = Psarc(raw)
    member = '/run/mag/ps3/settings/server.zpr'
    if member not in archive.names:
        raise ValueError('Missing server properties member')
    return project(retail, archive.extract(archive.names.index(member) + 1))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--elf', type=Path, required=True)
    parser.add_argument('--game-archive', type=Path, required=True)
    parser.add_argument('--config-output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.config_output.resolve() in {args.elf.resolve(), args.game_archive.resolve()}:
        raise ValueError('input_output_collision')
    payload = (json.dumps(produce(args.elf, args.game_archive), indent=2, allow_nan=False) + '\n').encode()
    if args.check:
        if args.config_output.read_bytes() != payload:
            raise ValueError('falling_damage_rules_differ')
    else:
        with args.config_output.open('xb') as stream:
            stream.write(payload)
    print(json.dumps({'projected': True, 'check': args.check}))


if __name__ == '__main__':
    main()
