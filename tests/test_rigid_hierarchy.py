import hashlib
from pathlib import Path
import struct
import sys
import unittest

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT/'tools/collision'))
from rigid_hierarchy import RigidHierarchyError, parse_rigid_resource, read_compiled_hierarchy
from parent_transforms import Hierarchy

def compact(x=0):return struct.pack('>8f',0,0,0,1,x,0,0,1)
def node(token,index,parent,child=65535,sibling=65535):return struct.pack('>6fII8H',0,0,0,1,1,1,token,0,0,65535,0,index,65535,sibling,child,parent)
def renderable(version=29):
    return (struct.pack('>II',version,123)+b'\0'+struct.pack('>4I',0,0,0,0)+bytes(32)+bytes(4)+bytes(16)+bytes(2)+bytes(2)+bytes(2)+bytes(8)+struct.pack('>5I4H4B',*([0]*13))+bytes(8))
def resource(*,version=63,renderable_version=29,attachments=0,nodes=None):
    rows=nodes or [node(10,2,65535,child=1),node(20,0,0,child=2),node(30,1,1)]
    base=struct.pack('>I4f6fHBII',0,0,0,0,1,0,0,0,1,1,1,0,0,1,attachments)
    atom=bytes(14)+renderable(renderable_version)+bytes(4)
    body=base+atom+struct.pack('>HHHII',len(rows),3,0,0,0)+compact(2)+compact(3)+compact(10)+b''.join(rows)+b'\0'
    common=struct.pack('>11I',6,0x600df00d,44+len(body),version,0,0,0,0,0xa4f4d23c,0x1234,0)
    raw=b'prefix!'+common+body
    return raw,{'header_offset':7,'header_end':len(raw),'resource_version':version,'class_token':'a4f4d23c','name_token':'00001234'}
def compiled(*,version=7,class_token=0xe3c2fba0,count=3):
    end=40+32*count;collision=end+72
    head=struct.pack('>10I',0x10001,collision,version,end,class_token,0x1234,0x555,0,0,count)
    return head+b''.join(compact(i)for i in range(count))+bytes(72)+struct.pack('>IB',0x10001,1),collision

class RigidHierarchyReaderTests(unittest.TestCase):
    def test_synthetic_resource_uses_typed_counts_and_distinct_transform_indices(self):
        raw,record=resource();parsed=parse_rigid_resource(raw,record)
        self.assertEqual([n.transform_index for n in parsed['nodes']],[2,0,1])
        self.assertEqual([n.parent_node_id for n in parsed['nodes']],[65535,0,1])
        self.assertEqual(parsed['node_array_offset'],7+99+149+14+3*32)
        self.assertEqual(parsed['node_evidence'][2]['source_offset'],parsed['node_array_offset']+96)
        graph=Hierarchy(parsed['nodes'],enumerate(parsed['reference_transforms']))
        self.assertEqual(graph.world_matrix_for_token(30)[3][0],15)
        self.assertEqual(parsed['unparsed_trailer']['bytes'],1)
    def test_renderable_layout_is_retail_descriptor_not_dev_stride(self):
        self.assertEqual(len(renderable()),131)
        raw,record=resource();parsed=parse_rigid_resource(raw,record)
        self.assertEqual(parsed['mesh_evidence'][0]['bytes'],149)
    def test_record_identity_cannot_substitute_name_or_class(self):
        raw,record=resource()
        for k,v in [('class_token','00000000'),('name_token','00000000')]:
            bad=dict(record);bad[k]=v
            with self.assertRaises(RigidHierarchyError):parse_rigid_resource(raw,bad)
    def test_unsupported_versions_and_attachments_fail(self):
        for kwargs in [dict(version=62),dict(renderable_version=28),dict(attachments=1)]:
            raw,record=resource(**kwargs)
            with self.assertRaises(RigidHierarchyError):parse_rigid_resource(raw,record)
    def test_truncated_or_oversized_header_never_searches_for_tokens(self):
        raw,record=resource()
        for cut in [10,100,len(raw)-30]:
            bad=dict(record);bad['header_end']=cut
            with self.assertRaises(RigidHierarchyError):parse_rigid_resource(raw,bad)
        bad=bytearray(raw);struct.pack_into('>I',bad,7+91,1000000)
        with self.assertRaises(RigidHierarchyError):parse_rigid_resource(bytes(bad),record)
    def test_graph_cycles_bad_transform_indices_and_duplicates_fail(self):
        for rows in [[node(10,2,2),node(20,0,0),node(30,1,1)], [node(10,3,65535),node(20,0,0),node(30,1,1)], [node(10,2,65535),node(10,0,0),node(30,1,1)]]:
            raw,record=resource(nodes=rows)
            with self.assertRaises(RigidHierarchyError):parse_rigid_resource(raw,record)
    def test_unreachable_and_inconsistent_native_lookup_graphs_fail(self):
        for rows in [[node(10,2,65535),node(20,0,0),node(30,1,1)], [node(10,2,65535,child=1),node(20,0,2,child=2),node(30,1,0)], [node(10,2,65535,child=1),node(20,0,0,sibling=1),node(30,1,0)]]:
            raw,record=resource(nodes=rows)
            with self.assertRaises(RigidHierarchyError):parse_rigid_resource(raw,record)
    def test_bad_future_parent_index_is_a_format_error(self):
        raw,record=resource(nodes=[node(10,2,65535),node(20,0,2),node(30,1,99)])
        with self.assertRaises(RigidHierarchyError):parse_rigid_resource(raw,record)
    def test_compiled_hierarchy_is_bounded_and_keeps_source_evidence(self):
        raw,collision=compiled();parsed=read_compiled_hierarchy(raw,0,collision)
        self.assertEqual(parsed['geometry_token'],0x1234)
        self.assertEqual(len(parsed['transforms']),3)
        self.assertEqual(parsed['transform_evidence'][1]['source_offset'],72)
        self.assertEqual(parsed['source_sha256'],hashlib.sha256(raw[:collision]).hexdigest())
    def test_collision_record_must_be_present_at_bound_target(self):
        raw,collision=compiled()
        for changed in [raw[:collision],raw[:collision+4]+b'\0',raw[:collision]+struct.pack('>IB',0,1)]:
            with self.assertRaises(RigidHierarchyError):read_compiled_hierarchy(changed,0,collision)
    def test_compiled_wrong_class_version_count_and_end_are_rejected(self):
        for kwargs in [dict(version=6),dict(class_token=0),dict(count=0)]:
            raw,collision=compiled(**kwargs)
            with self.assertRaises(RigidHierarchyError):read_compiled_hierarchy(raw,0,collision)
        raw,collision=compiled()
        with self.assertRaises(RigidHierarchyError):read_compiled_hierarchy(raw,0,collision-1)
        bad=bytearray(raw);struct.pack_into('>I',bad,12,41)
        with self.assertRaises(RigidHierarchyError):read_compiled_hierarchy(bytes(bad),0,collision)
    def test_nonfinite_quaternion_and_translation_fail_without_repair(self):
        for offset in [40,56]:
            raw,collision=compiled();bad=bytearray(raw);struct.pack_into('>f',bad,offset,float('nan'))
            with self.assertRaises(RigidHierarchyError):read_compiled_hierarchy(bytes(bad),0,collision)

if __name__=='__main__':unittest.main()
