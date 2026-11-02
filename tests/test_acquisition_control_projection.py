"""Invented conditional script and graph inputs; never original Lua or tokens."""
import copy
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest

ROOT=Path(__file__).resolve().parents[1]
SPEC=importlib.util.spec_from_file_location('control_projection',ROOT/'tools/extraction/project-retail-acquisition-control.py')
API=importlib.util.module_from_spec(SPEC)
if Path(SPEC.origin).exists():SPEC.loader.exec_module(API)


def invented():
    names=[f'p{i}' for i in range(12)]
    body=['function ObjectiveControl('+', '.join(names)+')']
    for i in range(4):
        body += [f'if (ZScript.Mission_GetObjectiveState(p{i*2}) == ZScript.ObjectiveState.kObjectiveStateCompleted) then',
                 f'ZScript.Mission_EnableObjective(p{i*2+1})',
                 # A deliberate cross-chain volume target must survive projection.
                 'ZScript.DamageVolume(p8, 0, 0, 0, 0, ZScript.DamageType.Invented, ZScript.DamageSourceType.Invented)',
                 f'elseif (ZScript.Mission_GetObjectiveState(p{i*2}) == ZScript.ObjectiveState.kObjectiveStateDeactivated) then',
                 f'ZScript.Mission_DisableObjective(p{i*2+1})','end']
    body += ['ZScript.UnboundClock(8, 9)','end']
    script=('\n'.join(body)+'\n').encode()
    chains=[];parameters=[]
    for i in range(4):
        vehicle=301+i
        chains.append(dict(chain=str(i//2+1)+('a' if i%2==0 else 'b'),spawn_objective_token=hex(100+i*2),
            arrival_objective_token=hex(101+i*2),vehicle_spawn_token=hex(vehicle),kill_volume_token=hex(200+i),
            spawn_definition=dict(class_name='ObjectiveSecureAndHoldProperties',fields={
                'ForwardSpawnPoint':dict(token=hex(vehicle)),
                'CompletedScript':dict(token=hex(999)),'DeactivatedScript':dict(token=hex(999))}),
            arrival_definition=dict(class_name='ObjectiveArrivalProperties',fields={
                'VehicleSpawnPoint':dict(token=hex(vehicle)),
                'CompletedScript':dict(token=hex(999)),'DeactivatedScript':dict(token=hex(999))})))
    for i in range(12):
        parameters.append(dict(index=i,label=names[i],object_token=hex(100+i if i<8 else 200+i-8)))
    proof=dict(schema='mag_acquisition_arrival_adjacent_frontier_v1',
        status='ALL_THREE_MAP_SCRIPT_AND_ESCORT_LINKS_BOUND_RETAIL_ARRIVAL_GUARDS_PARTIAL',
        original_game_sha256='a'*64,scripts={'gametype_128.lua':dict(member='invented.lua',sha256=hashlib.sha256(script).hexdigest())},
        maps=[dict(map=dict(map='invented',mode_enum='GameType128'),source=dict(archive_sha256='b'*64,clv_sha256='c'*64),
            script_instance_name_token=hex(999),script_function='ObjectiveControl',parameters=parameters,chains=chains)])
    spawn=dict(format='mag_spawn_catalog',schema_version=1,match_profile=dict(game_type=1,attacking_faction=0),
        source=json.dumps(dict(map='invented',game_archive_sha256='a'*64,level_archive_sha256='b'*64,level_clv_sha256='c'*64)),
        active=dict(map_name='invented'),maps=[dict(name='invented',locations=[dict(token=301+i) for i in range(4)])])
    return spawn,proof,script


class ControlProjectionTests(unittest.TestCase):
    def project(self, spawn, proof, script):
        return API.project(API.encode(spawn),API.encode(proof),script)

    def test_ordered_groups_resolve_external_identities_for_all_factions(self):
        spawn,proof,script=invented()
        for faction in range(3):
            spawn['match_profile']['attacking_faction']=faction
            out=self.project(spawn,proof,script)
            self.assertEqual(out['source']['spawn_catalog_sha256'],hashlib.sha256(API.encode(spawn)).hexdigest())
            self.assertEqual(out['chains'][3],dict(platoon=1,stage='b',capture_token=106,arrival_token=107,vehicle_spawn_token=304,volume_token=203))
            self.assertEqual(out['groups'][2]['branches'][0]['commands'][1],dict(kind='damage_volume',target=200,arguments=[0,0,0,0],damage_type_symbol='DamageType.Invented',damage_source_symbol='DamageSourceType.Invented'))
            self.assertEqual(out['unbound_calls'],[dict(symbol='UnboundClock',arguments=[8,9])])
            self.assertEqual(out,self.project(spawn,proof,script))

    def test_comments_and_debug_strings_do_not_become_commands(self):
        _,_,script=invented()
        edited=script.replace(b'function ObjectiveControl',b'-- header\nfunction ObjectiveControl').replace(
            b'ZScript.Mission_EnableObjective(p1)',b'ZScript.DebugPrint("text -- inside string") -- comment\nZScript.Mission_EnableObjective(p1) -- comment')
        self.assertEqual(API.parse_script(script),API.parse_script(edited))

    def test_unsupported_grammar_and_calls_fail_instead_of_dropping_behavior(self):
        _,_,script=invented()
        for bad in [script.replace(b'elseif',b'else if',1),script.replace(b'ZScript.Mission_EnableObjective(p1)',b'ZScript.UnknownMutation(p1)',1),
                    script.replace(b'ZScript.Mission_EnableObjective(p1)',b'while true do',1),script+b'ZScript.Other(1)\n',
                    script.replace(b'kObjectiveStateCompleted',b'kObjectiveStateUnknown',1)]:
            with self.subTest(script=bad[:60]):
                with self.assertRaises(ValueError):API.parse_script(bad)

    def test_foreign_or_inconsistent_proof_joins_are_refused(self):
        spawn,proof,script=invented()
        variants=[]
        bad=copy.deepcopy(proof);bad['maps'][0]['chains'][0]['arrival_definition']['fields']['VehicleSpawnPoint']['token']='0xffff';variants.append(bad)
        bad=copy.deepcopy(proof);bad['maps'][0]['parameters'][0]['object_token']='0xffff';variants.append(bad)
        bad=copy.deepcopy(proof);bad['maps'][0]['source']['clv_sha256']='f'*64;variants.append(bad)
        bad=copy.deepcopy(proof);bad['scripts']['gametype_128.lua']['sha256']='f'*64;variants.append(bad)
        for bad in variants:
            with self.assertRaises(ValueError):self.project(spawn,bad,script)
        spawn['maps'][0]['locations'].pop()
        with self.assertRaises(ValueError):self.project(spawn,proof,script)


if __name__=='__main__':unittest.main()
