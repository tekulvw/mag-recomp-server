"""SYNTHETIC projector contracts; no original resources, native instructions or captures.

Where success needs a hash-qualified input, a local module instance is repinned
only to newly invented bytes. Production CLIs offer no hash-override switch.
"""
import hashlib
import importlib.util
import json
from pathlib import Path
import struct
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[1]
TOOLS = ROOT / 'tools/extraction'
NAMES = (
    'extract-structure-catalog', 'project-character-blast-rules',
    'project-retail-combat', 'project-retail-bootstrap',
    'compare-native-item-catalog', 'extract-retail-falling-damage',
    'extract-retail-skill-purchases',
)


def load(name):
    spec = importlib.util.spec_from_file_location('synthetic_' + name.replace('-', '_'), TOOLS / (name + '.py'))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def encode(value):
    return json.dumps(value, allow_nan=False).encode()


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def archive_bytes(resources):
    payloads = [('\n'.join(resources) + '\n').encode(), *resources.values()]
    count = len(payloads)
    toc = 32 + 30 * count + count
    raw = bytearray(struct.pack('>4sI4sIIIII', b'PSAR', 0x10003, b'zlib', toc, 30, count, 256, 0))
    offset = toc
    for i, value in enumerate(payloads):
        raw.extend(bytes(16) + struct.pack('>I', i) + len(value).to_bytes(5, 'big') + offset.to_bytes(5, 'big'))
        offset += len(value)
    raw.extend(bytes(len(value) for value in payloads))
    raw.extend(b''.join(payloads))
    return bytes(raw)


class NativeProjectorTests(unittest.TestCase):
    def test_all_commands_help_without_external_inputs(self):
        for name in NAMES:
            with self.subTest(name=name):
                result = subprocess.run([sys.executable, '-B', str(TOOLS / (name + '.py')), '--help'], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertIn('--output' if name != 'extract-retail-falling-damage' else '--config-output', result.stdout)

    def test_missing_arguments_deny_without_writing(self):
        for name in NAMES:
            with self.subTest(name=name):
                result = subprocess.run([sys.executable, '-B', str(TOOLS / (name + '.py'))], capture_output=True, text=True)
                self.assertEqual(result.returncode, 2, result.stderr)
                self.assertIn('required', result.stderr)

    def test_structure_projection_reads_external_native_rules(self):
        tool = load('extract-structure-catalog')
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            assets = root / 'scripts'
            for name in tool.SOURCES:
                path = assets.parent / name if name.startswith('settings/') else assets / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text('Invented = {}')
            (assets / 'vehicles.zpr').write_text('VehicleTable = { Vehicle_Types = {} }')
            (assets / 'active_items.lua').write_text('ActiveItemDataTable = { repairkit_invented = "4" }')
            (assets / 'character_skills.zpr').write_text('MagCharacterSkillTable = { Skills = {} }')
            items = root / 'items.json'
            items.write_bytes(encode({'items': {'31': {'definition': {'group': 'active_items', 'name': 'repairkit'}}},
                'definitions': {'active_items': {'repairkit': {'parameters': {'invented': 7}}}, 'hitscan_ammo': {}, 'projectile_ammo': {}}}))
            rules = root / 'rules.json'
            native = {'health_condition_threshold_percent': [81., 52., 23.], 'dead_below_health': 2., 'minimum_rebuilt_health_percent': 21.}
            rules.write_bytes(encode(native))
            result = json.loads(tool.render(assets, items, rules))
            self.assertEqual(result['native_rules'], native)
            self.assertEqual(result['repair']['item_id'], 31)
            self.assertEqual(result['source_sha256']['vehicles.zpr'], sha((assets / 'vehicles.zpr').read_bytes()))
            native['health_condition_threshold_percent'].reverse()
            rules.write_bytes(encode(native))
            with self.assertRaisesRegex(ValueError, 'invalid_external_native_rules'):
                tool.render(assets, items, rules)

    def test_blast_requires_exact_qualified_witnesses(self):
        tool = load('project-character-blast-rules')
        with self.assertRaisesRegex(ValueError, 'incomplete_witness_set'):
            tool.project({}, Path('unused-checker'))
        with self.assertRaisesRegex(ValueError, 'unreviewed_witness'):
            tool.project({name: b'{}' for name in tool.WITNESS_SHA256}, Path('unused-checker'))

    def test_blast_projects_independent_synthetic_witnesses(self):
        tool = load('project-character-blast-rules')
        start, end = 0x108b7dc, 0x108b9cc  # Supported-build layout metadata only.
        words = {address: 0 for address in range(start, end, 4)}
        words.update({0x108b7dc: 64, 0x108b960: 68, 0x108b968: 2, 0x108b93c: 10})
        callback = {'original_elf_sha256': tool.RETAIL_SHA256, 'start': f'{start:x}', 'end_exclusive': f'{end:x}',
            'instructions': [{'address': f'{address:x}', 'word': f'{value:08x}'} for address, value in words.items()],
            'instruction_bytes_sha256': sha(b''.join(value.to_bytes(4, 'big') for value in words.values()))}
        def cell(address, value, build):
            raw = struct.pack('>f', value)
            return {'original_elf_sha256': build, 'width': 4, 'reads': [{'address': f'{address:08x}', 'raw_hex': raw.hex(), 'value_be': int.from_bytes(raw, 'big')}]}
        with tempfile.TemporaryDirectory() as directory:
            checker = Path(directory) / 'checker.py'
            checker.write_text('# Invented checker identity only\n')
            finding = {'retail_literal': {'interpreted_toc': '1000'}, 'dev_literal': {}, 'scope': 'SYNTHETIC',
                'prepared_rating_reduction': .125, 'prepared_prevention_reduction': .375, 'prepared_hit_location': 2, 'prepared_damage_type': 10}
            inputs = {
                'blast-callback.json': encode(callback),
                'toc-cells.json': encode(cell(0x1040, .125, tool.RETAIL_SHA256)),
                'blast-armor-literal.json': encode(cell(0x1044, .375, tool.RETAIL_SHA256)),
                'dev/dev-blast-armor-literal.json': encode(cell(0x1b9cef4, .375, tool.DEV_SHA256)),
            }
            report = {'dev_elf_sha256': tool.DEV_SHA256, 'retail_elf_sha256': tool.RETAIL_SHA256,
                'checker_sha256': sha(checker.read_bytes()), 'blast_prepared_armor_inputs': finding,
                'witnesses': [{'path': name, 'sha256': sha(raw)} for name, raw in inputs.items() if not name.startswith('dev/')]}
            inputs['comparison.json'] = encode(report)
            tool.WITNESS_SHA256 = {name: sha(raw) for name, raw in inputs.items()}
            output = tool.project(inputs, checker)
            self.assertEqual(output['character_damage']['armor_rating_reduction'], .125)
            self.assertEqual(output['character_damage']['armor_prevention_reduction'], .375)
            self.assertEqual(output['character_damage']['hit_location'], 2)
            checker.write_text('# Changed synthetic checker\n')
            with self.assertRaisesRegex(ValueError, 'unreviewed_original_checker'):
                tool.project(inputs, checker)

    def test_falling_reads_synthetic_elf_archive_and_rejects_changed_sources(self):
        tool = load('extract-retail-falling-damage')
        raw = bytearray(256 + 0x400 + 4)
        raw[:7] = b'\x7fELF\x02\x02\x01'
        struct.pack_into('>H', raw, 18, 21)
        struct.pack_into('>Q', raw, 32, 64)
        struct.pack_into('>HH', raw, 54, 56, 2)
        struct.pack_into('>IIQQQQQQ', raw, 64, 1, 5, 256, 0x7cba00, 0, 0x400, 0x400, 4)
        threshold = 0x1B35B20 + 0x20000 - 0x1C - 0x294 - 0x8000
        struct.pack_into('>IIQQQQQQ', raw, 120, 1, 4, 256 + 0x400, threshold, 0, 4, 4, 4)
        for address, value in [(0x7CBD3C, 3), (0x7CBA6C, 5), (0x7CBA7C, 1)]:
            struct.pack_into('>I', raw, 256 + address - 0x7cba00, value)
        struct.pack_into('>f', raw, 256 + 0x400, 6.5)
        arc = archive_bytes({'/run/mag/ps3/settings/server.zpr': b'Invented = { FallingDamageMinHeight = "2.5", FallingDamageFactor = "7" }'})
        with tempfile.TemporaryDirectory() as directory:
            elf, archive = Path(directory) / 'invented-elf', Path(directory) / 'invented-archive'
            elf.write_bytes(raw)
            archive.write_bytes(arc)
            tool.SOURCES = {'retail': sha(raw), 'archive': sha(arc)}
            result = tool.produce(elf, archive)
            self.assertEqual((result['min_height'], result['damage_factor'], result['minimum_damage_exclusive']), (2.5, 7, 6.5))
            self.assertEqual(result['jump_fall_state'], 3)
            elf.write_bytes(raw + b'x')
            with self.assertRaisesRegex(ValueError, 'SHA-256 mismatch'):
                tool.produce(elf, archive)

    def test_falling_rejects_nonfinite_and_duplicate_authored_scalars(self):
        tool = load('extract-retail-falling-damage')
        class InventedElf:
            def read(self, address, length):
                return struct.pack('>f', float('nan')), 0, 4
            def word(self, address):
                return 1
        for text in [b'A = { FallingDamageMinHeight = "2" }',
                     b'A = { FallingDamageMinHeight = "2", FallingDamageFactor = "7" }']:
            with self.assertRaises(ValueError):
                tool.project(InventedElf(), text)

    def test_comparator_denies_incomplete_and_unattested_inputs(self):
        tool = load('compare-native-item-catalog')
        report = tool.compare(b'{}', b'{}', b'{}')
        self.assertFalse(report['validated'])
        self.assertNotIn('binding_configuration', report)
        with self.assertRaisesRegex(ValueError, 'owner_attestation_required'):
            tool._attestation({'schema_version': 1, 'capture_kind': 'live_read_only', 'identity_verified_by_owner': False}, 'unused')
        with self.assertRaisesRegex(ValueError, 'duplicate_json_key'):
            tool._unique([('invented', 1), ('invented', 2)])

    def test_bootstrap_checks_slot_hash_and_caller_faction_mappings(self):
        tool = load('project-retail-bootstrap')
        with self.assertRaisesRegex(ValueError, 'gear_slots_hash_mismatch'):
            tool._slots(b'Invented enum')
        names = ['Primary', 'Backup', 'Swipe'] + ['Invented' + str(i) for i in range(19)]
        text = '\n'.join(f'DW_AT_name ("kItemSlot{name}") DW_AT_const_value ({i})' for i, name in enumerate(names))
        text += '\nDW_AT_name ("kNumItemSlots") DW_AT_const_value (22)'
        raw = text.encode()
        tool.GEAR_SLOTS_SHA256 = sha(raw)
        slots, width = tool._slots(raw)
        self.assertEqual(width, 22)
        self.assertEqual(slots['Swipe'], 2)
        with self.assertRaisesRegex(ValueError, 'duplicate_faction_mapping'):
            tool.project_multiple(b'', b'', b'', b'', raw, [(0, 'CharacterClassInfo71'), (0, 'CharacterClassInfo72')], b'', {0: ['invented']})

    def test_combat_rejects_nonfinite_and_unqualified_proof(self):
        tool = load('project-retail-combat')
        with self.assertRaisesRegex(ValueError, 'nonfinite_json'):
            tool.parse(b'{"invented": NaN}')
        with self.assertRaisesRegex(ValueError, 'invalid_numeric_parameter'):
            tool.number({'invented': float('inf')}, 'invented')
        selection = {group: ['invented'] for group in ('bullets', 'melee', 'grenades', 'healing')}
        policy = {'projectile_motion': 'authenticated_owner_reports', 'grenade_token_basis': 'expected_static_original_resources',
            'healing_tick_policy': 'guaranteed_ticks_with_bounded_catch_up', 'projectile_expiry_grace_ms': 17, 'healing_tick_seconds': 1}
        with self.assertRaisesRegex(ValueError, 'unreviewed_binding_report'):
            tool.project(b'{}', b'{}', b'{}', b'{}', b'{}', selection, policy)
        policy['healing_tick_seconds'] = -1
        with self.assertRaisesRegex(ValueError, 'invalid_runtime_policy'):
            tool.project(b'{}', b'{}', b'{}', b'{}', b'{}', selection, policy)

    def test_combat_projects_synthetic_catalog_and_explicit_policy(self):
        tool = load('project-retail-combat')
        names = ['invented_firearm', 'invented_swipe', 'invented_frag', 'invented_heal']
        items = {str(i): {'name': None} for i in range(667)}
        for i, name in enumerate(names):
            group = 'active_items' if i == 3 else 'weapons'
            items[str(i)] = {'name': name, 'definition': {'group': group, 'name': name}, 'source': {'invented': True}}
        damage = {key: 13. for key in ('character', 'characterAPierce', 'characterARedux', 'vehicle',
            'vehicleAPierce', 'vehicleARedux', 'objective', 'objectiveAPierce', 'objectiveARedux', 'mine')}
        bullet = {'ammo_name': 'invented_hitscan', 'effective_range': 8., 'max_range': 16.,
            'magazine_capacity': 17, 'initial_magazine_count': 3, 'initial_fire_mode': 'kSemiAuto',
            'infinite_ammo': False, 'fire_mode': {'refire_time': .25, 'reload_time': 2.}}
        melee = {**bullet, 'category': 'kWepCategoryKnife', 'initial_fire_mode': 'kSwipe', 'max_angle': 27., 'max_height': 2.}
        frag = {**bullet, 'ammo_name': 'invented_projectile'}
        ammo = {'damage': damage, 'full_damage_radius': 9., 'half_damage_radius': 4., 'detonation_timer': 3.,
            'm_fLifetimeMax': 6., 'detonation_strategy': 'invented', 'explosion_strategy': 'invented',
            **{key: 2. for key in tool.PHYSICS_FIELDS}}
        def definition(parameters, token=123):
            return {'parameters': parameters, 'source': {'invented': True}, 'native_token_u32': token}
        resource_names = ('weapons.lua', 'hitscanammo.lua', 'projectileammo.lua', 'armor_items.lua', 'damagemodel.lua', 'collision_info.lua', 'active_items.lua', 'item_list.zpr')
        catalog = {'schema_version': 2, 'application_id': 20231, 'items': items,
            'build': {'source_archive_sha256': '0' * 64}, 'sources': {name: {'sha256': '0' * 64} for name in resource_names},
            'definitions': {'weapons': {names[0]: definition(bullet), names[1]: definition(melee), names[2]: definition(frag)},
                'hitscan_ammo': {'invented_hitscan': definition({'damage': damage})},
                'projectile_ammo': {'invented_projectile': definition(ammo)}, 'armor_items': {},
                'active_items': {names[3]: definition({'cooldown_time': 3.})}},
            'resources': {
                'weapons.lua': {'parameters': {'WeaponDataTable': {'MeleeDamageScalarFront': 2., 'MeleeDamageScalarSide': 3., 'MeleeDamageScalarBack': 4.}}},
                'collision_info.lua': {'parameters': {'CharacterCollisionData': {'CharacterStyles': {'CharacterStyle0': {'CharacterStyle': {
                    'm_bodyData': {f'CollisionBodyInfo{i}': {'CollisionBodyInfo': {'armorArea': i % 4, 'hitpointMult': 1. + i / 32, 'incapMult': .37 + i / 64}} for i in range(16)}
                }}}}}},
                'active_items.lua': {'parameters': {'ActiveItemDataTable': {names[3] + '_heal_percent': 17., names[3] + '_heal_duration': 4.}}},
            }}
        catalog_raw = encode(catalog)
        report = {'validated': True, 'positions_compared': 667, 'mismatch_count': 0, 'mismatches': [],
            'catalog_sha256': sha(catalog_raw), 'snapshot_sha256': '1' * 64, 'scope': 'SYNTHETIC',
            'binding_configuration': {'application_id': 20231, 'original_retail_elf_sha256': tool.RETAIL_ELF_SHA256,
                'by_authored_ref': {str(i): i for i in range(667)}}}
        bootstrap = {'application_id': 20231, 'provenance': {'catalog_sha256': sha(catalog_raw), 'snapshot_sha256': report['snapshot_sha256']},
            'factions': [{'loadouts': [{'items': list(range(4))}]}]}
        binding_raw, bootstrap_raw, bullet_raw = encode(report), encode(bootstrap), b'{"synthetic":true}'
        melee_raw = encode({'facing': {'front_dot_threshold': -.25, 'back_dot_threshold': .25}})
        tool.BINDING_REPORT_SHA256, tool.BOOTSTRAP_SHA256, tool.BULLET_WITNESS_SHA256 = map(sha, (binding_raw, bootstrap_raw, bullet_raw))
        selection = {group: [name] for group, name in zip(('bullets', 'melee', 'grenades', 'healing'), names)}
        selection['melee_witness_sha256'] = sha(melee_raw)
        policy = {'projectile_motion': 'authenticated_owner_reports', 'grenade_token_basis': 'expected_static_original_resources',
            'healing_tick_policy': 'guaranteed_ticks_with_bounded_catch_up', 'projectile_expiry_grace_ms': 17, 'healing_tick_seconds': 1}
        result = tool.project(catalog_raw, binding_raw, bootstrap_raw, bullet_raw, melee_raw, selection, policy)
        self.assertEqual(result['runtime_policy'], policy)
        self.assertEqual(result['bullets']['0']['magazine_capacity'], 17)
        self.assertEqual(result['grenades']['2']['fuse_ms'], 3000)
        self.assertEqual(result['healing']['3']['heal_percent'], 17.)
        self.assertEqual(len(result['body']), 16)
        self.assertEqual(result['body'][7]['incapacitated_multiplier'], .479375)
        self.assertEqual(result['body'][7]['multiplier'], 1.21875)
        catalog['definitions']['weapons'][names[0]]['parameters']['effective_range'] = 32.
        changed = encode(catalog)
        with self.assertRaisesRegex(ValueError, 'catalog_hash_mismatch'):
            tool.project(changed, binding_raw, bootstrap_raw, bullet_raw, melee_raw, selection, policy)

    def test_purchases_project_synthetic_lookup_and_missing_token_skip(self):
        tool = load('extract-retail-skill-purchases')
        items = {'invented_ref': {'name': 'invented_award'}}
        items_raw = encode({'application_id': 20231, 'items': items})
        snapshot_raw = encode({'counts': [16, 220, 207, 224], 'lists': [{'tokens_hex': ''}]})
        report_raw = encode({'validated': True, 'positions_compared': 667, 'mismatch_count': 0,
            'snapshot_sha256': sha(snapshot_raw), 'catalog_sha256': sha(items_raw),
            'binding_configuration': {'first_match_by_name': {'invented_award': 117}, 'by_authored_ref': {str(i): i for i in range(667)}}})
        skill = {'schema_version': 2, 'application_id': 20231,
            'source_sha256': {'game.psarc': tool.ARCHIVE_SHA, 'character_skills.zpr': tool.SKILL_MEMBER_SHA,
                'gameplay/data/retail-items.json': sha(items_raw)},
            'table': {'Tiers': {'InventedTier': {'MagCharacterSkillTier': {'Category': 'invented'}}},
                'Skills': {'MagCharacterSkill17': {'MagCharacterSkill': {'Tier': 2, 'Cost': 9, 'Category': 'invented',
                    'Default': {'Equipment': {'A': {'element': 'invented_award'}, 'B': {'element': 'invented_missing'}}}}}}},
            'equipment_token_item_refs': {'invented_award': ['invented_ref']}}
        sources = {
            'gameplay/evidence/retail-skill-unlocks-native-20261026.json': encode({'equipment_lookup_flag': 1, 'available_item_bit_mask': 1023}),
            'gameplay/data/retail-skills.json': encode(skill), 'gameplay/data/retail-items.json': items_raw,
            'gameplay/data/retail-item-bindings.json': report_raw, 'gameplay/data/retail-itemlist-native-order.json': snapshot_raw,
            'gameplay/evidence/dev-itemlist-globalindex-49242c-20261024.json': b'{"synthetic":true}',
        }
        tool.PINS = {name: sha(raw) for name, raw in sources.items()}
        result = tool.project(sources)
        self.assertEqual(result['equipment_token_item_ids']['invented_award'], [117])
        self.assertEqual(result['equipment_tokens_without_item_ids'], ['invented_missing'])
        self.assertEqual(result['table']['Skills']['MagCharacterSkill17']['MagCharacterSkill']['Cost'], 9)

    def test_purchases_require_exact_proof_bundle(self):
        tool = load('extract-retail-skill-purchases')
        with self.assertRaisesRegex(ValueError, 'source_hash_mismatch'):
            tool.project({name: b'{}' for name in tool.PINS})
        with self.assertRaisesRegex(ValueError, 'duplicate_json_key'):
            tool.unique([('invented', 1), ('invented', 2)])


if __name__ == '__main__':
    unittest.main()
