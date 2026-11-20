#!/usr/bin/env python3
"""Project original Domination terrain mask textures to private material bytes."""

import argparse
from collections import Counter
import hashlib
import json
from pathlib import Path
import re
import struct
import sys
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / 'collision'))
from archive import Psarc
from zrb_records import parse

ELF_SHA256 = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
METADATA_SHA256 = 'adcc5bb4a4121b6ce611c5d1e84c840692235172c893e84548482c19be38f465'
GAME_SHA256 = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def colors_565(packed):
    red, green, blue = (packed >> 11), (packed >> 5) & 63, packed & 31
    return ((red << 3) | (red >> 2), (green << 2) | (green >> 4), (blue << 3) | (blue >> 2))


def selected_channels(block):
    alpha0, alpha1 = block[:2]
    if alpha0 > alpha1:
        alphas = [alpha0, alpha1] + [(alpha0*(7-i) + alpha1*i)//7 for i in range(1, 7)]
    else:
        alphas = [alpha0, alpha1] + [(alpha0*(5-i) + alpha1*i)//5 for i in range(1, 5)] + [0, 255]

    color0, color1 = map(colors_565, struct.unpack_from('<HH', block, 8))
    palette = [color0, color1]
    palette += [
        tuple((a * (3 - i) + b * i) // 3 for a, b in zip(color0, color1))
        for i in range(1, 3)
    ]
    dominant = [max(range(3), key=lambda channel: (rgb[channel], channel)) for rgb in palette]
    rgb_bits = int.from_bytes(block[12:16], 'little')
    alpha_bits = int.from_bytes(block[2:8], 'little')
    output = []
    for _ in range(16):
        color_slot = rgb_bits & 3
        rgb_index = dominant[color_slot]
        alpha = alphas[alpha_bits & 7]
        output.append(3 if alpha >= palette[color_slot][rgb_index] else rgb_index)
        rgb_bits >>= 2
        alpha_bits >>= 3
    return output


def project_texture(raw, dimension, indices):
    if (dimension < 4 or dimension % 4 or len(raw) != dimension**2
            or len(indices) != 4 or any(not 0 <= value <= 255 for value in indices)):
        raise ValueError('Invalid BC3 terrain mask dimensions, length or material slots')
    stride = dimension // 4
    decoded = bytearray(dimension * dimension)
    for block_index, block in enumerate(struct.iter_unpack('16s', raw)):
        block_x, block_y = block_index % stride, block_index // stride
        offset = (block_y * dimension + block_x) * 4
        channels = selected_channels(block[0])
        for row in range(4):
            start = offset + row*dimension
            decoded[start:start+4] = bytes(indices[i] for i in channels[4*row:4*row+4])
    return bytes(decoded)


def material_tokens(game):
    name = '/run/mag/ps3/scripts/materialproperties.zpr'
    body = game.extract(game.names.index(name) + 1)
    identities = re.findall(rb'MaterialProperties([0-9]+)\s*=\s*\{\s*MaterialName\s*=\s*"([^"]+)"', body)
    result = {zlib.crc32(name.lower()): int(index) for index, name in identities}
    require(len(result) == 28 and set(result.values()) == set(range(27)) | {255}, 'Wrong material names')
    return result


def projection(meta, archive, materials):
    source = meta['source']
    name = meta['map']['map']
    require(meta['map']['mode_enum'] == 'GameType256', 'Wrong original terrain mode')
    archive_raw = archive.read_bytes()
    require(sha(archive_raw) == source['archive_sha256'], 'Changed original terrain map archive')
    arc = Psarc(archive_raw)
    clv = arc.extract(arc.names.index(source['clv_member']) + 1)
    require(sha(clv) == source['clv_sha256'], 'Changed original terrain CLV')
    version, _, cols, cell_size, count = struct.unpack_from('>IIIfI', clv)
    require(version == 31 and cols in (8, 16) and cols*cols == count and cols*cell_size == 1024,
            'Unqualified original terrain grid dimensions')
    end = 20 + 20*count
    rows = list(struct.iter_unpack('>5I', clv[20:end]))
    require(struct.unpack_from('>H', clv, end)[0] == 128, 'Unqualified compiled visibility stride')
    centre_x, _, centre_z, extent_x, _, extent_z = struct.unpack_from('>6f', clv, end + 130)
    require((centre_x, centre_z, extent_x, extent_z) == (512.0,)*4, 'Unqualified original terrain bounds')
    require(len({row[0] for row in rows}) == count, 'Nonunique original terrain mask assets')
    by_token = {f'{r[0]:08x}': [materials[t] for t in r[1:]] for r in rows}
    mask_width = 4096 // cols
    found = {}
    for index, member in enumerate(arc.names, 1):
        if '/loadblock_' not in member or not member.endswith('.zrb'):
            continue
        block = arc.extract(index)
        sections = struct.unpack_from('>12I', block)
        for record in parse(block, retail13_compat=True)['records']:
            token = record['name_token']
            if token not in by_token or record['class_token'] != '85ce0660':
                continue
            header = bytes.fromhex(record['unparsed_resource_header_hex'])
            require(token not in found and record['resource_version'] == 5 and len(header) == 36
                    and header[:2] == bytes((1, 1))
                    and struct.unpack_from('>II', header, 6) == (mask_width, mask_width)
                    and header[14:18] == bytes.fromhex('0800aae4'), 'Unsupported original mask format')
            gpu = struct.unpack_from('>I', block, record['header_offset']+24)[0]
            require(gpu + mask_width**2 <= sections[8], 'Truncated original mask GPU bytes')
            raw = block[sections[7]+gpu:sections[7]+gpu+mask_width**2]
            found[token] = (project_texture(raw, mask_width, by_token[token]), sha(raw))
    require(found.keys() == by_token.keys(), 'Unresolved original terrain mask')
    result = b''.join(found[f'{row[0]:08x}'][0] for row in rows)
    receipt = {
        'map': name, 'schema': 'mag_domination_terrain_material_grid_v1',
        'level_archive_sha256': source['archive_sha256'], 'level_clv_sha256': source['clv_sha256'],
        'retail_elf_sha256': ELF_SHA256, 'columns': cols, 'cell_size': cell_size,
        'tile_width': mask_width, 'materials_sha256': sha(result), 'byte_count': len(result),
        'pixel_material_counts': dict(Counter(result)),
        'source_tile_sha256': [found[f'{row[0]:08x}'][1] for row in rows],
    }
    return name, receipt, result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--elf', type=Path, required=True, help='Original retail gameplay ELF')
    parser.add_argument('--metadata', type=Path, required=True, help='Qualified Domination map metadata')
    parser.add_argument('--levels', type=Path, required=True, help='Directory of original map PSARCs')
    parser.add_argument('--game', type=Path, required=True, help='Original shared game PSARC')
    parser.add_argument('--output', type=Path, required=True,
                        help='New private directory for per-map JSON and binary material grids')
    args = parser.parse_args()
    native = args.elf.read_bytes()
    require(sha(native) == ELF_SHA256, 'Wrong native material authority')
    authored = args.metadata.read_bytes()
    require(sha(authored) == METADATA_SHA256, 'Wrong original Domination metadata')
    game = args.game.read_bytes()
    require(sha(game) == GAME_SHA256, 'Wrong retail game archive')
    tokens = material_tokens(Psarc(game))
    args.output.mkdir(parents=True, exist_ok=False)
    for row in json.loads(authored)['maps']:
        path = args.levels / (row['map']['map'] + '.psarc')
        name, receipt, payload = projection(row, path, tokens)
        with (args.output / (name + '.bin')).open('xb') as stream:
            stream.write(payload)
        with (args.output / (name + '.json')).open('x') as stream:
            json.dump(receipt, stream, indent=2, allow_nan=False)
            stream.write('\n')
        print(name, receipt['columns'], receipt['tile_width'], receipt['pixel_material_counts'], flush=True)


if __name__ == '__main__':
    main()
