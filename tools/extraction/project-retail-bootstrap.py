#!/usr/bin/env python3
# Adapted project-owned projector; precursor source SHA256: d92472b0677be0d5e76b1dbfcf87e806c6b9095d1a203494a387862fc0b89141
"""Project a private retail bootstrap config from a fully attested catalog match.

Offline only. Owner attestation is trusted input, never independent process
verification. Synthetic fixture success is not evidence of a live capture.
"""
import argparse
import hashlib
import importlib.util
import json
import os
from pathlib import Path
import re
import sys

GEAR_SLOTS_SHA256 = '8df8c285a76b204be02445298372f4c8cb3775f362424cc601173b140efed956'
_spec = importlib.util.spec_from_file_location('bootstrap_catalog_comparator',
                                             Path(__file__).with_name('compare-native-item-catalog.py'))
_comparator = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_comparator)


def _check(condition, failure):
    if not condition:
        raise ValueError(failure)


def _sha(raw):
    return hashlib.sha256(raw).hexdigest()


def _unique(pairs):
    result = {}
    for key, value in pairs:
        _check(key not in result, 'duplicate_json_key')
        result[key] = value
    return result


def _slots(raw):
    _check(_sha(raw) == GEAR_SLOTS_SHA256, 'gear_slots_hash_mismatch')
    text = raw.decode('utf-8')
    pairs = re.findall(r'DW_AT_name\s*\("(k\w+)"\)\s*DW_AT_const_value\s*\((-?\d+)\)', text)
    values = _unique((name, int(value)) for name, value in pairs)
    width = values['kNumItemSlots']
    _check(width == 22, 'unsupported_gear_slot_count')
    # FirstAttachment is a boundary alias, not an equipment slot.
    slots = {name[len('kItemSlot'):]: value for name, value in values.items()
             if name.startswith('kItemSlot') and name not in ('kItemSlotNone', 'kItemSlotFirstAttachment')}
    _check(len(slots) == width and set(slots.values()) == set(range(width)), 'invalid_gear_slot_ordinals')
    return slots, width


def project(snapshot_bytes, catalog_bytes, attestation_bytes, rules_bytes,
            gear_slots_bytes, class_ref, faction):
    """Return a config only after all667 positions match; raise ValueError otherwise."""
    report = _comparator.compare(snapshot_bytes, catalog_bytes, attestation_bytes)
    _check(report.get('validated') is True and report.get('positions_compared') == 667,
           'catalog_binding_not_validated:' + str(report.get('failure', 'incomplete_match')))
    try:
        catalog = json.loads(catalog_bytes, object_pairs_hook=_unique)
        rules = json.loads(rules_bytes, object_pairs_hook=_unique)
        _check(type(rules['schema_version']) is int and rules['schema_version'] == 1
               and type(rules['application_id']) is int and rules['application_id'] == 20231,
               'invalid_rules_schema')
        _check(rules['item_catalog_sha256'] == _sha(catalog_bytes), 'rules_catalog_hash_mismatch')
        _check(rules['source_archive_sha256'] == catalog['build']['source_archive_sha256'], 'rules_archive_mismatch')
        _check(type(faction) is int and 0 <= faction <= 255, 'invalid_explicit_faction')
        _check(isinstance(class_ref, str) and bool(re.fullmatch(r'CharacterClassInfo\d+', class_ref)), 'invalid_class_ref')
        slots, width = _slots(gear_slots_bytes)
        selected = [row for row in rules['starter_templates'] if row['class_ref'] == class_ref]
        _check(len(selected) == 1, 'class_ref_not_unique_or_missing')
        template = selected[0]
        _check(type(template['inventory_template_count']) is int
               and template['inventory_template_count'] == 3, 'expected_three_starter_templates')
        binding = report['binding_configuration']
        by_name = binding['first_match_by_name']
        by_id = {wire: catalog['items'][ref] for ref, wire in binding['by_authored_ref'].items()}
        loadouts, sources = [], []
        rows = template['equipment_references']
        _check(isinstance(rows, list), 'invalid_equipment_references')
        for row in rows:
            _check(type(row['kit_index']) is int and 0 <= row['kit_index'] < 3, 'invalid_kit_index')
        for kit in range(3):
            items, seen, resolved, provenance = [None] * width, set(), {}, []
            for row in (r for r in rows if r['kit_index'] == kit):
                slot, name = row['slot'], row['equipment_name']
                _check(slot in slots and slot not in seen, 'unknown_or_duplicate_gear_slot')
                seen.add(slot)
                _check(name is None or isinstance(name, str), 'invalid_equipment_name')
                if not name:
                    continue
                _check(name in by_name, 'unresolved_equipment_name:' + name)
                wire = by_name[name]
                _check(type(wire) is int and 0 <= wire < 4096, 'invalid_wire_item_id')
                item = by_id[wire]
                items[slots[slot]] = wire
                resolved[slot] = item
                provenance.append({'slot': slot, 'equipment_name': name,
                                   'resolved_authored_ref': item['authored_ref'],
                                   'authored_item_refs': row.get('authored_item_refs', [])})
            capacities = {}
            for mandatory in ('Primary', 'Backup', 'Swipe'):
                _check(mandatory in resolved, 'missing_required_weapon:' + mandatory)
                item = resolved[mandatory]
                definition = item['definition']
                _check(item['item_type'] == 'kWeapon' and definition['group'] == 'weapons',
                       'required_slot_not_weapon:' + mandatory)
                weapon = catalog['definitions']['weapons'][definition['name']]
                capacity = weapon['parameters'].get('magazine_capacity')
                _check(type(capacity) is int and 1 <= capacity <= 255,
                       'unresolved_or_invalid_magazine_capacity:' + mandatory)
                capacities[mandatory] = capacity
            loadouts.append({'update_id': 1, 'items': items, 'primary_ammo': capacities['Primary']})
            sources.append({'kit_index': kit, 'equipment': provenance})
        loadouts.extend({'update_id': 1, 'items': [None] * width, 'primary_ammo': 0} for _ in range(2))
        return {'schema_version': 1, 'application_id': 20231,
                'provenance': {'review_only': True,
                    'scope': 'Exact supplied owner-attested catalog only; no gameplay acceptance or general hash proof.',
                    'snapshot_sha256': _sha(snapshot_bytes), 'catalog_sha256': _sha(catalog_bytes),
                    'owner_attestation_sha256': _sha(attestation_bytes), 'rules_sha256': _sha(rules_bytes),
                    'gear_slots_sha256': _sha(gear_slots_bytes),
                    'comparator_sha256': _sha(Path(_comparator.__file__).read_bytes()),
                    'original_retail_elf_sha256': binding['original_retail_elf_sha256'],
                    'verified_binding': {'validated': True, 'positions_compared': report['positions_compared'],
                        'blank_positions_compared': report['blank_positions_compared'],
                        'blank_zero_empirically_verified': report['blank_zero_empirically_verified'],
                        'process_provenance': report['process_provenance']},
                    'explicit_faction': faction, 'authored_faction_label': template['faction'],
                    'faction_mapping': 'Caller-selected numeric faction; not inferred from authored enum label.',
                    'starter_templates': sources},
                'factions': [{'faction': faction, 'class_ref': class_ref, 'loadouts': loadouts}]}
    except (KeyError, TypeError, AttributeError, IndexError, UnicodeError) as exc:
        raise ValueError('malformed_projection_input:' + str(exc)) from exc



_FACTION_ENUM_SHA256 = '3eb5ef8fd9edbea34dc2a2fc38cec00953eafd03fd2850dc0ad8a322fee5ba33'


def _faction_enum_evidence(enum_bytes, identities):
    _check(_sha(enum_bytes) == _FACTION_ENUM_SHA256, 'faction_enum_evidence_hash_mismatch')
    values = _unique((name, int(value)) for name, value in re.findall(
        r'DW_AT_name\s*\("(FactionType\w+)"\)\s*DW_AT_const_value\s*\((-?\d+)\)', enum_bytes.decode()))
    _check(set(identities) == {0, 1, 2}, 'expected_three_external_faction_identities')
    for faction, identity in identities.items():
        _check(len(identity) == 4 and all(isinstance(v, str) and v for v in identity), 'invalid_external_faction_identity')
        _check(values.get(identity[0]) == faction, 'faction_enum_evidence_disagreement')
    return {'sha256': _sha(enum_bytes), 'kind': 'Externally supplied named faction enum proof; exact hash qualified.'}


def project_multiple(snapshot_bytes, catalog_bytes, attestation_bytes, rules_bytes,
                     gear_slots_bytes, faction_classes, enum_bytes, identities):
    """Project all three explicit mappings, checked against enum and authored labels.

    The legacy project() function remains unchanged. Numeric factions come from
    named original enum evidence, class identity from the retail Faction member.
    """
    _check(isinstance(faction_classes, (list, tuple)), 'invalid_faction_class_mappings')
    factions, classes, mappings = set(), set(), []
    for row in faction_classes:
        _check(isinstance(row, (list, tuple)) and len(row) == 2, 'invalid_faction_class_mapping')
        faction, class_ref = row
        _check(type(faction) is int and faction in identities, 'invalid_explicit_faction')
        _check(isinstance(class_ref, str) and re.fullmatch(r'CharacterClassInfo\d+', class_ref), 'invalid_class_ref')
        _check(faction not in factions, 'duplicate_faction_mapping')
        _check(class_ref not in classes, 'duplicate_class_mapping')
        factions.add(faction)
        classes.add(class_ref)
        mappings.append((faction, class_ref))
    _check(factions == set(identities), 'expected_three_faction_class_mappings')
    enum_evidence = _faction_enum_evidence(enum_bytes, identities)
    try:
        rules = json.loads(rules_bytes, object_pairs_hook=_unique)
        class_resource = rules['resources']['scripts/character_class.zpr']
        class_rows = class_resource['parameters']['CharacterClassTable']['Classes']
        names_resource = rules['resources']['settings/gameproperties.zpr']
        levels = names_resource['parameters']['GameCreationPropertiesTable']['m_gameCreationTypes']['GameCreationType3']['GameCreationType']['m_gameLevels']
        projected, mapping_sources = [], []
        for faction, class_ref in sorted(mappings):
            label, name, level_name, display_name = identities[faction]
            selected = [row for row in rules['starter_templates'] if row['class_ref'] == class_ref]
            _check(len(selected) == 1, 'class_ref_not_unique_or_missing')
            template = selected[0]
            authored_class = class_rows[class_ref]['CharacterClassInfo']
            _check(template['faction'] == label and authored_class['Faction'] == label,
                   'faction_label_mismatch:' + class_ref)
            _check(template['inventory_template_count'] == 3
                   and len(authored_class['InventoryList']) == 3,
                   'expected_three_starter_templates')
            _check(template['class_type'] == authored_class['Class'] == 'CharacterClassTypeMAG',
                   'expected_mag_starter_class')
            name_rows = [(key, value['GameLevelData']) for key, value in levels.items()
                         if value['GameLevelData']['m_owningFaction'] == label]
            _check(len(name_rows) == 1, 'faction_display_source_not_unique')
            level_ref, name_row = name_rows[0]
            _check(name_row['m_levelName'] == level_name and name_row['m_displayName'] == display_name,
                   'faction_display_label_mismatch')
            config = project(snapshot_bytes, catalog_bytes, attestation_bytes, rules_bytes,
                             gear_slots_bytes, class_ref, faction)
            projected.append(config)
            mapping_sources.append({'faction': faction, 'class_ref': class_ref,
                'authored_faction_label': label, 'display_name': name,
                'class_source': {'file': 'scripts/character_class.zpr',
                    'sha256': class_resource['sha256'],
                    'pointer': '/parameters/CharacterClassTable/Classes/' + class_ref + '/CharacterClassInfo'},
                'display_source': {'file': 'settings/gameproperties.zpr',
                    'sha256': names_resource['sha256'],
                    'pointer': '/parameters/GameCreationPropertiesTable/m_gameCreationTypes/GameCreationType3/GameCreationType/m_gameLevels/' + level_ref + '/GameLevelData',
                    'authored_display_name': display_name, 'authored_level_name': level_name},
                'starter_templates': config['provenance']['starter_templates']})
        excluded = {'explicit_faction', 'authored_faction_label', 'faction_mapping', 'starter_templates'}
        provenance = {key: value for key, value in projected[0]['provenance'].items() if key not in excluded}
        provenance.update({'faction_mapping': 'Explicit numeric mappings checked against named original enum evidence and retail authored class Faction members; never inferred from list order.',
                           'faction_enum_evidence': enum_evidence, 'faction_mappings': mapping_sources,
                           'runtime_use': 'equipment_replication_only', 'combat_authority_enabled': False})
        return {'schema_version': 1, 'application_id': 20231, 'provenance': provenance,
                'factions': [config['factions'][0] for config in projected]}
    except (KeyError, TypeError, AttributeError, IndexError, UnicodeError) as exc:
        raise ValueError('malformed_multi_projection_input:' + str(exc)) from exc


def _faction_class(argument):
    match = re.fullmatch(r'([012]):(CharacterClassInfo\d+)', argument)
    if not match:
        raise argparse.ArgumentTypeError('Expected FACTION:CharacterClassInfoN with faction0,1,2')
    return int(match[1]), match[2]

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('snapshot', 'owner-attestation', 'catalog', 'rules', 'gear-slots', 'output'):
        parser.add_argument('--' + name, required=True, type=Path)
    parser.add_argument('--faction-enum', type=Path, help='qualified external enum text required for multiple factions')
    parser.add_argument('--faction-identities', type=Path, help='external JSON numeric faction -> [enum label, name, level name, display name]')
    parser.add_argument('--class-ref')
    parser.add_argument('--faction', type=int)
    parser.add_argument('--faction-class', type=_faction_class, action='append',
                        help='Repeat for all three explicit mappings, e.g.0:CharacterClassInfo0')
    parser.add_argument('--check', action='store_true', help='Require byte-identical existing output; write nothing')
    args = parser.parse_args()
    if args.faction_class:
        if args.faction_enum is None or args.faction_identities is None:
            parser.error('multiple factions require --faction-enum and --faction-identities')
        if args.class_ref is not None or args.faction is not None:
            parser.error('--faction-class cannot be combined with --class-ref/--faction')
    elif args.class_ref is None or args.faction is None:
        parser.error('Supply --class-ref and --faction, or repeat --faction-class for all three factions')
    try:
        inputs = [args.snapshot, args.catalog, args.owner_attestation, args.rules, args.gear_slots]
        _check(all(args.output.resolve() != path.resolve() for path in inputs + [p for p in (args.faction_enum, args.faction_identities) if p is not None]), 'refusing_input_output_collision')
        raw = [path.read_bytes() for path in inputs]
        config = (project_multiple(*raw, args.faction_class, args.faction_enum.read_bytes(),
                    {int(k): v for k, v in json.loads(args.faction_identities.read_bytes(), object_pairs_hook=_unique).items()}) if args.faction_class
                  else project(*raw, args.class_ref, args.faction))
        encoded = (json.dumps(config, indent=2) + '\n').encode()
        if args.check:
            _check(args.output.read_bytes() == encoded, 'projection_differs')
        else:
            fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(encoded)
    except (ValueError, OSError) as exc:
        print(json.dumps({'projected': False, 'failure': str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({'projected': True, 'review_only': True, 'output': str(args.output)}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
