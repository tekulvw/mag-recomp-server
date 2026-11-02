#!/usr/bin/env python3
# Adapted project-owned projector; precursor source SHA256: d8f9f948f1878a8107cb99b0d1521834ce6bd9bb8e5dad10784f6f155a79da19
"""Project typed authored retail combat data from the retained attested catalog.

Offline only. Item IDs use the all-faction attestation. Selected original retail
bullet, armor and health-normalization witnesses support ordinary hit resolution.
Frag/healing authority uses original resources with explicit local runtime policies;
projectile motion remains owner-reported and live runtime token aliases unmeasured.
"""
import argparse
import hashlib
import json
import math
import os
from pathlib import Path
import sys

BINDING_REPORT_SHA256 = '232cbc052e6eb3f0ad84f6c2554336415c934b1ba5f41b758627fc999dd9788b'
BOOTSTRAP_SHA256 = '35380d1fcb0090f29020c8485de3004f6982f9809c181c928b2a2b89af54857e'
RETAIL_ELF_SHA256 = '0089f0d6d17e48533ce7ff8d943dddd03487b925e7b04a1a055bfb907568694c'
BULLET_WITNESS_SHA256 = 'a53b389df2eebe2356ca6d29882092e9e36d3e1ba0aacbec412031bf280d6f76'
PHYSICS_FIELDS = ('launch_velocity', 'radius', 'capsuleHeight', 'collisionType',
                  'mass', 'friction', 'restitution', 'linearDamping',
                  'linearDampingOnGround', 'angularDamping', 'angularDampingOnGround',
                  'fBounceDampingDefault', 'fBounceDampingCharacter',
                  'propulsion_strategy', 'm_fLoftAngle', 'random_rotation_range')


def check(condition, failure):
    if not condition:
        raise ValueError(failure)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def unique(pairs):
    result = {}
    for key, value in pairs:
        check(key not in result, 'duplicate_json_key:' + key)
        result[key] = value
    return result


def parse(raw):
    return json.loads(raw, object_pairs_hook=unique,
                      parse_constant=lambda value: (_ for _ in ()).throw(ValueError('nonfinite_json:' + value)))


def encode(value):
    return (json.dumps(value, indent=2, allow_nan=False) + '\n').encode()


def number(parameters, name, positive=False):
    value = parameters[name]
    check(type(value) in (int, float) and math.isfinite(value)
          and (value > 0 if positive else value >= 0), 'invalid_numeric_parameter:' + name)
    return value


def byte(parameters, name):
    value = parameters[name]
    check(type(value) is int and 0 < value <= 255, 'invalid_byte_parameter:' + name)
    return value


def duration_ms(parameters, name):
    value = number(parameters, name, positive=True) * 1000
    check(value % 1 == 0 and value < 2**64, 'invalid_millisecond_duration:' + name)
    return int(value)


def project(catalog_bytes, binding_bytes, bootstrap_bytes, bullet_witness_bytes, melee_witness_bytes, selection, runtime_policy):
    """Only the exact retained binding and bootstrap evidence is accepted.

    Equipment selection is explicit operator input; changing selection requires
    independent qualification against the supplied witnesses. It is not capture.

    The pinned comparison report is trusted evidence, not a remeasurement of its
    original process. Changed evidence needs explicit review and repinning here.
    """
    check(isinstance(selection, dict), 'invalid_selection')
    for group in ('bullets', 'melee', 'grenades', 'healing'):
        names = selection.get(group)
        check(isinstance(names, list) and names and all(isinstance(n, str) and n for n in names)
              and len(names) == len(set(names)), 'invalid_selection:' + group)
    check(isinstance(runtime_policy, dict)
          and runtime_policy.get('projectile_motion') == 'authenticated_owner_reports'
          and runtime_policy.get('grenade_token_basis') == 'expected_static_original_resources'
          and runtime_policy.get('healing_tick_policy') == 'guaranteed_ticks_with_bounded_catch_up'
          and type(runtime_policy.get('projectile_expiry_grace_ms')) is int
          and 0 <= runtime_policy['projectile_expiry_grace_ms'] <= 10_000
          and type(runtime_policy.get('healing_tick_seconds')) is int
          and runtime_policy['healing_tick_seconds'] == 1, 'invalid_runtime_policy')
    check(sha(binding_bytes) == BINDING_REPORT_SHA256, 'unreviewed_binding_report')
    check(sha(bootstrap_bytes) == BOOTSTRAP_SHA256, 'unreviewed_bootstrap')
    check(sha(bullet_witness_bytes) == BULLET_WITNESS_SHA256, 'unreviewed_native_bullet_witness')
    try:
        catalog, report, bootstrap = map(parse, (catalog_bytes, binding_bytes, bootstrap_bytes))
        check(catalog['schema_version'] == 2 and catalog['application_id'] == 20231,
              'unsupported_catalog_schema_or_application')
        check(report['validated'] is True and report['positions_compared'] == 667
              and report['mismatch_count'] == 0 and report['mismatches'] == [], 'incomplete_catalog_comparison')
        check(report['catalog_sha256'] == sha(catalog_bytes), 'catalog_hash_mismatch')
        binding = report['binding_configuration']
        check(binding['application_id'] == 20231
              and binding['original_retail_elf_sha256'] == RETAIL_ELF_SHA256,
              'wrong_original_retail_identity')
        check(bootstrap['application_id'] == 20231
              and bootstrap['provenance']['catalog_sha256'] == sha(catalog_bytes)
              and bootstrap['provenance']['snapshot_sha256'] == report['snapshot_sha256'],
              'bootstrap_catalog_mismatch')
        by_ref = binding['by_authored_ref']
        check(set(by_ref) == set(catalog['items']) and len(by_ref) == 667,
              'incomplete_authored_mapping')
        check(all(type(wire) is int and 0 <= wire < 4096 for wire in by_ref.values())
              and len(set(by_ref.values())) == len(by_ref), 'invalid_wire_item_mapping')
        by_name = {}
        for ref, item in catalog['items'].items():
            if item['name']:
                by_name.setdefault(item['name'], []).append((ref, item))

        def item(name, group):
            candidates = by_name[name]
            check(len(candidates) == 1, 'ambiguous_equipment_name:' + name)
            ref, row = candidates[0]
            check(row['definition'] == {'group': group, 'name': name}, 'unresolved_definition:' + name)
            definition = catalog['definitions'][group][name]
            return str(by_ref[ref]), definition, {
                'authored_ref': ref, 'name': name,
                'definition': {'group': group, 'name': name},
                'source': definition['source'], 'item_source': row['source']}

        def ammo(weapon, group):
            name = weapon['parameters']['ammo_name']
            definition = catalog['definitions'][group][name]
            token = definition['native_token_u32']
            check(type(token) is int and 0 < token < 2**32, 'invalid_candidate_ammo_token')
            return definition, {'group': group, 'name': name, 'source': definition['source'],
                                'ammo_token_candidate': token,
                                'ammo_token_verified_for_retail': False}

        def damage(parameters):
            values = parameters['damage']
            return {key: number(values, key) for key in (
                'character', 'characterAPierce', 'characterARedux', 'vehicle',
                'vehicleAPierce', 'vehicleARedux', 'objective', 'objectiveAPierce',
                'objectiveARedux', 'mine')}

        bullets = {}
        for name in selection['bullets']:
            wire, weapon, out = item(name, 'weapons')
            ammunition, ammo_source = ammo(weapon, 'hitscan_ammo')
            p = weapon['parameters']
            effective, maximum = number(p, 'effective_range'), number(p, 'max_range', True)
            check(effective <= maximum, 'invalid_range_order')
            out.update({'ammo': ammo_source, 'damage': damage(ammunition['parameters']),
                        'effective_range': effective, 'max_range': maximum,
                        'magazine_capacity': byte(p, 'magazine_capacity'),
                        'initial_magazine_count': byte(p, 'initial_magazine_count')})
            # Missing or unsupported schedules remain usable for human-reported
            # damage, but cannot authorize autonomous fire. No timing fallback.
            mode = p.get('initial_fire_mode')
            schedule = p.get('fire_mode', {})
            if (mode in ('kAuto', 'kSemiAuto') and type(p.get('infinite_ammo')) is bool
                    and all(type(schedule.get(k)) in (int, float) and math.isfinite(schedule[k])
                            and 0 < schedule[k] <= 60 for k in ('refire_time', 'reload_time'))):
                out['firearm'] = {'initial_fire_mode': mode,
                                  'refire_seconds': schedule['refire_time'],
                                  'reload_seconds': schedule['reload_time'],
                                  'infinite_ammo': p['infinite_ammo']}
            bullets[wire] = out

        melee = {}
        for name in selection['melee']:
            wire, weapon, out = item(name, 'weapons')
            ammunition, ammo_source = ammo(weapon, 'hitscan_ammo')
            w = weapon['parameters']
            check(w['category'] == 'kWepCategoryKnife' and w['initial_fire_mode'] == 'kSwipe', 'unsupported_melee_category')
            out.update({'ammo': ammo_source, 'damage': damage(ammunition['parameters']),
                        'effective_range': number(w, 'effective_range'), 'max_range': number(w, 'max_range', True),
                        'gear_slot': 2, 'fire_mode': 0, 'category': w['category'], 'damage_type': 3,
                        'max_angle': number(w, 'max_angle'), 'max_height': number(w, 'max_height'),
                        'refire_seconds': number(w['fire_mode'], 'refire_time')})
            melee[wire] = out
        check(sha(melee_witness_bytes) == selection['melee_witness_sha256'], 'unreviewed_native_melee_witness')
        melee_witness = parse(melee_witness_bytes)
        weapon_globals = catalog['resources']['weapons.lua']['parameters']['WeaponDataTable']
        melee_direction = {key: melee_witness['facing'][key] for key in ('front_dot_threshold', 'back_dot_threshold')}
        melee_direction.update({key + '_multiplier': number(weapon_globals, 'MeleeDamageScalar' + key.title())
                                for key in ('front', 'side', 'back')})
        grenades = {}
        for name in selection['grenades']:
            wire, weapon, frag = item(name, 'weapons')
            ammunition, ammo_source = ammo(weapon, 'projectile_ammo')
            p, w = ammunition['parameters'], weapon['parameters']
            full, half = number(p, 'full_damage_radius', True), number(p, 'half_damage_radius', True)
            check(full > half, 'invalid_frag_radius_order')
            fuse, lifetime = duration_ms(p, 'detonation_timer'), duration_ms(p, 'm_fLifetimeMax')
            check(fuse <= lifetime, 'fuse_exceeds_authored_lifetime')
            frag.update({'ammo': ammo_source, 'damage': damage(p), 'full_radius': full,
                         'half_radius': half, 'fuse_ms': fuse, 'authored_lifetime_ms': lifetime,
                         'initial_count': byte(w, 'initial_magazine_count'),
                         'effective_range': number(w, 'effective_range'),
                         'max_range': number(w, 'max_range', True),
                         'detonation_strategy': p['detonation_strategy'],
                         'explosion_strategy': p['explosion_strategy'],
                         'physics': {key: p[key] for key in PHYSICS_FIELDS}})
            grenades[wire] = frag

        armor = {}
        for name in sorted(catalog['definitions']['armor_items']):
            wire, definition, out = item(name, 'armor_items')
            armor_type = definition['parameters']['armor_type']
            check(type(armor_type) is int and 1 <= armor_type <= 7, 'invalid_armor_type')
            row = catalog['definitions']['armor_types'][str(armor_type)]
            p = row['parameters']
            coverage = p['m_coverage']
            check(isinstance(coverage, list) and len(coverage) == 4
                  and all(type(v) in (int, float) and math.isfinite(v) and 0 <= v <= 1 for v in coverage),
                  'invalid_armor_coverage')
            out.update({'armor_type': armor_type, 'rating': number(p, 'rating'),
                        'prevention': number(p, 'prevention'), 'coverage': coverage,
                        'armor_type_source': row['source']})
            armor[wire] = out

        body_pointer = '/parameters/CharacterCollisionData/CharacterStyles/CharacterStyle0/CharacterStyle/m_bodyData'
        bodies = catalog['resources']['collision_info.lua']['parameters']['CharacterCollisionData']['CharacterStyles']['CharacterStyle0']['CharacterStyle']['m_bodyData']
        check(set(bodies) == {f'CollisionBodyInfo{i}' for i in range(16)}, 'unexpected_body_count')
        body = []
        for location in range(16):
            p = bodies[f'CollisionBodyInfo{location}']['CollisionBodyInfo']
            check(type(p['armorArea']) is int and 0 <= p['armorArea'] < 4, 'invalid_body_armor_area')
            body.append({'location': location, 'armor_area': p['armorArea'],
                         'multiplier': number(p, 'hitpointMult'),
                         'incapacitated_multiplier': number(p, 'incapMult'),
                         'source': {'file': 'collision_info.lua',
                                    'pointer': body_pointer + f'/CollisionBodyInfo{location}/CollisionBodyInfo'}})

        globals_ = catalog['resources']['active_items.lua']['parameters']['ActiveItemDataTable']
        healing = {}
        for name in selection['healing']:
            wire, definition, out = item(name, 'active_items')
            out.update({'cooldown_seconds': number(definition['parameters'], 'cooldown_time'),
                        'heal_percent': number(globals_, name + '_heal_percent'),
                        'duration_seconds': number(globals_, name + '_heal_duration', True),
                        'global_source': {'file': 'active_items.lua', 'pointer': '/parameters/ActiveItemDataTable',
                                          'fields': [name + '_heal_percent', name + '_heal_duration']}})
            if name == 'medkit':
                out['recipient_sphere'] = {
                    'forward_offset': number(globals_, 'medkit_heal_range'),
                    'radius': number(globals_, 'medkit_heal_radius', True),
                    'source': {'file': 'active_items.lua',
                               'pointer': '/parameters/ActiveItemDataTable',
                               'fields': ['medkit_heal_range', 'medkit_heal_radius'],
                               'sha256': catalog['sources']['active_items.lua']['sha256']}}
            healing[wire] = out

        used = {int(wire) for table in (bullets, melee, grenades, armor, healing) for wire in table}
        equipped = {v for faction in bootstrap['factions'] for row in faction['loadouts'] for v in row['items'] if v is not None}
        unsupported = {str(wire): {'name': catalog['items'][ref]['name'], 'authored_ref': ref,
                                  'reason': 'No projected combat profile; retain existing equipment replication only.'}
                       for ref, wire in by_ref.items() if wire in equipped - used}
        resources = ('weapons.lua', 'hitscanammo.lua', 'projectileammo.lua', 'armor_items.lua',
                     'damagemodel.lua', 'collision_info.lua', 'active_items.lua', 'item_list.zpr')
        return {'schema_version': 4, 'application_id': 20231,
                'status': 'retail bullets, owner-reported frag blasts and guaranteed healing ticks; live acceptance separate',
                'runtime_policy': runtime_policy,
                'authority': {key: key in ('bullet_damage', 'armor_reduction', 'body_damage', 'grenade_damage', 'healing_schedule', 'melee_damage') for key in ('bullet_damage', 'melee_damage', 'grenade_damage', 'grenade_physics',
                              'armor_reduction', 'body_damage', 'healing_schedule', 'ammo_tokens')},
                'provenance': {'catalog_sha256': sha(catalog_bytes), 'binding_report_sha256': sha(binding_bytes),
                    'bootstrap_sha256': sha(bootstrap_bytes),
                    'selection_sha256': sha(encode(selection)), 'runtime_policy_sha256': sha(encode(runtime_policy)),
                    'native_bullet_witness_sha256': BULLET_WITNESS_SHA256,
                    'native_melee_witness_sha256': sha(melee_witness_bytes),
                    'source_archive_sha256': catalog['build']['source_archive_sha256'],
                    'original_retail_elf_sha256': RETAIL_ELF_SHA256,
                    'source_snapshot_sha256': report['snapshot_sha256'],
                    'binding_positions_compared': 667, 'binding_mismatches': 0,
                    'binding_scope': report['scope'],
                    'sources': {name: catalog['sources'][name] for name in resources},
                    'projector_sha256': sha(Path(__file__).read_bytes())},
                'bullets': bullets, 'melee': melee, 'melee_direction': melee_direction, 'grenades': grenades, 'armor': armor, 'body': body,
                'healing': healing, 'unsupported': unsupported,
                'limitations': [
                    'Item identities use the exact retained owner-attested comparison report; this tool does not repeat its process capture.',
                    'Canonical retail CRC is statically verified; runtime ammo aliases and initialized ammo-list tokens remain unmeasured. This bullet resolver does not consume or accept ammo tokens.',
                    'Ordinary firearm source/range, armor and health normalization use selected original retail witnesses. Complete native admission, indirect hooks and nonneutral state branches remain outside this scope.',
                    'Projectile physics values are authored inputs; no native collision, bounce, gravity or integration formula is supplied.',
                    'Healing cadence is explicit external server policy; no exact native frame-loss behavior is claimed.',
                    'Medkit recipient sphere offset/radius are authored metadata only; native anchor, forward basis, recipient collision query and visibility remain unverified. No autonomous teammate admission is enabled.',
                    'Only explicitly selected attested firearms have bullet profiles; unprojected equipment remains unsupported.',
                    'Frag creation uses the construction-derived token with the original selected resources/override map; no observed runtime row or arbitrary later overrides are claimed.']}
    except (KeyError, TypeError, AttributeError, IndexError, UnicodeError) as exc:
        raise ValueError('malformed_projection_input:' + str(exc)) from exc


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    for name in ('catalog', 'bindings', 'bootstrap', 'bullet-witness', 'melee-witness', 'selection', 'runtime-policy', 'output'):
        parser.add_argument('--' + name, type=Path, required=True)
    parser.add_argument('--check', action='store_true')
    args = parser.parse_args()
    try:
        inputs = (args.catalog, args.bindings, args.bootstrap, args.bullet_witness, args.melee_witness, args.selection, args.runtime_policy)
        check(all(args.output.resolve() != path.resolve() for path in inputs), 'input_output_collision')
        raw = [path.read_bytes() for path in inputs]
        result = encode(project(*raw[:5], parse(raw[5]), parse(raw[6])))
        if args.check:
            check(args.output.read_bytes() == result, 'projection_differs')
        else:
            fd = os.open(args.output, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
            with os.fdopen(fd, 'wb') as stream:
                stream.write(result)
    except (ValueError, OSError) as exc:
        print(json.dumps({'projected': False, 'failure': str(exc)}), file=sys.stderr)
        return 1
    print(json.dumps({'projected': True, 'authority_enabled': ['melee_damage', 'bullet_damage', 'armor_reduction', 'body_damage', 'grenade_damage', 'healing_schedule'], 'check': args.check,
                      'output': str(args.output)}))
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
