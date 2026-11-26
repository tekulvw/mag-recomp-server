#!/usr/bin/env python3
"""Project the collision a retail Sabotage level's objectives place, per objective state.

Sabotage objectives place their own collision when they enter a state: the A/B
consoles (one object per state) and the C server racks with the antenna their
fire-and-forget dependent sequence raises (and its wreck once C is destroyed).
None of it is compiled static geometry, so the static world has none of it.

The output is world-posed: each objective lists, per authored state sequence,
the packs that sequence places itself and the dependent sequences it starts;
each dependent lists its own packs and the dependents it stops. A dependent's
delayed creation is recorded at its end state (the runtime places it when the
state is entered). The runtime replays the indices in docs/sabotage-structures.md.

Original assets are never executed or bundled into the source tree: the output
holds neutral meshes converted from the level's physics packs and must stay in
private bundles.
"""
import argparse
import hashlib
import json
import math
from pathlib import Path
import runpy
import struct


TOOLS = Path(__file__).resolve().parent
STRUCTURAL = runpy.run_path(str(TOOLS / 'project-retail-domination-structural-effects.py'))
SEQUENCE_CLASS = STRUCTURAL['SEQUENCE_CLASS']
STATES = STRUCTURAL['STATES']
SCHEMA = 'mag_sabotage_objective_collision_v1'
OBJECTIVE_CLASSES = ('ObjectiveDemolitionStructuralProperties', 'ObjectiveSecureAndHoldProperties')
CREATE = 'AnimObjectCreationEventDef'
STARTS = ('PlayDependentSequenceEventDef', 'PlayIndependentSequenceEventDef')
STOP = 'StopSequencesEventDef'
# Events that place no collision. Anything else in a followed sequence is refused.
PRESENTATION = frozenset({
    'PrintEventDef', 'ParticleEventDef', 'ParticleAttachedEventDef', 'SoundEventDef',
    'SoundAttachedEventDef', 'ExplosionEventDef', 'AnimEventDef', 'PlayVOEventDef',
    'RotateNodeEventDef'})
IDENTITY = ((1.0, 0.0, 0.0), (0.0, 1.0, 0.0), (0.0, 0.0, 1.0))


def require(condition, message):
    if not condition:
        raise ValueError(message)


def sha(raw):
    return hashlib.sha256(raw).hexdigest()


def _typed(fields, key, kind, size, default=None):
    if key not in fields:
        require(default is not None, f'Missing sequence field {key}')
        return default
    actual, raw = fields[key]
    require(actual == kind and len(raw) == size, f'Wrong sequence field type for {key}')
    return raw


def _float(fields, key, default=None):
    raw = _typed(fields, key, 'float', 4, None if default is None else struct.pack('>f', default))
    value, = struct.unpack('>f', raw)
    require(math.isfinite(value), f'Nonfinite sequence field {key}')
    return value


def _vector(fields, key):
    raw = _typed(fields, key, 'SMath::Vec3', 16, bytes(16))
    xyz = struct.unpack('>3f', raw[:12])
    require(all(map(math.isfinite, xyz)), f'Nonfinite vector {key}')
    return xyz


def _token(fields, key):
    return _typed(fields, key, 'util::Token', 4, bytes(4)).hex()


def _flag(fields, key):
    raw = _typed(fields, key, 'bool', 1, b'\0')
    require(raw in (b'\0', b'\1'), f'Invalid bool {key}')
    return raw == b'\1'


def sequence_actions(raw):
    """Collision-relevant actions of one sequence resource, in authored order.

    Creations keep their event pose; starts keep their transform choice;
    presentation events are dropped; any other event class is refused.
    """
    actions = []
    for event, fields in STRUCTURAL['sequence_event_fields'](raw):
        if event == CREATE:
            start = _float(fields, 'start')
            duration = _float(fields, 'dur')
            require(start >= 0 and duration == -1, 'Unqualified creation lifetime')
            actions.append({'kind': 'create', 'object_token': _token(fields, 'objectName'),
                            'geometry_token': _token(fields, 'geometry'),
                            'collision_token': _token(fields, 'collision'),
                            'position': _vector(fields, 'position'),
                            'rotation': _vector(fields, 'rotation'), 'start_seconds': start})
        elif event in STARTS:
            actions.append({'kind': 'start', 'sequence_token': _token(fields, 'SequenceName'),
                            'use_parent_transform': _flag(fields, 'useParentTransform'),
                            'fire_and_forget': _flag(fields, 'fireAndForget'),
                            'position': _vector(fields, 'position'),
                            'rotation': _vector(fields, 'rotation')})
        elif event == STOP:
            actions.append({'kind': 'stop', 'sequence_token': _token(fields, 'SequencesName')})
        else:
            require(event in PRESENTATION, f'Unsupported sequence event {event}')
    return actions


def axis_rotation(degrees):
    """Event rotation matrix (rows). Only a yaw, or a yaw with both X and Z at
    a half turn, is accepted: those mean one turn about Y under every Euler
    order, so no unproven order is assumed."""
    x, y, z = degrees
    flipped = abs(abs(x) - 180) < 1e-3 and abs(abs(z) - 180) < 1e-3
    require(flipped or abs(x) < 1e-3 and abs(z) < 1e-3, 'Unqualified event rotation')
    yaw = math.radians(180 - y if flipped else y)
    c, s = math.cos(yaw), math.sin(yaw)
    return ((c, 0.0, s), (0.0, 1.0, 0.0), (-s, 0.0, c))


def compose(parent, local):
    (pt, pr), (lt, lr) = parent, local
    translation = tuple(pt[i] + sum(pr[i][k] * lt[k] for k in range(3)) for i in range(3))
    rotation = tuple(tuple(sum(pr[i][k] * lr[k][j] for k in range(3)) for j in range(3))
                     for i in range(3))
    return translation, rotation


def pose_json(pose):
    translation, rotation = pose
    return {'translation': [float(v) for v in translation],
            'rotation': [[float(v) for v in row] for row in rotation]}


def follow_dependents(roots, actions_of):
    """Every sequence reachable from `roots` through starts. A dependent must be
    fire-and-forget in the world frame: the runtime has no parent pose for it."""
    seen, pending = set(), list(roots)
    while pending:
        token = pending.pop()
        for action in actions_of(token):
            if action['kind'] != 'start' or action['sequence_token'] in seen:
                continue
            require(action['fire_and_forget'] and not action['use_parent_transform']
                    and action['position'] == (0, 0, 0) and action['rotation'] == (0, 0, 0),
                    'Dependent sequence needs a parent transform')
            seen.add(action['sequence_token'])
            pending.append(action['sequence_token'])
    return seen


def place(frame, creation, packs, parent_poses):
    """World-posed packs of one creation event under `frame`."""
    event = (creation['position'], axis_rotation(creation['rotation']))
    root = compose(frame, event)
    placed = []
    for pack in packs[creation['collision_token']]:
        pose = root
        if pack['parent_token'] != '00000000':
            parent = parent_poses[creation['geometry_token']][pack['parent_token']]['pose']
            pose = compose(root, (parent['translation'], parent['rotation']))
        placed.append({'object_token': creation['object_token'],
                       'collision_token': creation['collision_token'],
                       'packfile_sha256': pack['source_sha256'],
                       'neutral_sha256': pack['neutral_sha256'],
                       'collision_tag': pack['collision_tag'],
                       'material_indices': pack['material_indices'],
                       'start_seconds': creation['start_seconds'],
                       'pose': pose_json(pose)})
    return placed


def surfaces(actions, frame, packs, parent_poses):
    return [row for action in actions if action['kind'] == 'create'
            and action['collision_token'] != '00000000'
            for row in place(frame, action, packs, parent_poses)]


def standing_surfaces(index, map_name):
    """Standing packs of animated instances (`retail_compiled.py --animated-root-packs`).

    Their meshes are already in world coordinates, so each stands at the
    identity pose for the whole match."""
    require(index.get('schema') == 'mag_animated_root_packs_v1' and index.get('map') == map_name,
            'Wrong animated root pack index')
    require(len(index['packs']) <= 32, 'Excess animated root packs')
    return [{'object_token': row['instance_name'][2:], 'collision_token': row['resource_token'],
             'packfile_sha256': row['packfile_sha256'], 'neutral_sha256': row['neutral_sha256'],
             'collision_tag': row['collision_tag'], 'material_indices': row['material_indices'],
             'start_seconds': 0.0, 'pose': pose_json(((0.0, 0.0, 0.0), IDENTITY))}
            for row in index['packs']]


def build_index(identity, objectives, actions_of, packs, parent_poses, standing=()):
    """Assemble the world-posed index from projected objectives and packs."""
    rows, dependents = [], set()
    for objective in objectives:
        frame = (objective['pose']['translation'], objective['pose']['rotation'])
        sequences = []
        for token in objective['sequence_tokens']:
            actions = actions_of(token)
            require(not any(a['kind'] == 'stop' for a in actions), 'Top-level sequence stops')
            starts = [a['sequence_token'] for a in actions if a['kind'] == 'start']
            dependents.update(follow_dependents([token], actions_of))
            sequences.append({'source_sha256': objective['sequence_sha256'][token],
                              'surfaces': surfaces(actions, frame, packs, parent_poses),
                              'starts': starts})
        rows.append({'objective_token': objective['objective_token'],
                     'class': objective['class'],
                     'definition_sha256': objective['definition_sha256'],
                     'state_sequence_indices': objective['state_sequence_indices'],
                     'sequences': sequences})
    nested = {}
    for token in sorted(dependents):
        actions = actions_of(token)
        nested[token] = {'surfaces': surfaces(actions, ((0.0, 0.0, 0.0), IDENTITY), packs, parent_poses),
                         'starts': [a['sequence_token'] for a in actions if a['kind'] == 'start'],
                         'stops': [a['sequence_token'] for a in actions if a['kind'] == 'stop']}
        require(set(nested[token]['stops']) <= dependents, 'Stop target outside the dependents')
    require(rows and len(rows) <= 32 and len(nested) <= 64, 'Invalid Sabotage structure count')
    require(any(s['surfaces'] for row in rows for s in row['sequences']), 'No placed collision')
    return {'schema': SCHEMA, **identity, 'objectives': rows, 'dependents': nested,
            'standing': list(standing)}


def project(level_path, output, animated_roots=None):
    raw = level_path.read_bytes()
    builder = runpy.run_path(str(TOOLS / 'build-retail-spawn-catalog.py'))
    archive = builder['extractor']().Psarc(raw)
    clvs = [name for name in archive.names if name.endswith('.clv')]
    require(len(clvs) == 1, 'Expected one level CLV')
    clv_name, = clvs
    map_name = clv_name.rsplit('/', 1)[1][:-4]
    sabotage = runpy.run_path(str(TOOLS / 'project-retail-sabotage-objectives.py'))
    profile = sabotage['map_profile'](map_name)
    require(sha(raw) == profile['level_sha256'], 'Wrong original Sabotage level archive')
    block = clv_name.rsplit('/', 1)[0] + '/resource_blocks/'

    def member(name):
        require(archive.names.count(name) == 1, f'Missing level member {name}')
        return archive.extract(archive.names.index(name) + 1)
    clv, data, physics = member(clv_name), member(block + 'globaldata.zrb'), member(block + 'physics.zrb')
    require(sha(clv) == profile['clv_sha256'], 'CLV hash mismatch')
    zrb = runpy.run_path(str(TOOLS.parent / 'collision/zrb_records.py'))
    helpers = runpy.run_path(str(TOOLS / 'retail_map_helpers.py'))
    graph = runpy.run_path(str(TOOLS / 'extract-retail-maps.py'))['objective_graph'](clv, helpers)
    records = zrb['parse'](data)['records']
    sequences = {r['name_token']: r for r in records if r['class_token'] == SEQUENCE_CLASS}
    require(len(sequences) == sum(r['class_token'] == SEQUENCE_CLASS for r in records),
            'Ambiguous sequence resource identity')
    cache = {}

    def actions_of(token):
        require(token in sequences, 'Missing original sequence resource')
        if token not in cache:
            record = sequences[token]
            require(record['resource_version'] == 3, 'Unsupported sequence version')
            cache[token] = sequence_actions(bytes.fromhex(record['unparsed_resource_header_hex']))
        return cache[token]

    definitions = {}
    for definition in graph['definitions']:
        if definition['class_name'] not in OBJECTIVE_CLASSES:
            continue
        tokens, indices = STRUCTURAL['sequence_slots'](definition['fields'], sequences)
        followed = set(tokens) | follow_dependents(tokens, actions_of)
        if not any(a['kind'] == 'create' and a['collision_token'] != '00000000'
                   for token in followed for a in actions_of(token)):
            continue  # Presentation only (the satellite dishes): nothing to collide with.
        definitions[definition['definition_token']] = (definition, tokens, indices)
    objectives = []
    for instance in graph['instances']:
        if instance['definition_token'] not in definitions:
            continue
        definition, tokens, indices = definitions[instance['definition_token']]
        fields = {entry['name']: entry['value'] for entry in instance['fields']}
        translation = [fields['Translate' + axis] for axis in 'XYZ']
        angles = [fields['Rotate' + axis] for axis in 'XYZ']
        require([fields['Scale' + axis] for axis in 'XYZ'] == [1, 1, 1]
                and all(map(math.isfinite, translation + angles)), 'Unqualified objective transform')
        basis = helpers['euler_basis'](angles)
        objectives.append({
            'objective_token': instance['name_token'], 'class': definition['class_name'],
            'definition_sha256': definition['source_sha256'], 'state_sequence_indices': indices,
            'sequence_tokens': tokens,
            'sequence_sha256': {t: sha(bytes.fromhex(sequences[t]['unparsed_resource_header_hex']))
                                for t in tokens},
            'pose': {'translation': translation, 'rotation': [list(r) for r in zip(*basis)]}})

    followed = {t for o in objectives for t in o['sequence_tokens']}
    followed |= follow_dependents(followed, actions_of)
    creations = [a for t in sorted(followed) for a in actions_of(t)
                 if a['kind'] == 'create' and a['collision_token'] != '00000000']
    parsed_physics = zrb['parse'](physics, retail13_compat=True)
    resources = {}
    for record in parsed_physics['records']:
        if record['class_token'] == '2a8bdf5a' and record['name_token'] in {
                c['collision_token'] for c in creations}:
            require(record['name_token'] not in resources and record['packfiles'],
                    'Ambiguous or empty collision resource')
            resources[record['name_token']] = {'packfile_sha256': [p['sha256'] for p in record['packfiles']]}
    require(set(resources) == {c['collision_token'] for c in creations}, 'Unresolved collision resource')
    projection = {'map': map_name, 'collision_resources': resources,
                  'sequences': {t: {'events': [dict(a, kind='create_animated_object')
                                               for a in actions_of(t) if a['kind'] == 'create']}
                                for t in followed}}
    packs, parent_poses = STRUCTURAL['export_collision_packs'](
        projection, data, records, physics, parsed_physics['records'], output)
    # Bundles address files by content: name each neutral mesh by its own hash.
    for rows in packs.values():
        for pack in rows:
            source = output / (pack['source_sha256'] + '.json')
            target = output / (pack['neutral_sha256'] + '.json')
            if source.exists():
                require(sha(source.read_bytes()) == pack['neutral_sha256'], 'Neutral mesh hash mismatch')
                source.rename(target)
            require(target.exists(), 'Missing neutral mesh')
    identity = {'map': map_name, 'level_archive_sha256': sha(raw), 'level_clv_sha256': sha(clv),
                'global_data_sha256': sha(data), 'physics_sha256': parsed_physics['sha256']}
    standing = []
    if animated_roots is not None:
        roots = json.loads((animated_roots / 'index.json').read_text())
        standing = standing_surfaces(roots, map_name)
        for row in standing:
            payload = (animated_roots / (row['neutral_sha256'] + '.json')).read_bytes()
            require(sha(payload) == row['neutral_sha256'], 'Animated root pack hash mismatch')
            (output / (row['neutral_sha256'] + '.json')).write_bytes(payload)
    index = build_index(identity, objectives, actions_of, packs, parent_poses, standing)
    (output / 'index.json').write_text(json.dumps(index, indent=1, allow_nan=False) + '\n')
    return index


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--level-archive', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True, help='new directory outside the checkout')
    parser.add_argument('--animated-root-packs', type=Path,
                        help='retail_compiled.py --animated-root-packs output for this level: '
                             'standing packs of animated instances the static world omits')
    args = parser.parse_args()
    output = args.output.resolve()
    if output.exists() or output.is_relative_to(TOOLS.parents[1]):
        parser.error('Output must be a new directory outside the source checkout')
    project(args.level_archive, output, args.animated_root_packs)


if __name__ == '__main__':
    main()
