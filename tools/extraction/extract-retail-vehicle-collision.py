#!/usr/bin/env python3
"""Read ground-vehicle impact rules from the hash-qualified retail ELF and archive.

The two authored values come from the server properties member of the game
archive. The remaining values are scalar operands of the retail server's
vehicle-character collision handling, read at fixed supported-build addresses
whose instruction encodings are checked first. No disassembly, game execution
or runtime capture is performed, and no code is copied.
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
MEMBER = '/run/mag/ps3/settings/server.zpr'

# Supported-build layout: (address, expected high halfword) of each 16-bit
# immediate operand, and the address of each float constant.
IMMEDIATES = {
    # How long after leaving a vehicle a character is immune to it (ms).
    'recent_exit_grace_ms': (0x7C68D0, 0x3BBD),
    # Age after which a sampled position no longer gives a velocity (ms).
    'motion_freshness_ms': (0x7ED610, 0x3BBD),
    # Damage kind written into the collision's damage data.
    'damage_type': (0x7EE2CC, 0x3920),
    # Damage source type written into the collision's damage data.
    'damage_source_type': (0x7EE2F4, 0x3800),
}
FLOATS = {
    # Flat raw damage of a catastrophic collision.
    'catastrophic_damage_hp': 0x1B4E48C,
    # Shortest interval a sampled velocity divides by (ms).
    'minimum_motion_interval_ms': 0x1B4CCA4,
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

    config = {
        'schema_version': 1, 'source_build': 'retail', 'application_id': 20231,
        'minimum_velocity_mps': authored('VehicleCollisionDamageMinVelocity'),
        'damage_per_mps': authored('VehicleCollisionDamageFactor'),
    }
    for name, address in FLOATS.items():
        value = struct.unpack('>f', retail.read(address, 4)[0])[0]
        if not math.isfinite(value) or value <= 0:
            raise ValueError('Invalid native scalar ' + name)
        config[name] = value
    for name, (address, high) in IMMEDIATES.items():
        word = retail.word(address)
        if word >> 16 != high:
            raise ValueError('Unexpected native operand encoding for ' + name)
        config[name] = word & 0xFFFF
    order = ['schema_version', 'source_build', 'application_id', 'minimum_velocity_mps',
             'damage_per_mps', 'catastrophic_damage_hp', 'recent_exit_grace_ms',
             'motion_freshness_ms', 'minimum_motion_interval_ms', 'damage_type',
             'damage_source_type']
    config = {key: config[key] for key in order}
    if (config['minimum_velocity_mps'] < 0 or config['damage_per_mps'] <= 0
            or config['damage_type'] != 5 or config['damage_source_type'] != 3
            or not 0 < config['motion_freshness_ms'] <= 60000
            or config['recent_exit_grace_ms'] > 60000):
        raise ValueError('Invalid vehicle collision rules')
    config['provenance'] = {
        'retail_elf_sha256': SOURCES['retail'],
        'game_archive_sha256': SOURCES['archive'],
        'server_properties_member': MEMBER,
        'server_properties_sha256': sha(zpr),
    }
    return config


def produce(elf_path, archive_path):
    retail = Elf(elf_path, 'retail')
    raw = archive_path.read_bytes()
    if sha(raw) != SOURCES['archive']:
        raise ValueError('Original retail archive SHA256 mismatch')
    archive = Psarc(raw)
    if MEMBER not in archive.names:
        raise ValueError('Missing server properties member')
    return project(retail, archive.extract(archive.names.index(MEMBER) + 1))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--elf', type=Path, required=True)
    parser.add_argument('--game-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    if args.output.resolve() in {args.elf.resolve(), args.game_archive.resolve()}:
        raise ValueError('input_output_collision')
    payload = (json.dumps(produce(args.elf, args.game_archive), indent=2, allow_nan=False) + '\n').encode()
    if args.check:
        if args.output.read_bytes() != payload:
            raise ValueError('vehicle_collision_rules_differ')
    else:
        with args.output.open('xb') as stream:
            stream.write(payload)
    print(json.dumps({'projected': True, 'check': args.check}))


if __name__ == '__main__':
    main()
