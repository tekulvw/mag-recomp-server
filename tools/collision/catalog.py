#!/usr/bin/env python3
"""Inventory all available prototype/retail maps and convert offline geometry.
Original archives are read-only. Output belongs outside tracked source paths.
Names are discovery; packfile ownership is not inferred from signature proximity.
"""
from pathlib import Path
import argparse,collections,gzip,hashlib,json,re,sys,zipfile
from archive import Psarc
from packfile import MAGIC,parse_packfile,CollisionFormatError
from zrb_records import parse as parse_zrb

def sha(data):return hashlib.sha256(data).hexdigest()
def file_sha(path):
    h=hashlib.sha256()
    with Path(path).open('rb') as stream:
        while chunk:=stream.read(1024*1024):h.update(chunk)
    return h.hexdigest()
def encoded(value):return (json.dumps(value,sort_keys=True,separators=(',',':'),allow_nan=False)+'\n').encode()
def dump(path,value):path.parent.mkdir(parents=True,exist_ok=True);path.write_bytes(encoded(value))

class Catalog:
    def __init__(self,out,retain_raw,include_instance_evidence=True):
        self.include_instance_evidence=include_instance_evidence
        self.out=out;self.retain_raw=retain_raw;self.known={};self.resources={};self.geometry={}
    def member(self,data,member):
        member_sha=sha(data);report={'member':member,'bytes':len(data),'sha256':member_sha,'packfiles':[],'invalid_candidates':[],'discovery':'embedded signature plus bounded structural packfile validation; structural resource ownership attached where native header reader succeeds'}
        if self.retain_raw and member.endswith('/physics.zrb'):
            raw=self.out/'raw-offline'/'members'/f'{member_sha}.zrb';raw.parent.mkdir(parents=True,exist_ok=True);raw.write_bytes(data)
            report['raw_offline_source_member']=str(raw)
        if member_sha in self.resources:
            original=self.resources[member_sha];return {**original,'member':member}
        for m in re.finditer(re.escape(MAGIC),data):
            base=m.start()
            try:geometry=parse_packfile(data,base)
            except CollisionFormatError as e:
                report['invalid_candidates'].append({'offset':base,'reason':str(e)});continue
            ident=geometry['packfile_sha256']
            if ident not in self.known:
                neutral=encoded(geometry);compressed=gzip.compress(neutral,compresslevel=1,mtime=0)
                relative=f'geometry/{ident}.json.gz';dump_path=self.out/relative;dump_path.parent.mkdir(parents=True,exist_ok=True);dump_path.write_bytes(compressed)
                self.geometry[ident]=[{'shape_id':s['shape_id'],'subpart_index':s.get('subpart_index')} for s in geometry['meshes']]
                self.known[ident]={'file':relative,'sha256':sha(compressed),'decoded_sha256':sha(neutral),'bytes':len(compressed),'decoded_bytes':len(neutral)}
                if self.retain_raw:
                    raw=self.out/'raw-offline'/f'{ident}.hkx';raw.parent.mkdir(parents=True,exist_ok=True);raw.write_bytes(data[base:base+geometry['packfile_length']])
            entry={'offset':base,'length':geometry['packfile_length'],'source_sha256':ident,'neutral_output':self.known[ident],'class_counts':geometry['shape_class_coverage'],'meshes':len(geometry['meshes']),'vertices':sum(len(s['vertices_xyz']) for s in geometry['meshes']),'triangles':sum(len(s['triangles']) for s in geometry['meshes']),'unsupported':geometry['unsupported'],'unsupported_collision_objects':sum(1 for x in geometry['unsupported'] if x['source_object']['class'].startswith('hkp')),'world_assembly':'unresolved'}
            report['packfiles'].append(entry)
        if data[:4]==bytes.fromhex('00000006'):
            try:
                structural=parse_zrb(data,retail13_compat=True)
                report['structural_resource_header']={'reader':'native-zrb6-physics12-with-observed-retail13-compatibility-v1','counts':structural['counts'],'signature_consistency':structural['signature_consistency'],'block_header':structural['block_header'],'unknown_header_fields':'unconverted; no opaque native header bytes are geometry inputs'}
                by_offset={p['offset']:p for p in report['packfiles']}
                bindings=[]
                for record in structural['records']:
                    if 'packfiles' not in record:continue
                    bound={'resource_name_token':record['name_token'],'resource_class_token':record['class_token'],'resource_header_offset':record['header_offset'],'resource_version':record['resource_version'],'physics_type_raw':record['physics_resource_type'],'classification':'unresolved','body_basis':record.get('body_basis'),'retail13_opaque_word_hex':record.get('retail13_opaque_word_hex'),'packfiles':[]}
                    for native in record['packfiles']:
                        found=by_offset.get(native['offset'])
                        binding={'source_offset':native['offset'],'source_bytes':native['bytes'],'declared_packfile_sha256':native['sha256'],'parent_token':native['parent_token'],'collision_tag_raw':native['collision_tag'],'material_indices':native['material_indices'],'material_array_source_offset':native['material_indices_offset']}
                        if found is None:
                            binding['conversion_status']='unconverted_declared_packfile'
                        else:
                            if found['length']>native['bytes']:raise CollisionFormatError('converted packfile exceeds declared resource span')
                            binding['conversion_status']='converted_geometry';found['resource_binding']={**binding,'resource_name_token':record['name_token'],'physics_type_raw':record['physics_resource_type'],'resource_body_basis':record.get('body_basis'),'retail13_opaque_word_hex':record.get('retail13_opaque_word_hex')}
                            mapped=[]
                            for mesh in self.geometry[found['source_sha256']]:
                                subpart=mesh['subpart_index']
                                mapped.append({'shape_id':mesh['shape_id'],'subpart_index':subpart,'material_index':native['material_indices'][subpart] if subpart is not None and subpart<len(native['material_indices']) else None})
                            found['subpart_material_bindings']=mapped
                            if len(mapped)!=len(native['material_indices']) or any(x['material_index'] is None for x in mapped):
                                found['material_binding_status']='unresolved_subpart_material_count'
                            else:found['material_binding_status']='verified_external_material_indices_only'
                        bound['packfiles'].append(binding)
                    bindings.append(bound)
                report['physics_resource_bindings']=bindings
            except ValueError as e:
                report['structural_resource_header']={'status':'unsupported_or_malformed','reason':str(e)}
        self.resources[member_sha]=report
        return report
    def map(self,archive,build,name,source,pinned_sha):
        if sha(archive)!=pinned_sha:raise CollisionFormatError('map archive differs from pinned spawn evidence')
        p=Psarc(archive);members=[];excluded=[]
        for index,member in enumerate(p.names,1):
            if member.lower().endswith(('.zrb','.hsx','.hkx','.hkt')):
                members.append(self.member(p.extract(index),member))
            else:excluded.append({'member':member,'bytes':p.entries[index][1]})
        classes=collections.Counter();vertices=triangles=meshes=packfiles=unsupported=invalid=0
        for member in members:
            invalid+=len(member['invalid_candidates'])
            for item in member['packfiles']:
                packfiles+=1;meshes+=item['meshes'];vertices+=item['vertices'];triangles+=item['triangles'];unsupported+=len(item['unsupported']);classes.update(item['class_counts'])
        collision_unsupported=sum(p['unsupported_collision_objects'] for m in members for p in m['packfiles'])
        unowned=sum(1 for m in members for p in m['packfiles'] if p['class_counts'].get('hkpMoppBvTreeShape') and 'resource_binding' not in p)
        unresolved_materials=sum(1 for m in members for p in m['packfiles'] if p['class_counts'].get('hkpMoppBvTreeShape') and p.get('material_binding_status')!='verified_external_material_indices_only')
        counts={'unowned_collision_packfiles':unowned,'unresolved_material_binding_packfiles':unresolved_materials,'unsupported_collision_objects':collision_unsupported,'archive_members':len(p.names),'scanned_resource_members':len(members),'excluded_non_resource_members':len(excluded),'packfiles':packfiles,'converted_meshes':meshes,'vertices':vertices,'triangles':triangles,'unsupported_objects':unsupported,'invalid_packfile_candidates':invalid,'classes':dict(classes)}
        report={'schema_version':1,'build':build,'map':name,'source':{**source,'archive_bytes':len(archive),'archive_sha256':pinned_sha},'counts':counts,'resource_members':members,'excluded_members':excluded,'coverage':'resource_local_conversion_only','world_assembly':{'status':'unresolved','instance_count':0,'reason':'signature discovery is not resource ownership/placement proof'}}
        file=f'maps/{build}/{name}.json';dump(self.out/file,report)
        compact={'build':build,'map':name,'source':report['source'],'counts':counts,'manifest':{'file':file,'sha256':sha((self.out/file).read_bytes())},'coverage':report['coverage'],'world_assembly':report['world_assembly'],'authored_instance_candidates':report.get('authored_instance_candidates')}
        print(json.dumps({'map':name,'build':build,'counts':counts},sort_keys=True),flush=True)
        return compact
