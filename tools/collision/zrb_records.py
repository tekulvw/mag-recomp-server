"""Offline evidence reader: native version6 resource headers, physics resource12.
Never guesses record locations from Havok signatures; not a runtime asset loader.
"""
import collections, hashlib, json, struct
from pathlib import Path
MAGIC=bytes.fromhex('57e0e05710c0c010')
def parse(data, *, retail13_compat=False):
 def u(p):
  if p<0 or p+4>len(data):raise ValueError('u32 out of bounds')
  return struct.unpack_from('>I',data,p)[0]
 if len(data)<50:raise ValueError('short block')
 h=struct.unpack_from('>12I',data)
 if h[0]!=6 or data[49]!=0:raise ValueError('only version6 without TOC proven')
 if h[1]!=50 or h[11]>100000:raise ValueError('unexpected header location/count')
 if h[1]+h[2]>h[3] or h[3]>len(data):raise ValueError('header outside header section')
 for start,size in ((h[3],h[4]),(h[5],h[6]),(h[7],h[8]),(h[9],h[10])):
  if start>len(data) or size>len(data)-start:raise ValueError('section outside file')
 classes=[(u(50+i*8),u(54+i*8)) for i in range(h[11])]
 base=50+8*h[11];p=base;records=[];spans=[]
 if sum(c for _,c in classes)>100000:raise ValueError('excess resource count')
 for ordinal in range(sum(c for _,c in classes)):
  words=[u(p+4*i) for i in range(11)]
  version,magic,nxt,rv,main,temp,gpu,io,cls,name,priority=words
  end=base+nxt
  if version!=6 or magic!=0x600df00d or not p+44<=end<=h[3]:raise ValueError(f'bad record {ordinal} at {p:x}')
  record=dict(ordinal=ordinal,header_offset=p,header_end=end,resource_version=rv,class_token=f'{cls:08x}',name_token=f'{name:08x}',main_relative_offset=main,main_file_offset=h[3]+main,priority=priority)
  if cls==0x2a8bdf5a and (rv==12 or (rv==13 and retail13_compat)):
   typ,n=u(p+44),u(p+48)
   array_base=p+52+(4 if rv==13 else 0)
   if rv==13:
    record['retail13_opaque_word_hex']=data[p+52:p+56].hex()
    record['retail13_instance_count']=u(p+52)
   record['body_basis']='prototype native ReadHeader' if rv==12 else 'retail native ReadHeader: type/count/instance-count, sizes, material-index counts, parent tokens, collision tags'
   if n>100000 or array_base+16*n>end:raise ValueError('bad physics count')
   sizes=[u(array_base+i*4) for i in range(n)]
   counts=[u(array_base+4*n+i*4) for i in range(n)]
   parents=[u(array_base+8*n+i*4) for i in range(n)]
   tags=[struct.unpack_from('>i',data,array_base+12*n+i*4)[0] for i in range(n)]
   pos=h[3]+main;mat=pos+sum(sizes);packs=[]
   for i,size in enumerate(sizes):
    if size<64 or pos+size>h[3]+h[4] or data[pos:pos+8]!=MAGIC:raise ValueError(f'bad declared packfile {ordinal}/{i} at {pos:x}')
    if mat+counts[i]>h[3]+h[4] or len(data[mat:mat+counts[i]])!=counts[i]:raise ValueError('material range outside main section')
    packs.append(dict(index=i,offset=pos,bytes=size,sha256=hashlib.sha256(data[pos:pos+size]).hexdigest(),parent_token=f'{parents[i]:08x}',collision_tag=tags[i],material_indices_offset=mat,material_indices=list(data[mat:mat+counts[i]])))
    spans.append((pos,pos+size));pos+=size;mat+=counts[i]
   record.update(physics_resource_type=typ,packfiles=packs,unparsed_header_tail_hex=data[array_base+16*n:end].hex(),payload_end=mat)
  else:record['unparsed_resource_header_hex']=data[p+44:end].hex()
  records.append(record);p=end
 actual=collections.Counter(r['class_token'] for r in records)
 if actual!=collections.Counter({f'{k:08x}':v for k,v in classes}):raise ValueError('class totals disagree')
 spans.sort()
 if any(a[1]>b[0] for a,b in zip(spans,spans[1:])):raise ValueError('overlapping declared packfiles')
 scans=[];pos=0
 while (pos:=data.find(MAGIC,pos))>=0:scans.append(pos);pos+=8
 declared=[a for a,b in spans]
 return dict(schema='native-zrb6-physics12-evidence-v1',sha256=hashlib.sha256(data).hexdigest(),bytes=len(data),block_header=dict(zip(['version','header_offset','header_size','main_offset','main_size','temp_offset','temp_size','gpu_offset','gpu_size','io_offset','io_size','class_count'],h)),localized=data[48],has_toc=data[49],first_resource_base=base,class_counts=[dict(class_token=f'{k:08x}',count=v) for k,v in classes],counts=dict(resources=len(records),declared_packfiles=len(spans),signature_occurrences=len(scans),unparsed_resources=sum('packfiles' not in r for r in records)),signature_consistency=dict(undeclared_offsets=sorted(set(scans)-set(declared)),declared_missing_from_scan=sorted(set(declared)-set(scans))),records=records)
if __name__=='__main__':
 import argparse
 ap=argparse.ArgumentParser();ap.add_argument('source',type=Path);ap.add_argument('output',type=Path);a=ap.parse_args()
 result=parse(a.source.read_bytes());a.output.write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result['counts']))
