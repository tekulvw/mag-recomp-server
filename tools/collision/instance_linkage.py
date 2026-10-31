"""Evidence-only authored CLV instance/property/token joins for all installed maps.
No terrain snapping, hierarchy resolution, runtime collision or motion claims.
"""
from pathlib import Path
import collections,math,gzip,hashlib,importlib.util,json,struct,sys,zipfile
ROOT=Path(__file__).resolve().parents[2]
HERE=Path(__file__).resolve().parent
def load(n,p):
 s=importlib.util.spec_from_file_location(n,p);m=importlib.util.module_from_spec(s);s.loader.exec_module(m);return m
sp=load('spawn',ROOT/'tools/extraction/extract-retail-spawns.py')
zr=load('zrb',HERE/'zrb_records.py')
class Tokens(dict):
 def get(self,k,d=None):return super().get(k,f'0x{k:08x}')
class Clv(sp.Clv):
 def fields(self,p,limit=None):
  result=[];limit=limit or len(self.b)
  while p<limit and self.b[p]==1:
   name,p=self.string(p+1);typ,p=self.string(p);size=self.u32(p)
   if not 4<=size<=100000 or p+size>limit:raise ValueError('invalid property boundary')
   start=p+4;raw=self.b[start:p+size];p+=size;value=raw.hex();token=None
   if typ=='float' and len(raw)==4:
    value=struct.unpack('>f',raw)[0]
    if not math.isfinite(value):raise ValueError('nonfinite authored value')
   elif typ in ('uint32','int32') and len(raw)==4:value=int.from_bytes(raw,'big',signed=typ=='int32')
   elif typ=='bool' and len(raw)==1:
    if raw[0]>1:raise ValueError('invalid boolean')
    value=bool(raw[0])
   elif typ in ('util::Token','util::LocalizedString') and len(raw)==4:token=int.from_bytes(raw,'big');value=self.tokens.get(token)
   elif typ=='enum' and len(raw)==13 and raw[:12]==bytes.fromhex('deadbeef0000000100000000'):value=int.from_bytes(raw[12:],'big',signed=True)
   result.append(dict(name=name,type=typ,value=value,raw_hex=raw.hex(),offset=hex(start),bytes=len(raw),**({'token':f'0x{token:08x}'} if token is not None else {})))
  if p>=limit or self.b[p]!=0:raise ValueError('missing properties terminator')
  return result,p

def sha(b):return hashlib.sha256(b).hexdigest()
def inspect(archive,tokens,source,build,name,output):
 ps=sp.Psarc(archive);ci=next(i for i,n in enumerate(ps.names,1) if n.endswith('.clv'));b=ps.extract(ci)
 pi=next(i for i,n in enumerate(ps.names,1) if n.endswith('/physics.zrb'));physics=zr.parse(ps.extract(pi), retail13_compat=True);resources={int(x['name_token'],16):x for x in physics['records']}
 tokens=Tokens(tokens);classes={};pos=0;marker=bytes.fromhex('deadbeef00000003')
 while (pos:=b.find(marker,pos))>=0:
  if pos>=16 and b[pos-16:pos-12]==struct.pack('>I',7):
   n=struct.unpack_from('>I',b,pos+8)[0]
   if 0<n<128 and pos+12+n<=len(b):
    cls=b[pos+12:pos+12+n].rstrip(b'\0').decode('ascii');tok=struct.unpack_from('>I',b,pos-12)[0]
    if tok in classes and classes[tok]!=cls:raise ValueError('ambiguous class')
    classes[tok]=cls;tokens[tok]=cls
  pos+=4
 clv=Clv(b,tokens);tables={cls:clv.tables(cls) for cls in classes.values()};inst=tables.pop('InstanceProperties',[])
 defs={};matches=[]
 for cls,ts in tables.items():
  for t in ts:
   # Only direct serialized token fields; no arbitrary byte-string token scans.
   refs=[dict(field=f['name'],token=f['token'],resource=resources[int(f['token'],16)]['ordinal']) for f in t['fields'] if 'token' in f and int(f['token'],16) in resources]
   summary=dict(class_name=cls,name_token=f"{t['name_token']:08x}",offset=t['offset'],sha256=t['source_sha256'],direct_physics_references=refs)
   # Preserve explicit authored motion/filter switches without inferring behavior.
   summary['authored_physics_fields']=[f for f in t['fields'] if any(k in f['name'].lower() for k in ('physic','collis','static','dynamic','rigid','mass','model','hsx'))]
   if t['name_token'] in defs:raise ValueError('duplicate definition token')
   defs[t['name_token']]=summary
 rows=[]
 for t in inst:
  fs={f['name']:f['value'] for f in t['fields']};d=defs.get(t['preceding_token']);raw={f['name']:f for f in t['fields']}
  row=dict(instance_offset=t['offset'],instance_sha256=t['source_sha256'],definition_token=f"{t['preceding_token']:08x}",name=fs.get('Name'),authored_object_id=fs.get('ObjectID'),translation=[fs.get('Translate'+a) for a in 'XYZ'],rotation_degrees=[fs.get('Rotate'+a) for a in 'XYZ'],scale=[fs.get('Scale'+a) for a in 'XYZ'],snap_to_terrain=fs.get('SnapToTerrain_M'),is_frozen_authored=fs.get('IsFrozen_M'),is_visible_authored=fs.get('IsVisible_M'),definition_class=d['class_name'] if d else None,direct_physics_references=d['direct_physics_references'] if d else [],world_transform_verified=False,static_dynamic_classification=None)
  node_version=clv.u32(int(t['offset'],16)-12)
  if node_version not in (4,5):raise ValueError('unrecognized prefab node version')
  end=int(t['end'],16);light=clv.u32(end);collision=clv.u32(end+4)
  row.update(node_version=node_version,lightmap_token=f'{light:08x}',collision_mesh_token=f'{collision:08x}',collision_resource_ordinal=resources[collision]['ordinal'] if collision in resources else None,collision_link_basis='PrefabInstanceNode::Read version>2 trailer into InstanceProperties.CollisionMesh; prototype native, retail compatible serialized layout')
  rows.append(row)
 output_document=dict(schema='authored-instance-direct-token-evidence-v1',build=build,map=name,source=dict(**source,archive_sha256=sha(archive),clv_member=ps.names[ci-1],clv_sha256=sha(b),physics_member=ps.names[pi-1],physics_sha256=physics['sha256']),limits=['Direct serialized token equality only; references in nested objects/hierarchy are unresolved.','Authored translate/rotate/scale and SnapToTerrain are not final world collision placement.','Prototype native resource12 body is proven; retail resource13 body remains opaque.','Class names and switches are retained, not interpreted as final runtime static/dynamic ownership.'],counts=dict(instances=len(rows),definitions=len(defs),physics_resources=len(resources),instances_with_definition=sum(x['definition_class'] is not None for x in rows),instances_with_direct_physics_ref=sum(bool(x['direct_physics_references']) for x in rows),instances_snap_true=sum(x['snap_to_terrain'] is True for x in rows),instances_with_collision_mesh=sum(x['collision_mesh_token']!='00000000' for x in rows),instances_with_local_physics_resource=sum(x['collision_resource_ordinal'] is not None for x in rows)),property_class_counts={k:len(v) for k,v in tables.items()},definitions=list(defs.values()),instances=rows,resource_headers=[{k:v for k,v in x.items() if k in ('ordinal','name_token','class_token','resource_version','main_file_offset','physics_resource_type')} for x in physics['records']])
 output_bytes=(json.dumps(output_document,separators=(',',':'))+'\n').encode()
 if output is not None:output.write_bytes(gzip.compress(output_bytes,mtime=0))
 return output_document

def main():
 import argparse
 p=argparse.ArgumentParser(description=__doc__)
 p.add_argument('--archive',type=Path,required=True)
 p.add_argument('--output',type=Path,required=True)
 p.add_argument('--map',required=True)
 a=p.parse_args()
 if a.output.resolve()==a.archive.resolve():p.error('output must not replace input')
 inspect(a.archive.read_bytes(),{},dict(archive=a.archive.name),'retail-bcus98110-01.00',a.map,a.output)
if __name__=='__main__':main()
