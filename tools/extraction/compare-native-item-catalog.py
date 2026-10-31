#!/usr/bin/env python3
# Adapted project-owned projector; precursor source SHA256: 6170b28f016ecb0ba4bd44e81467eb23d82755f0d0fab863ccee2a90405feed4
"""Offline, owner-attested comparison; never changes production catalogs.

Candidate name tokens use the maintained extractor's prototype-derived baseline.
An exact supplied live snapshot match validates only these catalog inputs, not a
universal retail hash algorithm, constructor default, or gameplay acceptance.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import runpy

BUILD_SHA = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
ARCHIVE_SHA = 'b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f'
ITEM_SOURCE_SHA = '29059ed028e13e2dd4e5d19fcaa9dafdde93013b8e2678f97f359df5e0984dee'
COUNTS = [16, 220, 207, 224]
NAMESPACES = [f'FactionItemList{i}' for i in range(4)]
_TOKEN = runpy.run_path(str(Path(__file__).with_name('extract-item-catalog.py')))['native_token']


def _check(condition, reason):
    if not condition:
        raise ValueError(reason)


def _unique(pairs):
    result = {}
    for key, value in pairs:
        _check(key not in result, 'duplicate_json_key')
        result[key] = value
    return result


def _sha(raw):
    return hashlib.sha256(raw).hexdigest()


def _u32(value):
    return type(value) is int and 0 <= value < 2**32


def _snapshot(value):
    _check(type(value['schema_version']) is int and value['schema_version'] == 1 and value['ok'] is True
           and value['read_phase'] == 'complete', 'incomplete_snapshot_schema')
    _check(value['build']['elf_sha256'] == BUILD_SHA, 'snapshot_build_mismatch')
    expected = {'getter_toc': 0x1B45B04, 'cache_base_cell': 0x1B3FC9C,
                'cache_base_expected': 0x1DC3750,
                'cached_index_address': 0x1DC4320, 'cached_generation_address': 0x1DC4322,
                'resolver': '00223920', 'lookup_global_cell': 0x1B31F88,
                'itemlist_pointer_adjustment': 0, 'token_encoding': 'concatenated_be_u32_hex'}
    _check(all(value['provenance'].get(k) == v
               and type(value['provenance'].get(k)) is type(v) for k, v in expected.items()),
           'snapshot_provenance_mismatch')
    _check(value['counts'] == COUNTS and value['total_items'] == 667,
           'snapshot_count_mismatch')
    _check(type(value['logical_read_bytes']) is int and type(value['read_budget_bytes']) is int
           and value['logical_read_bytes'] == 2730
           and 2730 <= value['read_budget_bytes'] <= 16384, 'snapshot_read_budget_mismatch')
    _check(type(value['cache_base']) is int and value['cache_base'] == 0x1DC3750,
           'snapshot_cache_base_mismatch')
    handle, lookup, pointer = value['cached_handle'], value['lookup'], value['itemlist_pointer']
    _check(type(handle['index']) is int and 0 < handle['index'] <= 65535
           and type(handle['generation']) is int and 0 <= handle['generation'] <= 65535,
           'snapshot_cached_handle_invalid')
    _check(_u32(pointer) and pointer > 65535 and lookup['slot'] == pointer
           and lookup['generation'] == handle['generation']
           and _u32(lookup['global']) and lookup['global'] > 0
           and _u32(lookup['table']) and lookup['table'] > 0, 'snapshot_lookup_invalid')
    vector, lists = value['vector'], value['lists']
    _check(vector['count'] == 4 and _u32(vector['begin']) and vector['begin'] > 0
           and _u32(vector['end']) and vector['end'] - vector['begin'] == 96,
           'snapshot_vector_invalid')
    _check(len(lists) == 4, 'snapshot_list_count_invalid')
    for index, row in enumerate(lists):
        _check(type(row['ordinal']) is int and type(row['count']) is int
               and row['ordinal'] == index and row['count'] == COUNTS[index],
               'snapshot_list_order_invalid')
        _check(_u32(row['begin']) and row['begin'] > 0 and _u32(row['end'])
               and row['end'] - row['begin'] == 16 * COUNTS[index],
               'snapshot_item_vector_invalid')
        _check(isinstance(row['tokens_hex'], str)
               and re.fullmatch('[0-9a-f]{' + str(8 * COUNTS[index]) + '}', row['tokens_hex']),
               'snapshot_token_encoding_invalid')
    _check(value['memory_phases'] == ['cached_handle', 'resolved_itemlist', 'list_vector',
           'item_vectors'] + [f'list.{i}.tokens' for i in range(4)], 'snapshot_phases_invalid')


def _attestation(value, snapshot_sha):
    _check(type(value['schema_version']) is int and value['schema_version'] == 1
           and value['capture_kind'] == 'live_read_only'
           and value['identity_verified_by_owner'] is True, 'owner_attestation_required')
    _check(value['snapshot_sha256'] == snapshot_sha and value['guest_elf_sha256'] == BUILD_SHA,
           'attestation_snapshot_or_build_mismatch')
    process = value['process']
    _check(type(process['pid']) is int and process['pid'] > 0
           and isinstance(process['binary_path'], str) and os.path.isabs(process['binary_path'])
           and isinstance(process['binary_sha256'], str)
           and re.fullmatch('[0-9a-f]{64}', process['binary_sha256']),
           'owner_process_provenance_invalid')


def _rows(catalog):
    _check(type(catalog['schema_version']) is int and catalog['schema_version'] == 2
           and catalog['application_id'] == 20231,
           'catalog_schema_mismatch')
    _check(catalog['build']['source_archive_sha256'] == ARCHIVE_SHA
           and catalog['sources']['item_list.zpr']['sha256'] == ITEM_SOURCE_SHA,
           'catalog_source_mismatch')
    tables = catalog['resources']['item_list.zpr']['parameters']['ItemList']['factionItemLists']
    _check(list(tables) == NAMESPACES, 'authored_namespace_order_mismatch')
    items = catalog['items']
    _check(len(items) == 667, 'authored_count_mismatch')
    seen = set()
    for item in items.values():
        _check(item['authored_ref'] not in seen, 'duplicate_authored_ref')
        seen.add(item['authored_ref'])
    rows = []
    for list_index, (namespace, table) in enumerate(tables.items()):
        source = table['items']
        _check(list(source) == [f'ItemListData{i}' for i in range(COUNTS[list_index])],
               'authored_item_order_mismatch')
        for ordinal, literal in enumerate(source.values()):
            ref = f'{namespace}:{ordinal}'
            item = items[ref]
            _check(item['authored_ref'] == ref and item['faction_list'] == namespace
                   and type(item['local_ordinal']) is int and item['local_ordinal'] == ordinal
                   and item['wire_item_id'] is None, 'authored_identity_mismatch')
            _check(item['source']['file'] == 'item_list.zpr'
                   and item['source']['pointer'] == f'/ItemList/factionItemLists/{namespace}/items/ItemListData{ordinal}',
                   'authored_source_pointer_mismatch')
            name = literal.get('id')
            _check(name == item['name'] and literal.get('item_type') == item['item_type'],
                   'authored_literal_mismatch')
            if name is None:
                _check(literal == {}, 'unproved_blank_source_shape')
                token = 0  # Accepted only when this supplied native position also contains zero.
            else:
                _check(isinstance(name, str) and name and name.isascii(), 'unsupported_item_name')
                token = _TOKEN(name)[1]
            rows.append((list_index, ordinal, ref, name, token))
    _check(seen == {row[2] for row in rows}, 'authored_identity_set_mismatch')
    return rows


def compare(snapshot_bytes, catalog_bytes, attestation_bytes):
    """Consume exact JSON file bytes; only a complete attested match yields bindings.

    Owner attestation is trusted input: this offline function never verifies a
    running process. Synthetic tests exercise this contract without live evidence.
    """
    report = {'schema_version': 1, 'validated': False,
              'blank_zero_empirically_verified': False,
              'scope': 'Exact supplied owner-attested snapshot and authored catalog only; '
                       'no general retail hash/default-constructor or gameplay acceptance proof',
              'snapshot_sha256': _sha(snapshot_bytes), 'catalog_sha256': _sha(catalog_bytes),
              'owner_attestation_sha256': _sha(attestation_bytes)}
    try:
        captured, catalog, owner = [json.loads(raw, object_pairs_hook=_unique)
                                   for raw in (snapshot_bytes, catalog_bytes, attestation_bytes)]
        _snapshot(captured)
        _attestation(owner, report['snapshot_sha256'])
        rows = _rows(catalog)
        mismatches = []
        for list_index, ordinal, ref, name, token in rows:
            actual = captured['lists'][list_index]['tokens_hex'][8 * ordinal:8 * ordinal + 8]
            if actual != f'{token:08x}':
                mismatches.append({'authored_ref': ref, 'expected_candidate_token': f'{token:08x}',
                                   'native_token': actual, 'blank': name is None})
        report.update(positions_compared=len(rows),
                      blank_positions_compared=sum(row[3] is None for row in rows),
                      mismatch_count=len(mismatches), mismatches=mismatches[:20])
        if mismatches:
            report['failure'] = 'native_token_order_mismatch'
            return report
        by_ref, positions, names, reserved = {}, {}, {}, []
        for list_index, ordinal, ref, name, token in rows:
            wire_id = ((list_index << 10) | ordinal) + (50 if list_index else 0)
            _check(0 <= wire_id < 4096 and wire_id not in by_ref.values(), 'duplicate_or_invalid_wire_id')
            by_ref[ref] = wire_id
            if name is None:
                reserved.append(ref)
                continue
            _check(token != 0, 'named_null_token_cannot_be_looked_up')
            key = f'{token:08x}'
            positions.setdefault(key, []).append({'authored_ref': ref, 'name': name, 'wire_item_id': wire_id})
            names[name] = key
        aliases = [{'token_hex': token, 'positions': entries,
                    'first_authored_ref': entries[0]['authored_ref'],
                    'first_wire_item_id': entries[0]['wire_item_id']}
                   for token, entries in positions.items() if len(entries) > 1]
        report.update(validated=True, blank_zero_empirically_verified=True, token_aliases=aliases,
                      process_provenance=owner['process'],
                      binding_configuration={'application_id': 20231, 'review_only': True,
                       'source_snapshot_sha256': report['snapshot_sha256'],
                       'source_catalog_sha256': report['catalog_sha256'],
                       'owner_attestation_sha256': report['owner_attestation_sha256'],
                       'original_retail_elf_sha256': BUILD_SHA,
                       'by_authored_ref': by_ref, 'reserved_authored_refs': reserved,
                       'first_match_by_token': {k: v[0]['wire_item_id'] for k, v in positions.items()},
                       'first_match_by_name': {name: positions[token][0]['wire_item_id'] for name, token in names.items()},
                       'formula_evidence': 'original 004B9944/004B9C9C; ordinary inventory flag 0 at004546F0'})
    except (ValueError, KeyError, TypeError, AttributeError, IndexError) as exc:
        report['failure'] = str(exc) if isinstance(exc, ValueError) else 'malformed_input_schema'
    return report


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--snapshot', required=True, type=Path)
    parser.add_argument('--catalog', type=Path, required=True)
    parser.add_argument('--owner-attestation', required=True, type=Path,
                        help='schema1: capture_kind=live_read_only, identity_verified_by_owner=true, '
                        'snapshot_sha256, guest_elf_sha256, process{pid,binary_path,binary_sha256}')
    parser.add_argument('--output', required=True, type=Path, help='new private review report; never a production catalog')
    args = parser.parse_args()
    _check(all(args.output.resolve() != p.resolve() for p in (args.catalog, args.snapshot, args.owner_attestation)), 'input_output_collision')
    report = compare(args.snapshot.read_bytes(), args.catalog.read_bytes(), args.owner_attestation.read_bytes())
    fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    with os.fdopen(fd, 'w', encoding='utf-8') as stream:
        json.dump(report, stream, indent=2)
        stream.write('\n')
    print(json.dumps({'validated': report['validated'], 'output': str(args.output),
                      'failure': report.get('failure')}))
    return 0 if report['validated'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
