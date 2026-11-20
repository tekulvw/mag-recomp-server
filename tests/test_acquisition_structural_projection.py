"""Synthetic source receipts for the Acquisition Structural interaction projection."""
import hashlib
import importlib.util
import json
from pathlib import Path
import unittest
from unittest.mock import patch

TOOLS = Path(__file__).resolve().parents[1] / 'tools' / 'extraction'
SPEC = importlib.util.spec_from_file_location(
    'acquisition_structural', TOOLS / 'project-retail-acquisition-structural.py')
API = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(API)


def encode(value):
    return json.dumps(value).encode()


def digest(value):
    return hashlib.sha256(encode(value)).hexdigest()


def fixture():
    structural = 'ObjectiveDemolitionStructuralProperties'
    trigger_type = 'deadbeef000000010000000003'
    name = 'invented_acquisition'
    map_info = dict(map=name, mode_enum='GameType128')
    level = dict(archive_sha256='a' * 64, clv_sha256='b' * 64)
    key, secure = '0x00000011', '0x00000012'
    first, second = '0x00000031', '0x00000032'
    structural_values = dict(OwnerTeam=0, CanDeactivate=True, TimeToArmMs=2200,
        TimeToDefuseMs=3100, TimeToDetonateMs=16000, CanRepair=True,
        TimeToRepairMs=37000, MinTimeToRepairMs=8000, ActivationScoreValue=0,
        DeactivationScoreValue=0)
    definitions = [
        dict(definition_token='0x00000021', class_name=structural,
             fields={name: {'value': value} for name, value in structural_values.items()}),
        dict(definition_token='0x00000022', class_name='ObjectiveSecureAndHoldProperties', fields={}),
        dict(definition_token='0x00000023', class_name='ObjectiveTriggerProperties',
             fields=dict(ObjectiveID=dict(raw_hex=key[2:]), TriggerType=dict(raw_hex=trigger_type),
                         ObjectiveTriggerVolume=dict(token='0x00000041'))),
    ]
    instances = [
        dict(name_token=key, definition_token='0x00000021', fields=[
            dict(name='TranslateX', value=20.0), dict(name='TranslateY', value=30.0),
            dict(name='TranslateZ', value=40.0)]),
        dict(name_token=secure, definition_token='0x00000022', fields=[]),
        dict(name_token=first, definition_token='0x00000023', fields=[]),
        dict(name_token=second, definition_token='0x00000023', fields=[]),
    ]
    metadata = dict(maps=[dict(map=map_info, source=level,
                               objectives=dict(definitions=definitions, instances=instances))])
    initial_sha = 'c' * 64
    objective = dict(format='mag_retail_objective_catalog', schema_version=1, game_type=1,
        map_name=name, entity_family=12, network_type=12,
        source=dict(capture_qualification='native_source_gameplay',
                    authored_catalog_sha256=digest(metadata), binding_capture_sha256=initial_sha,
                    startup_proof_sha256='d' * 64, source_image_sha256=API.RETAIL),
        objects=[dict(token=key, object_index=0, initial_state=0),
                 dict(token=secure, object_index=1, initial_state=0, interaction={'existing': True})])
    bindings = dict(schema='mag_acquisition_structural_bindings_v1', map=name,
        metadata_sha256=digest(metadata), initial_objective_sha256=initial_sha,
        level_archive_sha256=level['archive_sha256'], level_clv_sha256=level['clv_sha256'],
        rows={0: dict(token=int(key, 16), **{'class': 'structural'}),
              1: dict(token=int(secure, 16), **{'class': 'secure_hold'})})

    def trigger(trigger_token, index, translation, center):
        volume = dict(trigger_token=trigger_token, objective_token=key, trigger_type_raw_hex=trigger_type,
            objective_class=structural, trigger_position=translation,
            volume_class='TriggerVolumeBoxProperties', volume_token='0x00000041',
            box=dict(centre=center, axes=[[1, 0, 0], [0, 1, 0], [0, 0, 1]], half_extents=[1, 2, 3]))
        return dict(trigger_token=trigger_token, objective_token=key, objective_initial_client_network_index=0,
            client_trigger_index=index, sim_trigger_index=index, family56_level_object_slot=None,
            authored_volume=volume)

    trigger_proof = dict(status='COMMON_TRIGGER_VECTOR_WITNESSES_REPLAYED_ON_ACQUISITION_SOURCE_INPUTS',
        maps=[dict(map=map_info, source=level, triggers=[
            trigger(first, 1, [19, 31, 40], [19, 32, 40]),
            trigger(second, 0, [22, 29, 38], [22, 30, 38]),
        ])])
    schedule = dict(schema='mag_acquisition_objective_sim_schedule_v1',
        status='RETAIL_SIM_SOURCE_ORDER_BOUND', retail_elf_sha256=API.RETAIL,
        metadata_sha256=digest(metadata), client_source_bindings_sha256=initial_sha,
        maps=[dict(map=map_info, source=level, objects=[
            dict(token=key, sim_slot=0, class_name=structural),
            dict(token=secure, sim_slot=1, class_name='ObjectiveSecureAndHoldProperties'),
        ])])
    return metadata, objective, bindings, trigger_proof, schedule


class AcquisitionStructuralProjectionTests(unittest.TestCase):
    def project(self, metadata, objective, bindings, trigger, schedule):
        with patch.multiple(API, METADATA=digest(metadata),
                            TRIGGER_PROOF_SHA256=digest(trigger), SIM_SCHEDULE_SHA256=digest(schedule)):
            return API.project_interactions(
                encode(metadata), encode(objective), bindings, encode(trigger), encode(schedule))

    def test_geometry_and_placement_follow_native_vector_order(self):
        data = fixture()
        catalog, structural = self.project(*data)
        row, secure = catalog['objects']
        self.assertEqual(row['native_class'], 'structural')
        self.assertEqual((row['demolition']['owner_team'], row['demolition']['arm_ms'],
                          row['demolition']['defuse_ms'], row['demolition']['detonate_ms']),
                         (0, 2200, 3100, 16000))
        self.assertEqual(row['repair'], dict(owner_team=0, can_repair=True, repair_ms=37000,
                                             minimum_repair_ms=8000))
        self.assertEqual([box['centre'] for box in row['demolition']['trigger_volumes']],
                         [[22, 30, 38], [19, 32, 40]])
        self.assertEqual(structural['schema'], 'mag_acquisition_structural_bindings_v2')
        self.assertEqual(structural['rows'][0]['trigger_positions'], [[22, 29, 38], [19, 31, 40]])
        self.assertEqual(structural['rows'][1]['trigger_positions'], [])
        self.assertEqual(secure, data[1]['objects'][1])
        self.assertEqual(catalog['source'], data[1]['source'])

    def test_can_deactivate_without_a_manual_trigger_does_not_admit_demolition(self):
        data = fixture()
        authored = data[0]['maps'][0]['objectives']
        del authored['instances'][2:]
        row = authored['definitions'][0]['fields']
        row['CanRepair']['value'] = False
        row['TimeToRepairMs']['value'] = 0
        data[1]['source']['authored_catalog_sha256'] = digest(data[0])
        data[2]['metadata_sha256'] = digest(data[0])
        data[3]['maps'][0]['triggers'].clear()
        data[4]['metadata_sha256'] = digest(data[0])

        catalog, structural = self.project(*data)
        self.assertNotIn('demolition', catalog['objects'][0])
        self.assertFalse(catalog['objects'][0]['repair']['can_repair'])
        self.assertEqual(structural['rows'][0]['trigger_positions'], [])

    def test_rejects_missing_trigger_or_disagreeing_sim_order(self):
        data = fixture()
        data[3]['maps'][0]['triggers'].pop()
        with self.assertRaisesRegex(ValueError, 'trigger inventory'):
            self.project(*data)

        data = fixture()
        data[4]['maps'][0]['objects'][0]['sim_slot'] = 1
        with self.assertRaisesRegex(ValueError, 'Sim.*catalog'):
            self.project(*data)

    def test_rejects_changed_receipt_even_when_the_join_could_still_match(self):
        metadata, objective, bindings, trigger, schedule = fixture()
        with patch.multiple(API, METADATA=digest(metadata),
                            TRIGGER_PROOF_SHA256=digest(trigger), SIM_SCHEDULE_SHA256=digest(schedule)):
            trigger['maps'][0]['triggers'][0]['authored_volume']['trigger_position'][2] += 1
            with self.assertRaisesRegex(ValueError, 'trigger proof'):
                API.project_interactions(
                    encode(metadata), encode(objective), bindings, encode(trigger), encode(schedule))

    def test_unqualified_trigger_owner_or_level_is_rejected(self):
        data = fixture()
        data[3]['maps'][0]['triggers'][0]['objective_token'] = '0x00000012'
        with self.assertRaisesRegex(ValueError, 'trigger.*binding'):
            self.project(*data)

        data = fixture()
        data[4]['maps'][0]['source'] = dict(archive_sha256='e' * 64, clv_sha256='f' * 64)
        with self.assertRaisesRegex(ValueError, 'level'):
            self.project(*data)


if __name__ == '__main__':
    unittest.main()
