"""Invented native-initial-load evidence, deliberately shuffled slot bindings."""
import copy
import importlib.util
import json
from pathlib import Path
import unittest

SPEC=importlib.util.spec_from_file_location('initial_objectives',Path(__file__).resolve().parents[1]/'tools/extraction/project-retail-acquisition-initial-objectives.py')
API=importlib.util.module_from_spec(SPEC)
if Path(SPEC.origin).exists():SPEC.loader.exec_module(API)


def fixture():
    objects=[]
    for t,index in [(1001,63),(1002,17)]:
        objects.append(dict(name_token=hex(t),native_initial_client_binding=dict(entity_family=12,client_pool_slot=index,
            network_type=12,network_index=index,packed_client_entity_id=hex(0x0c800000|index),
            initial_network_state=0,source_model_only=True,runtime_observed=False)))
    squads=[dict(team=team,platoon_id=platoon,squad=squad,squad_index=team*16+platoon*4+squad,
        ordered_source_entries=[dict(objective_token='0x3e9',authored_order=9),dict(objective_token='0x3ea',authored_order=2),dict(objective_token='0x3e9',authored_order=9)])
        for team in range(2) for platoon in range(2) for squad in range(4)]
    proof=dict(schema='mag_acquisition_objective_initial_client_bindings_v1',
        status='ORIGINAL_CLIENT_FACTORY_AND_INITIAL_SUCCESSFUL_LOAD_TOKEN_SLOT_NET12_MODEL_BOUND',
        catalog_sha256='a'*64,original_elf_sha256=dict(retail='b'*64),source_model_binding_count=2,
        maps=[dict(map=dict(map='invented',mode_enum='GameType128'),source=dict(archive_sha256='c'*64,clv_sha256='d'*64),
            native_initial_client_pool=dict(array=12,capacity=128,expected_occupied_slots=[17,63],initial_load_success_scope=True,runtime_observed=False),
            objectives=objects,squads=squads)])
    spawn=dict(format='mag_spawn_catalog',schema_version=1,match_profile=dict(game_type=1),active=dict(map_name='invented'),
        source=json.dumps(dict(map='invented',level_archive_sha256='c'*64,level_clv_sha256='d'*64)),maps=[dict(name='invented')])
    return spawn,proof


class InitialObjectiveTests(unittest.TestCase):
    def test_native_bindings_are_used_and_squad_list_order_duplicates_survive(self):
        spawn,proof=fixture();out=API.project(API.encode(spawn),API.encode(proof))
        self.assertEqual([(o['object_index'],o['token'],o['initial_state']) for o in out['objects']],[(17,'0x000003ea',0),(63,'0x000003e9',0)])
        self.assertEqual(out['squads'][-1],dict(team=1,squad_index=23,initial_objective_index=65535,
            authored_objectives=[dict(object_index=63,order=9),dict(object_index=17,order=2),dict(object_index=63,order=9)]))
        self.assertEqual(out['source']['capture_qualification'],'native_initial_load')
        self.assertNotIn('interaction',out['objects'][0])

    def test_unqualified_mixed_family_wrong_state_and_incomplete_inventory_fail(self):
        spawn,proof=fixture()
        for field,value in [('entity_family',27),('network_index',8),('initial_network_state',1),('source_model_only',False),('runtime_observed',True)]:
            bad=copy.deepcopy(proof);bad['maps'][0]['objectives'][0]['native_initial_client_binding'][field]=value
            with self.assertRaises(ValueError):API.project(API.encode(spawn),API.encode(bad))
        bad=copy.deepcopy(proof);bad['maps'][0]['squads'].pop()
        with self.assertRaises(ValueError):API.project(API.encode(spawn),API.encode(bad))
        bad=copy.deepcopy(proof);bad['maps'][0]['native_initial_client_pool']['expected_occupied_slots']=[17]
        with self.assertRaises(ValueError):API.project(API.encode(spawn),API.encode(bad))
        bad=copy.deepcopy(proof);bad['maps'][0]['source']['clv_sha256']='e'*64
        with self.assertRaises(ValueError):API.project(API.encode(spawn),API.encode(bad))


if __name__=='__main__':unittest.main()
