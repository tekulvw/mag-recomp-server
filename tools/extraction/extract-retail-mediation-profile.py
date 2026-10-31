#!/usr/bin/env python3
"""Extract retail's MAS/MLS protocol version through ELF load mappings.

No game resources are edited. The selected local server profile is JSON data;
its disabled catalog grants are an explicit policy pending retail extraction.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


RETAIL_SHA = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'


def extract(path):
    raw = path.read_bytes()
    digest = hashlib.sha256(raw).hexdigest()
    if digest != RETAIL_SHA: raise ValueError('Unexpected retail ELF identity')
    if raw[:6] != b'\x7fELF\x02\x02': raise ValueError('Expected big-endian ELF64')
    header = struct.unpack_from('>16sHHIQQQIHHHHHH', raw)
    segments = [struct.unpack_from('>IIQQQQQQ', raw, header[5]+i*header[9]) for i in range(header[10])]
    def word(ea):
        for kind, flags, offset, address, physical, size, memory, alignment in segments:
            if kind == 1 and address <= ea and ea+4 <= address+size:
                file_offset = offset+ea-address
                return struct.unpack_from('>I', raw, file_offset)[0], file_offset
        raise ValueError('Address has no file-backed load segment')
    toc = 0x1b35b20
    pointer, pointer_offset = word(toc-0x6c88)
    version, value_offset = word(pointer)
    return {'schema_version':1, 'application_id':20231, 'protocol_version':version,
        'enable_character_catalogs':False,
        'catalog_policy':'Do not grant prototype inventories, starter kits, skills or progression to fresh retail accounts.',
        'provenance':{'elf_sha256':digest, 'original_check':'001BBA20', 'compare':'001BBAE8',
            'toc':hex(toc), 'pointer_ea':hex(toc-0x6c88), 'pointer_file_offset':hex(pointer_offset),
            'value_ea':hex(pointer), 'value_file_offset':hex(value_offset), 'word':f'{version:08x}',
            'mismatch_result':14}}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--elf', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists(): raise SystemExit('Refusing existing profile')
    profile = extract(args.elf)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(profile, indent=2)+'\n')
    print(json.dumps(profile, indent=2))


if __name__ == '__main__': main()
