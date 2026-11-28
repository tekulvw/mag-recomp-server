#!/usr/bin/env python3
"""Build private, source-bound 2.12 authentication-only character catalogs.

Only the maintained CLI profile is qualified. Output policies are diagnostic
operator choices, not recovered original server defaults. No skill purchases,
training rewards, gameplay worlds or native create/login acceptance are claimed.
"""
import argparse
from dataclasses import dataclass
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import tempfile
import zlib

from reflective_zrb import Reader

ROOT = Path(__file__).resolve().parents[2]
SLOTS = ('Primary', 'Backup', 'Swipe', 'Heavy', 'Gear1', 'Gear2', 'PDA', 'C4',
         'Satchel', 'Parachute', 'ApparelHead', 'ApparelFace', 'Armor', 'Uniform',
         'PrimarySight', 'PrimaryMagazine', 'PrimaryFirePoint', 'PrimaryUnderBarrel',
         'BackupSight', 'BackupMagazine', 'BackupFirePoint', 'HeavySight')


@dataclass(frozen=True)
class SourceSpec:
    name: str
    engine: str
    sources: dict
    bank_counts: tuple
    item_count: int
    weapon_count: int
    level_count: int


SUPPORTED = SourceSpec('retail-v2.12-bootstrap',
    '705a6ec6a25458b2401b869e75e2904597304e2c06d9534a171a1dcd54aca2b3', {
        'headers': '9845c9c024dddbe035578d0afe09fe3022b2d19a7969a89dd0701f8a8e391326',
        'classes': 'b6c722ac6acb53710b470b96e394942a03c223a08c0703c42b2d7024d6562325',
        'equipment': 'e187f6989d363aef7f782d7d23ef1e38fd94aa338c449b390836fb2137e9d53d'},
    (23, 228, 215, 230), 642, 116, 71)
POLICY = dict(initial_level=1, additional_skill_points=0, additional_leadership_points=0,
              authored_kits=3, wire_loadouts=5, update_id=1, skill_grants='none',
              training_rewards=False, gameplay_worlds=False)
QUALIFICATION = dict(kind='static-diagnostic', native_acceptance=False, residuals=[
    'Selected native vector-read order, 22 inventory slots, bank encoding and ownership mask are statically bound to the selected engine. Live vectors and later postload mutations remain unobserved.',
    'Equipment CRC32 identity joins are unique source-data consistency evidence; native canonicalization and every associated-item resolution helper are not fully audited.',
    'Level1, additional points0, three authored kits plus two cleared inventories, update IDs1 and starter ownership union are explicit local bootstrap policies, not recovered original server defaults.',
    'No purchased skills, training rewards, gameplay, economy or new character-record tail semantics are qualified.',
    'Native client create/login acceptance remains unverified.'])


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(data):
    return hashlib.sha256(data).hexdigest()


def encode(value):
    return (json.dumps(value, indent=2, sort_keys=True, allow_nan=False) + '\n').encode()


def objects(data, name):
    marker = Reader.OBJECT + struct.pack('>I', len(name) + 1) + name.encode('ascii') + b'\0'
    reader, cursor, result = Reader(data), 0, []
    while (cursor := data.find(marker, cursor)) >= 0:
        value, cursor = reader.object(cursor)
        result.append(value)
    return result


def one(data, name):
    values = objects(data, name)
    require(len(values) == 1, 'Expected exactly one resource object: ' + name)
    return values[0]


def value(node, key, kind=None):
    field = node['fields'][key]
    require(kind is None or field['type'] == kind, 'Unexpected property type: ' + key)
    return field['value']


def rows(node, key, classname=None):
    result = value(node, key, 'ztl::vector')
    require(isinstance(result, list) and (classname is None or all(row['class'] == classname for row in result)),
            'Unsupported vector layout: ' + key)
    return result


def enumeration(node, key):
    raw = value(node, key, 'enum')
    prefix = 'deadbeef0000000100000000'
    require(isinstance(raw, str) and len(raw) == 26 and raw.startswith(prefix), 'Unexpected enum layout: ' + key)
    return int(raw[-2:], 16)


def normalize(node):
    output = {}
    for key, field in node['fields'].items():
        require(not field.get('opaque'), 'Opaque progression field: ' + key)
        raw = field['value']
        if isinstance(raw, dict):
            raw = normalize(raw)
        elif isinstance(raw, list):
            raw = {f"{row['class']}{ordinal}": {row['class']: normalize(row)} for ordinal, row in enumerate(raw)}
        elif isinstance(raw, float) and raw.is_integer():
            raw = int(raw)
        output[key] = raw
    return output


def render(inputs, spec=SUPPORTED):
    """Pure projection; synthetic callers supply an explicit identity spec."""
    require(set(inputs) == {'engine', 'headers', 'classes', 'equipment'}, 'Exactly four source inputs are required.')
    require(sha(inputs['engine']) == spec.engine, 'Engine SHA256 differs from the supported profile.')
    require(all(sha(inputs[key]) == digest for key, digest in spec.sources.items()), 'Resource SHA256 differs from the supported profile.')
    base = dict(schema_version=1, application_id=20231, profile=spec.name,
                protocol_version=1793, engine_sha256=spec.engine, source_sha256=spec.sources)
    banks = rows(one(inputs['headers'], 'ItemList'), 'factionItemLists', 'FactionItemList')
    require(tuple(len(rows(bank, 'items', 'ItemListData')) for bank in banks) == spec.bank_counts and len(banks) == 4,
            'Unexpected bank count or serialized item-vector lengths.')
    weapons = {}
    weapon_rows = objects(inputs['equipment'], 'WeaponData')
    require(len(weapon_rows) == spec.weapon_count, 'Unexpected weapon definition count.')
    for weapon in weapon_rows:
        name = value(weapon, 'id', 'ztl::string')
        require(name and name.isascii(), 'Weapon identity must be nonempty ASCII.')
        token = f"0x{zlib.crc32(name.lower().replace(chr(92), '/').encode('ascii')):08x}"
        require(token not in weapons, 'Ambiguous equipment token join.')
        capacity = value(weapon, 'magazine_capacity')
        capacity = capacity if type(capacity) is int and 1 <= capacity <= 255 else None
        weapons[token] = dict(name=name, capacity=capacity)
    by_token, items, by_wire = {}, [], {}
    for bank, entry in enumerate(banks):
        for ordinal, item in enumerate(rows(entry, 'items', 'ItemListData')):
            if 'id' not in item['fields']:
                continue  # Authored empty positions keep their ordinal.
            token = value(item, 'id', 'util::Token')
            require(token != '0x00000000' and token not in by_token, 'Duplicate or null assigned item token.')
            wire = ((bank << 10) | ordinal) + (50 if bank else 0)
            require(0 <= ordinal < 1024 and 0 <= wire < 4096 and (wire & 1023) < 768, 'Wire/ownership range overflow.')
            weapon = weapons.get(token)
            row = dict(wire_id=wire, name=weapon['name'] if weapon else token,
                       authored_ref=f'FactionItemList{bank}:{ordinal}', available_bit=wire & 1023,
                       factions=[0, 1, 2] if bank == 0 else [bank - 1], primary_ammo=weapon['capacity'] if weapon else None)
            require(wire not in by_wire, 'Duplicate wire identity.')
            items.append(row); by_token[token] = wire; by_wire[wire] = row
    require(len(items) == spec.item_count, 'Unexpected nonempty item count.')
    factions = {}
    for entry in rows(one(inputs['classes'], 'CharacterClassTable'), 'Classes'):
        if enumeration(entry, 'Class') != 21:
            continue
        faction = enumeration(entry, 'Faction')
        require(faction in (0, 1, 2) and faction not in factions, 'Duplicate or foreign selected faction.')
        kits = []
        for kit in rows(entry, 'InventoryList', 'CharacterClassInv'):
            require(set(kit['fields']) in (set(SLOTS), set(SLOTS) - {'BackupMagazine'}), 'Unknown or missing authored inventory slots.')
            slots = []
            for slot in SLOTS:
                token = value(kit, slot, 'util::Token') if slot in kit['fields'] else '0x00000000'
                wire = None if token == '0x00000000' else by_token.get(token)
                require(token == '0x00000000' or wire is not None, 'Unresolved starter item token.')
                require(wire is None or faction in by_wire[wire]['factions'], 'Foreign faction starter item.')
                slots.append(wire)
            require(slots[0] is not None and by_wire[slots[0]]['primary_ammo'] is not None, 'Starter primary has no qualified magazine capacity.')
            kits.append(dict(update_id=1, items=slots))
        require(len(kits) == 3, 'Expected three authored starter inventories.')
        bits = sorted({wire & 1023 for kit in kits for wire in kit['items'] if wire is not None})
        kits += [dict(update_id=1, items=[None] * 22) for _ in range(2)]
        factions[faction] = dict(faction=faction, available_item_bits=bits, skill_ids=[], loadouts=kits)
    require(set(factions) == {0, 1, 2}, 'Missing selected starter faction.')
    experience = one(inputs['headers'], 'ExperienceTable')
    require(len(rows(experience, 'Levels', 'ExperienceLevel')) == spec.level_count, 'Unexpected progression level count.')
    armory = dict(base, items=sorted(items, key=lambda row: row['wire_id']), factions=[factions[index] for index in range(3)])
    progression = dict(base, table=normalize(experience), creation_settings=dict(
        root_defaults=dict(StartingLevel=1, AdditionalSkillPoints=0, AdditionalLeadershipPoints=0),
        provenance='Explicit local bootstrap operator policy; not recovered original creation defaults.'))
    outputs, members = {}, {}
    for role, document in [('armory', armory), ('progression', progression)]:
        name = f'retail-v212-bootstrap-{role}.json'
        payload = encode(document)
        require(len(payload) <= 2 * 1024 * 1024, 'Normalized catalog exceeds runtime byte bound.')
        outputs[name] = payload; members[role] = dict(file=name, sha256=sha(payload))
    manifest = dict(base, format='mag-retail-v212-bootstrap-v1', catalogs=members, policy=POLICY, qualification=QUALIFICATION)
    outputs['retail-v212-bootstrap-profile.json'] = encode(manifest)
    return outputs


def write_outputs(output, outputs):
    output = output.resolve()
    require(not output.exists(), 'Output directory already exists; refusing overwrite.')
    require(output.parent.is_dir(), 'Output parent directory must already exist.')
    if output.is_relative_to(ROOT):
        require(output.is_relative_to(ROOT / 'local'), 'Catalog outputs must stay outside public source paths.')
    temporary = Path(tempfile.mkdtemp(prefix='.v212-bootstrap-', dir=output.parent))
    try:
        for name, payload in outputs.items():
            require(Path(name).name == name, 'Output member must be a basename.')
            (temporary / name).write_bytes(payload)
        require(not output.exists(), 'Output directory appeared; refusing overwrite.')
        os.rename(temporary, output)
    finally:
        if temporary.exists(): shutil.rmtree(temporary)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('engine', 'headers', 'classes', 'equipment'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--output-dir', type=Path, required=True)
    args = parser.parse_args(argv)
    inputs = {}
    for name in ('engine', 'headers', 'classes', 'equipment'):
        path = getattr(args, name)
        require(path.is_file() and path.stat().st_size <= (128 if name == 'engine' else 32) * 1024 * 1024,
                'Source missing or exceeds byte budget: ' + name)
        inputs[name] = path.read_bytes()
    outputs = render(inputs)  # No filesystem mutation until all checks succeed.
    write_outputs(args.output_dir, outputs)
    print(json.dumps({'profile': SUPPORTED.name, 'outputs': {name: sha(data) for name, data in outputs.items()}}))


if __name__ == '__main__':
    main()
