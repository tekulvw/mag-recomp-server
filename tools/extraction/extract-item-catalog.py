#!/usr/bin/env python3
"""Read the prototype's literal Lua/ZPR equipment data without executing Lua.

All authored tables are retained. Numeric/boolean/vector strings are normalized,
with their exact source literals retained by JSON pointer. Unknown syntax fails
closed. --check regenerates in memory and requires byte-identical catalog output.
"""
import argparse
from collections import Counter
from dataclasses import dataclass
import hashlib
import json
import math
from pathlib import Path
import re
import sys
import tempfile
import zlib

REPO = Path(__file__).resolve().parents[2]
RETAIL_ARCHIVE_HASH = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
ELF_HASH = 'efea4d5ef0d14915c75382a56c23f5006fd6916b4080b8a386de01603358a09d'
GROUPS = {
    'weapons.lua': ('weapons', 'WeaponData'),
    'hitscanammo.lua': ('hitscan_ammo', 'HitscanAmmoData'),
    'projectileammo.lua': ('projectile_ammo', 'ProjectileAmmoData'),
    'attachments.lua': ('attachments', 'AttachmentData'),
    'active_items.lua': ('active_items', 'ActiveItemData'),
    'passive_items.lua': ('passive_items', 'PassiveItemData'),
    'simple_items.lua': ('simple_items', 'SimpleItemData'),
    'armor_items.lua': ('armor_items', 'ArmorItemData'),
    'uniform_items.lua': ('uniform_items', 'UniformItemData'),
    'tracers.lua': ('tracers', 'TracerData'),
}
ITEM_GROUPS = {
    'kWeapon': 'weapons', 'kAttachmentItem': 'attachments',
    'kActiveItem': 'active_items', 'kPassiveItem': 'passive_items',
    'kSimpleItem': 'simple_items', 'kArmorItem': 'armor_items',
    'kUniformItem': 'uniform_items',
}
NUMBER = r'[+-]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][+-]?\d+)?'
TOKEN = re.compile(r'(?P<space>\s+)|(?P<comment>--\[\[.*?\]\]|--[^\n]*)|'
                   r'(?P<string>"(?:\\.|[^"\\])*"|\'(?:\\.|[^\'\\])*\')|'
                   rf'(?P<number>{NUMBER})|(?P<identifier>[A-Za-z_]\w*)|'
                   r'(?P<symbol>[{}\[\]=,;])', re.S)


@dataclass
class Scalar:
    value: object
    literal: str
    kind: str
    line: int


@dataclass
class Table:
    entries: list
    line: int


class Parser:
    def __init__(self, text, name):
        self.name, self.tokens, self.index = name, [], 0
        offset, line = 0, 1
        while offset < len(text):
            token = TOKEN.match(text, offset)
            if not token:
                raise ValueError(f'{name}:{line}: unsupported syntax {text[offset:offset+50]!r}')
            kind = token.lastgroup
            if kind not in ('space', 'comment'):
                self.tokens.append((kind, token.group(), line))
            line += token.group().count('\n')
            offset = token.end()

    def peek(self, offset=0):
        return self.tokens[self.index+offset][1] if self.index+offset < len(self.tokens) else None

    def take(self, expected=None):
        if self.index == len(self.tokens):
            raise ValueError(f'{self.name}: unexpected EOF; expected {expected}')
        token = self.tokens[self.index]
        if expected is not None and token[1] != expected:
            raise ValueError(f'{self.name}:{token[2]}: expected {expected}, got {token[1]}')
        self.index += 1
        return token

    def value(self):
        kind, literal, line = self.take()
        if literal == '{':
            entries = []
            while self.peek() != '}':
                if self.peek() is None:
                    raise ValueError(f'{self.name}:{line}: unterminated table')
                key = None
                if self.peek(1) == '=':
                    key = self.take()[1]
                    self.take('=')
                elif self.peek() == '[':
                    self.take('[')
                    key = self.value()
                    self.take(']')
                    self.take('=')
                entries.append((key, self.value()))
                if self.peek() in (',', ';'):
                    self.take()
                # Shipped ZPR omits separators between some named sub-tables.
            self.take('}')
            return Table(entries, line)
        if kind == 'string':
            return Scalar(decode_string(literal), literal, kind, line)
        if kind == 'number':
            return Scalar(number(literal), literal, kind, line)
        if kind == 'identifier':
            value = {'true': True, 'false': False, 'nil': None}.get(literal, {'$lua_identifier': literal})
            return Scalar(value, literal, kind, line)
        raise ValueError(f'{self.name}:{line}: unsupported value {literal!r}')

    def parse(self):
        entries = []
        while self.peek() is not None:
            key = self.take()[1]
            self.take('=')
            entries.append((key, self.value()))
            if self.peek() in (',', ';'):
                self.take()
        return Table(entries, 1)


def decode_string(literal):
    escapes = {'a': '\a', 'b': '\b', 'f': '\f', 'n': '\n', 'r': '\r',
               't': '\t', 'v': '\v', '\\': '\\', '"': '"', "'": "'"}
    def escape(match):
        text = match[1]
        if text.isdigit():
            value = int(text)
            if value > 255:
                raise ValueError(f'invalid Lua byte escape {text}')
            return chr(value)
        if text in escapes:
            return escapes[text]
        if text == '\n':
            return '\n'
        raise ValueError(f'unsupported Lua string escape {text!r}')
    return re.sub(r'\\(\d{1,3}|.)', escape, literal[1:-1], flags=re.S)


def number(text):
    value = float(text) if any(c in text for c in '.eE') else int(text)
    if isinstance(value, float) and not math.isfinite(value):
        raise ValueError(f'non-finite authored number {text}')
    return value


def pointer(path, key):
    return path + '/' + str(key).replace('~', '~0').replace('/', '~1')


def normalize(node, path='', literals=None):
    if literals is None:
        literals = {}
    if isinstance(node, Scalar):
        value = node.value
        if node.kind == 'string':
            if value in ('true', 'false'):
                value = value == 'true'
            elif re.fullmatch(NUMBER, value):
                value = number(value)
            elif len(value.split()) > 1 and all(re.fullmatch(NUMBER, n) for n in value.split()):
                value = [number(n) for n in value.split()]
            if value != node.value or type(value) is not str:
                literals[path] = node.literal
        elif node.kind == 'number':
            literals[path] = node.literal
        return value
    keys = [key for key, _ in node.entries]
    if all(isinstance(k, str) for k in keys) and len(set(keys)) == len(keys):
        return {key: normalize(value, pointer(path, key), literals) for key, value in node.entries}
    if keys and all(k is None for k in keys):
        return [normalize(value, pointer(path, i), literals) for i, (_, value) in enumerate(node.entries)]
    # Mixed, duplicate-key and explicit-index tables retain entry order and keys.
    entries = []
    for i, (key, value) in enumerate(node.entries):
        p = pointer(pointer(path, '$lua_table'), i)
        k = normalize(key, pointer(p, 'key'), literals) if isinstance(key, Scalar) else key
        entries.append({'key': k, 'value': normalize(value, pointer(p, 'value'), literals)})
    return {'$lua_table': entries}


def walk(node, path=''):
    if not isinstance(node, Table):
        return
    keys = [k for k, _ in node.entries]
    plain = all(isinstance(k, str) for k in keys) and len(set(keys)) == len(keys)
    array = keys and all(k is None for k in keys)
    for i, (key, value) in enumerate(node.entries):
        p = pointer(path, key) if plain else pointer(path, i) if array else pointer(pointer(pointer(path, '$lua_table'), i), 'value')
        yield key, value, p
        yield from walk(value, p)


def fields(node):
    if not isinstance(node, Table):
        raise ValueError('definition is not a table')
    return {key: value.value for key, value in node.entries if isinstance(value, Scalar)}


def sha(data):
    return hashlib.sha256(data).hexdigest()


def extract_item_rows(node, definitions, retail=False):
    """Keep retail list-local identities distinct; never infer their wire mapping."""
    items, namespaces = {}, {}
    for key, child, path in walk(node):
        match = re.fullmatch(r'ItemListData(\d+)', str(key))
        if not match:
            continue
        ordinal = int(match[1])
        namespace = None
        if retail:
            owner = re.fullmatch(r'/ItemList/factionItemLists/(FactionItemList\d+)/items/(?:ItemListData\d+|\$lua_table/\d+/value)', path)
            if owner is None:
                raise ValueError('retail item is outside a faction-list namespace: ' + path)
            namespace = owner[1]
        identity = f'{namespace}:{ordinal}' if retail else str(ordinal)
        if identity in items:
            raise ValueError('duplicate authored item identity: ' + identity)
        namespaces.setdefault(namespace, []).append(ordinal)
        authored = fields(child)
        name, kind = authored.get('id'), authored.get('item_type')
        group = ITEM_GROUPS.get(kind)
        resolved = group is not None and name in definitions[group]
        row = {'item_id': ordinal, 'name': name, 'item_type': kind,
               'definition': {'group': group, 'name': name} if resolved else None,
               'source': {'file': 'item_list.zpr', 'line': child.line, 'pointer': path}}
        if retail:
            del row['item_id']
            row = {'authored_ref': identity, 'faction_list': namespace, 'local_ordinal': ordinal,
                   'wire_item_id': None, **row}
        items[identity] = row
    if not items:
        raise ValueError('empty item list')
    for namespace, ordinals in namespaces.items():
        if sorted(ordinals) != list(range(len(ordinals))):
            raise ValueError(f'item ordinal sequence is not contiguous in {namespace}')
    return items


def extract(assets, retail=False, token_proof_path=None):
    equipment = Parser((assets/'equipment.zpr').read_text(), 'equipment.zpr').parse()
    equipment_table = equipment.entries[0][1]
    equipment_files = set(fields(equipment_table).values())
    if equipment_files != set(GROUPS):
        raise ValueError(f'EquipmentTables changed: {equipment_files ^ set(GROUPS)}')
    names = sorted(equipment_files | {'equipment.zpr', 'item_list.zpr', 'damagemodel.lua', 'collision_info.lua'})
    resources, sources, definitions = {}, {}, {group: {} for group, _ in GROUPS.values()}
    parsed = {}
    for name in names:
        raw = (assets/name).read_bytes()
        node = Parser(raw.decode('utf-8-sig'), name).parse()
        literals = {}
        resources[name] = {'parameters': normalize(node, literals=literals), 'source_literals': literals}
        parsed[name] = node
        sources[name] = {'path': str((assets/name).relative_to(REPO)) if (assets/name).is_relative_to(REPO) else name,
                         'sha256': sha(raw), 'bytes': len(raw)}
        if (assets/name).read_bytes() != raw:
            raise ValueError(f'source changed during extraction: {name}')
        if name in GROUPS:
            group, kind = GROUPS[name]
            for key, child, path in walk(node):
                if key != kind:
                    continue
                identity = fields(child).get('id')
                if not isinstance(identity, str) or not identity:
                    raise ValueError(f'{name}:{child.line}: missing definition id')
                definition_literals = {}
                record = {'parameters': normalize(child, literals=definition_literals),
                    'source_literals': definition_literals, 'source': {'file': name, 'line': child.line, 'pointer': path}}
                if identity in definitions[group]:
                    previous = definitions[group][identity]
                    if 'variants' not in previous:
                        previous = {'variants': [previous]}
                        definitions[group][identity] = previous
                    previous['variants'].append(record)
                else:
                    definitions[group][identity] = record
    items = extract_item_rows(parsed['item_list.zpr'], definitions, retail=retail)
    by_name = {}
    for identity, item in items.items():
        by_name.setdefault(item['name'], []).append(identity if retail else item['item_id'])
    references = []
    reference_fields = {'ammo_name': ['hitscan_ammo', 'projectile_ammo'], 'tracer_id': ['tracers'],
                        'required_sight_attachment': ['attachments'],
                        'armor_dependency': ['armor_items'], 'weapon_name': ['weapons']}
    for group, records in definitions.items():
        for name, record in records.items():
            for variant in record.get('variants', [record]):
                refs = []
                for field, targets in reference_fields.items():
                    value = variant['parameters'].get(field)
                    if value:
                        resolved = [{'group': t, 'name': value} for t in targets if value in definitions[t]]
                        refs.append({'field': field, 'name': value, 'resolved': resolved})
                for list_field, target in [('attachment_list', 'attachments'), ('valid_armor_list', 'armor_items')]:
                    for ref_path, value in nested_names(variant['parameters'].get(list_field, {}), '/' + list_field):
                        resolved = [{'group': target, 'name': value}] if value in definitions[target] else []
                        refs.append({'field': ref_path, 'name': value, 'resolved': resolved})
                if group == 'armor_items':
                    armor_type = variant['parameters']['armor_type']
                    refs.append({'field': 'armor_type', 'name': str(armor_type),
                                 'resolved': [{'group': 'armor_types', 'name': str(armor_type)}]})
                variant['references'] = refs
                references.extend({'definition': {'group': group, 'name': name}, **ref} for ref in refs if not ref['resolved'])
            record['item_refs' if retail else 'item_ids'] = by_name.get(name, [])
    # Explicit armor row indices retain the resource's zero-based row and one-based binding.
    armor_rows = {}
    for key, child, path in walk(parsed['damagemodel.lua']):
        match = re.fullmatch(r'ArmorTypeInfo(\d+)', str(key))
        if match:
            row = int(match[1])
            payload = next(v for k, v in child.entries if k == 'ArmorTypeInfo')
            literals = {}
            armor_rows[str(row+1)] = {'armor_type': row+1, 'native_index': row,
                'parameters': normalize(payload, literals=literals), 'source_literals': literals,
                'source': {'file': 'damagemodel.lua', 'line': payload.line, 'pointer': pointer(path, 'ArmorTypeInfo')}}
    definitions['armor_types'] = armor_rows
    unresolved = [identity if retail else i['item_id'] for identity, i in items.items() if i['definition'] is None]
    orphans = [{'group': g, 'name': n} for g, records in definitions.items() if g != 'armor_types'
               for n, d in records.items() if not d['item_refs' if retail else 'item_ids'] and g not in ('hitscan_ammo', 'projectile_ammo', 'tracers')]
    if not retail:
        if token_proof_path is None:
            raise ValueError('Prototype extraction requires --token-proof from independent native analysis')
        token_proof_raw = token_proof_path.read_bytes()
        token_proof = json.loads(token_proof_raw)
        assert token_proof['source_elf_sha256'] == ELF_HASH
        assert token_proof['token_proof']['all_256_words_match_reflected_polynomial_0xedb88320']
        for profile in token_proof['profiles']:
            assert native_token(profile['authored_ammo'])[1] == int(profile['native_table_token'], 16)
            weapon = definitions['weapons'][profile['weapon']]['parameters']
            ammo = definitions['hitscan_ammo'][weapon['ammo_name']]['parameters']
            assert weapon['ammo_name'] == profile['authored_ammo']
            assert (weapon['effective_range'], weapon['max_range']) == (profile['effective_range'], profile['max_range'])
            assert ammo['damage']['character'] == profile['raw_character_hp']
            assert ammo['damage']['characterAPierce'] == profile['armor_rating_redux']
            assert ammo['damage']['characterARedux'] == profile['armor_prevention_redux']
    for group in ('hitscan_ammo', 'projectile_ammo'):
        for name, record in definitions[group].items():
            canonical, token = native_token(name)
            record['canonical_name'] = canonical
            record['native_token_u32'] = token
            record['native_token_hex'] = f'0x{token:08x}'
    report = {'item_count': len(items), 'assigned_item_count': sum(i['item_type'] is not None for i in items.values()),
              'resolved_item_count': len(items)-len(unresolved),
              'reserved_item_ids': [identity if retail else i['item_id'] for identity, i in items.items() if i['item_type'] is None],
              'unresolved_assigned_item_ids': [i for i in unresolved if items[str(i)]['item_type'] is not None], 'item_types': dict(sorted(Counter(i['item_type'] or '<unassigned>' for i in items.values()).items())),
              'definition_counts': {g: len(d) for g, d in definitions.items()},
              'unresolved_item_ids': unresolved, 'unlisted_item_definitions': orphans,
              'unresolved_references': references,
              'duplicate_item_names': {n: ids for n, ids in by_name.items() if n is not None and len(ids) > 1},
              'duplicate_definitions': [{'group': g, 'name': n, 'variant_count': len(d['variants'])}
                                        for g, records in definitions.items() for n, d in records.items() if 'variants' in d]}
    if retail:
        build = {'name': 'original retail BCUS98110 v1.0', 'platform': 'ps3', 'resource_build_path': '/run/mag/ps3/scripts',
                 'source_archive': 'game.psarc (decrypted)', 'source_archive_sha256': RETAIL_ARCHIVE_HASH}
        derivation = {'algorithm': 'ASCII lowercase, backslash to slash, reflected CRC32 polynomial0xedb88320; init/final XOR0xffffffff',
            'evidence': None, 'limitation': 'Prototype HashString algorithm applied unverified to the retail ELF; Retail ItemListData ordinals restart in each FactionItemList. Namespace-qualified authored references are NOT numeric wire IDs; runtime binding remains unverified.'}
    else:
        build = {'name': 'bundled prototype mage_g', 'platform': 'ps3', 'resource_build_path': 'builds/3/run/mag/ps3/scripts',
                 'source_elf': 'user-supplied prototype ELF', 'source_elf_sha256': ELF_HASH}
        derivation = {'algorithm': 'ASCII lowercase, backslash to slash, reflected CRC32 polynomial0xedb88320; init/final XOR0xffffffff',
            'evidence': token_proof_path.name, 'evidence_sha256': sha(token_proof_raw),
            'limitation': 'Derived HashString baseline only; optional runtime token overrides are not applied or confirmed.'}
    if retail:
        for field in ('reserved_item_ids', 'unresolved_assigned_item_ids', 'unresolved_item_ids'):
            report[field.replace('_ids', '_refs')] = report.pop(field)
        report['namespace_counts'] = {namespace: sum(i['faction_list'] == namespace for i in items.values())
                                      for namespace in sorted({i['faction_list'] for i in items.values()})}
    result = {'schema_version': 2 if retail else 1, 'application_id': 20231 if retail else 20230,
        'build': build, 'token_derivation': derivation,
        'sources': sources, 'items': items, 'definitions': definitions, 'resources': resources, 'report': report,
        'limitations': ['Authored resource data only; extraction does not enable mechanics or infer native formulas.',
                       'No defaults are invented for absent fields. Unresolved item definitions/references remain explicit.',
                       'Native ammo tokens derive from proven baseline HashString; optional runtime overrides remain unknown.']}
    return result


def native_token(name):
    if not name.isascii():
        raise ValueError(f'non-ASCII token needs independent canonicalization evidence: {name!r}')
    canonical = name.lower().replace('\\', '/')
    return canonical, zlib.crc32(canonical.encode('ascii'))


def nested_names(value, path):
    if isinstance(value, dict):
        for key, child in value.items():
            p = pointer(path, key)
            if key == 'name' and isinstance(child, str) and child:
                yield p, child
            else:
                yield from nested_names(child, p)
    elif isinstance(value, list):
        for i, child in enumerate(value):
            yield from nested_names(child, pointer(path, i))


def resolve_pointer(value, path):
    for segment in path.split('/')[1:]:
        key = segment.replace('~1', '/').replace('~0', '~')
        value = value[int(key)] if isinstance(value, list) else value[key]
    return value


def render(value):
    return (json.dumps(value, indent=2, ensure_ascii=False, allow_nan=False) + '\n').encode()


def retail_assets(archive_path, directory):
    """Copy the retail script members the extractor reads out of the hash-checked archive."""
    sys.path.insert(0, str(REPO / 'tools/collision'))
    from archive import Psarc
    raw = archive_path.read_bytes()
    if sha(raw) != RETAIL_ARCHIVE_HASH:
        raise ValueError('wrong retail game.psarc')
    archive = Psarc(raw)
    for name in [*GROUPS, 'equipment.zpr', 'item_list.zpr', 'damagemodel.lua', 'collision_info.lua']:
        (directory / name).write_bytes(archive.extract(archive.names.index('/run/mag/ps3/scripts/' + name) + 1))


def retail_main(args):
    output = args.output
    with tempfile.TemporaryDirectory() as directory:
        retail_assets(args.game_archive, Path(directory))
        data = render(extract(Path(directory), retail=True))
    if args.check:
        if output.read_bytes() != data:
            raise ValueError('retail catalog differs from original source regeneration')
    else:
        output.write_bytes(data)
    catalog = json.loads(data)
    print(json.dumps({'mode': 'check' if args.check else 'extract', 'catalog_sha256': sha(data), **catalog['report']}, indent=2))


def main():
    cli = argparse.ArgumentParser(description=__doc__)
    cli.add_argument('--assets', type=Path, help='prototype scripts directory')
    cli.add_argument('--output', type=Path, required=True)
    cli.add_argument('--elf', type=Path, help='original prototype ELF')
    cli.add_argument('--token-proof', type=Path, help='independent prototype token proof JSON')
    cli.add_argument('--game-archive', type=Path, help='decrypted retail game.psarc: extract the retail catalog instead of the prototype one')
    cli.add_argument('--check', action='store_true', help='require byte-identical regeneration; write nothing')
    args = cli.parse_args()
    sources = [v.resolve() for v in (args.game_archive, args.elf, args.token_proof) if v]
    if args.output.resolve() in sources or (args.assets and args.output.resolve().is_relative_to(args.assets.resolve())):
        cli.error('output must be separate from source inputs')
    if args.game_archive:
        retail_main(args)
        return
    if not all((args.assets, args.elf, args.token_proof)):
        cli.error('prototype mode requires --assets, --elf and --token-proof')
    if sha(args.elf.read_bytes()) != ELF_HASH:
        raise ValueError('prototype ELF identity changed')
    catalog = extract(args.assets.resolve(), token_proof_path=args.token_proof)
    data = render(catalog)
    if args.check:
        if args.output.read_bytes() != data:
            raise ValueError('catalog differs from original source regeneration')
    else:
        args.output.parent.mkdir(parents=True, exist_ok=True)
        args.output.write_bytes(data)
    print(json.dumps({'mode': 'check' if args.check else 'extract', 'catalog_sha256': sha(data),
                      'source_files': len(catalog['sources']), **catalog['report']}, indent=2))


if __name__ == '__main__':
    main()
