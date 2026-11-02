"""Synthetic APC projection evidence. No original game records or resources."""
import copy
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
SPEC = importlib.util.spec_from_file_location('apc_projection', ROOT / 'tools/extraction/project-retail-apc-insertion.py')
API = importlib.util.module_from_spec(SPEC)
if SPEC.origin and Path(SPEC.origin).exists():
    SPEC.loader.exec_module(API)


def f32(value):
    return struct.unpack('<f', struct.pack('<f', value))[0]


def invented(faction=0):
    points, locations, squads, unsupported, fallbacks = [], [], [], [], []
    for i in range(72):
        apc = i in (0, 3, 6, 9)
        basic = i < 48 and not apc and i % 3 != 2
        team = 0 if i < 24 else 1
        p = dict(name_token=hex(1000 + i), catalog_id=f'invented:{i:03}',
                 spawn_type=3 if apc else 0 if basic else 111,
                 user_class=0, category=i % 3, active=True, team=team,
                 vehicle_type_token='0x00001388', scale_xyz=[1, 1, 1], squad_links=[])
        points.append(p)
        locations.append(dict(index=i, token=1000+i, kind='insertion' if basic and i % 3 == 0
                              else 'alternate' if basic else 'unsupported', active=True,
                              position=dict(x=100+i, y=12, z=200)))
        if not basic:
            unsupported.append(dict(index=i, spawn_type=p['spawn_type'], user_class=0,
                                    category=p['category'], active=True, squad_links=[]))
    for s in range(16):
        team, slot = divmod(s, 8)
        i = s * 3
        links = dict(team=team, platoon_id=slot//4, squad=('Alpha','Bravo','Charlie','Delta')[slot%4])
        for j in range(3):
            points[i+j]['squad_links'].append(dict(links, slot=j))
        default = i+1 if i in (0,3,6,9) else i
        squad = dict(index=team*16+slot, team=team, platoon=slot//4, squad=slot%4,
                     faction=None if team == 0 else (faction+1)%3,
                     available=[default] if default != i else [i,i+1], default_spawn=default)
        squads.append(squad)
        if default != i:
            fallbacks.append(dict(squad_index=squad['index'], default_spawn=default, policy='temporary'))
    source = dict(map='invented', game_archive_sha256='c'*64, level_archive_sha256='a'*64,
                  level_clv_sha256='b'*64, unsupported_spawn_links=unsupported,
                  default_spawn_fallbacks=fallbacks)
    spawn = dict(format='mag_spawn_catalog', schema_version=1, source=json.dumps(source),
                 match_profile=dict(game_type=1, attacking_faction=faction),
                 active=dict(map_name='invented', squad_index=0, default_spawn=1, available=[1]),
                 maps=[dict(name='invented', spawn_count=72, runtime_indices_verified=False,
                            owner_faction=(faction+1)%3, locations=locations, squads=squads)])
    authored = dict(map=dict(map='invented', mode_enum='GameType128'), points=points,
                    source=dict(archive_sha256='a'*64, clv_sha256='b'*64))
    proof = b'Invented independently supplied initial-load proof\n'
    bindings = dict(format='mag_spawn_bindings', schema_version=1, map_name='invented',
                    qualification='native_initial_load', level_archive_sha256='a'*64,
                    level_clv_sha256='b'*64, native_elf_sha256='d'*64,
                    proof_sha256=hashlib.sha256(proof).hexdigest(), points=[
                        dict(token=1000+i, index=71-i, position=[100+i, 12.3456789, 200],
                             rotation_xyzw=[0,1,0,0]) for i in range(72)])
    props = dict(GroundVehicleProperties=dict(DamageProperties=dict(MaxHitpoints=str(101+faction)),
        Placement_Y_Offset='0.1', Seats=dict(MaxOccupancy='3', SeatPropertiesList={
            'SeatProperties0':dict(SeatProperties=dict(SeatType='VehicleOperatorSeat')),
            'SeatProperties1':dict(SeatProperties=dict(SeatType='TurretOperatorSeat',SeatCount='2')),
            'SeatProperties2':dict(SeatProperties=dict(SeatType='PassengerSeat',SeatCount='3'))})))
    registry = {'0x00001388':dict(variants=[dict(faction=name, ModifierLevel='0',
        PropertiesFileName=f'synthetic_{f}.zpr') for f,name in enumerate(('Valor','Raven','SVER'))])}
    properties = {f'synthetic_{faction}.zpr':props}
    hashes = {'/run/mag/ps3/scripts/vehicles.zpr':'e'*64,
              f'/run/mag/ps3/scripts/vehicle/synthetic_{faction}.zpr':'f'*64}
    exits = []
    for i in (0,3,6,9):
        origin = [100+i, f32(f32(12.3456789)+f32(.1)), 200]
        exits.append(dict(map='invented', spawn_token=hex(1000+i), faction=faction,
                          vehicle_world_position=origin, rotation_xyzw=[0,1,0,0],
                          qualification='neutral_static_ground_and_clearance',
                          provenance=dict(query_results_sha256='9'*64), candidates=[
                              dict(position_world=[origin[0]-2, origin[1]-.5, 200], ranking_offset_model=[1,0,0]),
                              dict(position_world=[origin[0]+3, origin[1]+.25, 200], ranking_offset_model=[-1,0,0])]))
    return dict(spawn_catalog=spawn, authored_map=authored, bindings=bindings, registry=registry,
                property_trees=properties, property_hashes=hashes, exits=exits, binding_proof=proof)


class ApcProjectionTests(unittest.TestCase):
    def project(self, data=None):
        self.assertTrue(hasattr(API, 'project'), 'APC projector must implement project()')
        return API.project(**(data or invented()))

    def test_complete_reverse_remap_restores_all_authored_apc_defaults(self):
        data=invented(); original=copy.deepcopy(data)
        spawn, ground=self.project(data)
        self.assertEqual(data, original, 'projection must not mutate external evidence')
        pool=spawn['maps'][0]
        self.assertEqual((pool['spawn_count'],len(pool['locations']),len(pool['squads'])), (72,72,16))
        for point in pool['locations']:
            old=point['token']-1000
            self.assertEqual(point['index'],71-old)
            self.assertEqual(point['position']['y'],f32(12.3456789))
            if old in (0,3,6,9):
                self.assertEqual(point['kind'],'vehicle_passenger')
            elif old%3==2 or old>=48:
                self.assertEqual(point['kind'],'unsupported')
        for s in pool['squads']:
            old=(s['index'] if s['team']==0 else s['index']-8)*3
            self.assertEqual(s['default_spawn'],71-old)
            self.assertEqual(s['available'],[71-old,70-old])
        self.assertEqual(spawn['active']['default_spawn'],71)
        self.assertEqual(spawn['active']['available'],[71,70])
        source=json.loads(spawn['source'])
        self.assertEqual(source['default_spawn_fallbacks'],[])
        self.assertEqual([r['index'] for r in source['unsupported_spawn_links']],
                         [71-r['index'] for r in json.loads(original['spawn_catalog']['source'])['unsupported_spawn_links']])
        self.assertTrue(pool['runtime_indices_verified'])
        self.assertIn('initial',pool['runtime_indices_basis'])
        self.assertEqual([v['spawn_point_index'] for v in ground['vehicles']],[62,65,68,71])
        self.assertEqual([v['index'] for v in ground['vehicles']],[0,1,2,3])
        self.assertEqual(ground['source']['spawn_catalog_sha256'],hashlib.sha256(API.encode(spawn)).hexdigest())

    def test_all_factions_property_expansion_f32_once_and_inverse_rotation(self):
        for faction in range(3):
            with self.subTest(faction=faction):
                data=invented(faction); spawn,ground=self.project(data)
                for v in ground['vehicles']:
                    self.assertEqual(v['faction'],faction)
                    self.assertEqual(v['health_quarter_points'],4*(101+faction))
                    self.assertEqual(v['passenger_seats'],[3,4,5])
                    self.assertEqual(v['maximum_occupancy'],3)
                    self.assertEqual(v['position']['y'],f32(f32(12.3456789)+f32(.1)))
                    self.assertNotIn('maximum_mount_distance_m',v)
                    self.assertNotIn('maximum_mount_position_age_ms',v)
                    self.assertEqual(v['passenger_exit_candidates'],[
                        dict(ranking_offset=[1.,0.,0.],position_offset=[2.,-.5,0.]),
                        dict(ranking_offset=[-1.,0.,0.],position_offset=[-3.,.25,0.])])

    def test_bad_evidence_fails_closed(self):
        changes=[('proof',lambda d:d.update(binding_proof=b'wrong')),
            ('qualification',lambda d:d['bindings'].update(qualification='live_guess')),
            ('hash',lambda d:d['bindings'].update(level_clv_sha256='1'*64)),
            ('duplicate',lambda d:d['bindings']['points'][0].update(index=70)),
            ('token',lambda d:d['bindings']['points'][0].update(token=999999)),
            ('range',lambda d:d['bindings']['points'][0].update(index=128)),
            ('quaternion',lambda d:d['bindings']['points'][0].update(rotation_xyzw=[0,0,0,2])),
            ('unqualified exit',lambda d:d['exits'][0].update(qualification='provisional')),
            ('wrong origin',lambda d:d['exits'][0].update(vehicle_world_position=[0,0,0])),
            ('wrong rotation',lambda d:d['exits'][0].update(rotation_xyzw=[0,0,0,1])),
            ('empty exits',lambda d:d['exits'][0].update(candidates=[])),
            ('missing exits',lambda d:d.update(exits=[])),
            ('wrong faction',lambda d:d['exits'][0].update(faction=2)),
            ('nonfinite exits',lambda d:d['exits'][0]['candidates'][0].update(position_world=[float('nan'),1,2]))]
        for name,change in changes:
            with self.subTest(name=name):
                d=invented(); change(d)
                with self.assertRaises(ValueError): self.project(d)

    def test_rejects_preactivated_non_apc_unsupported_kind(self):
        d=invented()
        d['spawn_catalog']['maps'][0]['locations'][2]['kind']='insertion'
        with self.assertRaises(ValueError): self.project(d)

    def test_property_and_exit_provenance_failures(self):
        changes=[lambda d:d['property_hashes'].update({'/run/mag/ps3/scripts/vehicles.zpr':'bad'}),
            lambda d:d['registry']['0x00001388']['variants'].append(copy.deepcopy(d['registry']['0x00001388']['variants'][0])),
            lambda d:d['property_trees']['synthetic_0.zpr']['GroundVehicleProperties']['Seats'].update(MaxOccupancy='4'),
            lambda d:d['property_trees']['synthetic_0.zpr']['GroundVehicleProperties']['Seats']['SeatPropertiesList']['SeatProperties0']['SeatProperties'].update(SeatCount='17'),
            lambda d:d['property_trees']['synthetic_0.zpr']['GroundVehicleProperties']['DamageProperties'].update(MaxHitpoints='1024'),
            lambda d:d['exits'][0].update(provenance={}),
            lambda d:d['exits'].append(copy.deepcopy(d['exits'][0]))]
        for change in changes:
            d=invented(); change(d)
            with self.assertRaises(ValueError): self.project(d)

    def test_cli_requires_supported_original_elf_and_preserves_inputs(self):
        self.assertTrue(hasattr(API,'SUPPORTED_NATIVE_ELF_SHA256'), 'CLI must pin supported original ELF')
        with tempfile.TemporaryDirectory() as tmp:
            base=Path(tmp); inputs={}
            for name in ('game-archive','spawn-catalog','bindings','exits','binding-proof','native-elf'):
                inputs[name]=base/name
                inputs[name].write_bytes(b'invented input')
            command=[sys.executable,SPEC.origin,'--disc',str(base/'disc')]
            for name,path in inputs.items():command.extend(['--'+name,str(path)])
            command.extend(['--spawn-output',str(base/'spawns.json'),'--ground-output',str(base/'ground.json')])
            completed=subprocess.run(command,capture_output=True,text=True)
            self.assertNotEqual(completed.returncode,0)
            self.assertIn('Wrong retail native ELF',completed.stderr)
            self.assertFalse((base/'spawns.json').exists())
            self.assertFalse((base/'ground.json').exists())
            overwrite=command.copy(); overwrite[overwrite.index('--spawn-output')+1]=str(inputs['bindings'])
            completed=subprocess.run(overwrite,capture_output=True,text=True)
            self.assertNotEqual(completed.returncode,0)
            self.assertIn('Outputs must be distinct',completed.stderr)
            self.assertEqual(inputs['bindings'].read_bytes(),b'invented input')

    def test_inactive_apc_and_vehicle_user_remain_unsupported(self):
        for change in ('inactive','vehicle_user'):
            with self.subTest(change=change):
                d=invented()
                p=d['authored_map']['points'][0]
                if change=='inactive':
                    p['active']=False; d['spawn_catalog']['maps'][0]['locations'][0]['active']=False
                else:p['user_class']=1
                d['exits']=d['exits'][1:]
                spawn,ground=self.project(d)
                self.assertEqual(next(p for p in spawn['maps'][0]['locations'] if p['token']==1000)['kind'],'unsupported')
                self.assertEqual(spawn['maps'][0]['squads'][0]['default_spawn'],70)
                self.assertEqual(len(ground['vehicles']),3)


if __name__=='__main__': unittest.main()
