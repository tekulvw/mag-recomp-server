#!/usr/bin/env python3
"""Extract authored retail v1.0 spawn locations, without mutating game assets.

Property parser adapted from the original implementation's prototype extract-spawn-catalog.py
(commit 58e39f7). Retail records supply their own class names and token IDs;
no prototype map coordinates or token-map names are used as retail data.

Adapted from the project's retail spawn extractor; history references are
recorded in tools/extraction/PROVENANCE.md. Inputs are supplied explicitly.
Outputs are derived metadata, never bundled native resources.
"""
from pathlib import Path
import argparse, collections, csv, hashlib, json, math, re, struct
import sys
ROOT=Path(__file__).resolve().parents[2]
sys.path.insert(0,str(ROOT/'tools/collision'))
from archive import Psarc

SPAWN_TYPES={-1:'SpawnPointTypeUndefined',0:'SpawnPointBasic',1:'SpawnPointHALO',2:'SpawnPointNaval',3:'SpawnPointAPC',4:'SpawnPointHelo',5:'SpawnPointLALODropZone',6:'SpawnPointBeachhead',7:'SpawnPointMotorPool',8:'SpawnPointFieldHospital',9:'SpawnPointEscortVehicle'}
CATEGORIES={0:'SpawnPointCategoryInsertion',1:'SpawnPointCategoryAlternate',2:'SpawnPointCategoryForward'}
USERS={0:'SpawnPointUserPlayer',1:'SpawnPointUserVehicle'}
FACTIONS={'FactionTypeGreen':'Valor','FactionTypeRed':'SVER','FactionTypeBlack':'Raven'}
MODES={'GameType64':'Sabotage','GameType128':'Acquisition','GameType256':'Domination','GameTypeFactionExercise':'Faction Exercise'}

def sha(b):return hashlib.sha256(b).hexdigest()
def dump(path,value):path.write_text(json.dumps(value,indent=2,allow_nan=False)+'\n')

class Clv:
 def __init__(self,b,tokens):self.b=b;self.tokens=tokens;self.byname={v:k for k,v in tokens.items()};self.audit={}
 def u32(self,p):return struct.unpack_from('>I',self.b,p)[0]
 def string(self,p):
  n=self.u32(p);assert n<=4096 and p+4+n<=len(self.b)
  return self.b[p+4:p+4+n].rstrip(b'\0').decode(),p+4+n
 def fields(self,p,limit=None):
  result=[];limit=limit or len(self.b)
  while p<limit and self.b[p]==1:
   name,p=self.string(p+1);typ,p=self.string(p);size=self.u32(p);assert 4<=size<=100000 and p+size<=limit
   start=p+4;raw=self.b[start:p+size];p+=size
   value=raw.hex();token=None
   if typ=='float' and len(raw)==4:value=struct.unpack('>f',raw)[0];assert math.isfinite(value)
   elif typ in ('uint32','int32') and len(raw)==4:value=int.from_bytes(raw,'big',signed=typ=='int32')
   elif typ=='bool' and len(raw)==1:assert raw[0]<=1;value=bool(raw[0])
   elif typ in ('util::Token','util::LocalizedString') and len(raw)==4:
    token=int.from_bytes(raw,'big');value=self.tokens.get(token);assert value is not None
   elif typ=='enum':
    assert len(raw)==13 and raw[:12]==bytes.fromhex('deadbeef0000000100000000');value=int.from_bytes(raw[12:],'big',signed=True)
   result.append({'name':name,'type':typ,'value':value,'raw_hex':raw.hex(),'offset':hex(start),'bytes':len(raw),**({'token':f'0x{token:08x}'} if token is not None else {})})
  assert p<limit and self.b[p]==0
  return result,p
 def tables(self,classname):
  if classname not in self.byname:return []
  pat=struct.pack('>I',self.byname[classname]);p=0;result=[];hits=0;rejected=0
  while (p:=self.b.find(pat,p))>=0:
   hits+=1
   # Only real version-7 property records, not token references in other fields.
   if p>=4 and self.u32(p-4)==7 and self.b[p+12:p+20]==bytes.fromhex('deadbeef00000003'):
    name,start=self.string(p+20);assert name==classname
    fields,end=self.fields(start);assert self.u32(p+4)==end+1,(classname,hex(p),hex(self.u32(p+4)),hex(end+1))
    result.append({'offset':hex(p),'end':hex(end+1),'name_token':self.u32(p+8),'preceding_token':self.u32(p-8),'fields':fields,'source_sha256':sha(self.b[p-4:end+1])})
   else:rejected+=1
   p+=1
  self.audit[classname]={'token_occurrences':hits,'records':len(result),'non_record_token_references':rejected}
  return result
 def squad_references(self,table):
  values={f['name']:f['value'] for f in table['fields']};refs=[]
  for f in table['fields']:
   if not f['name'].startswith('SquadProps_'):continue
   assert f['type']=='object';start=int(f['offset'],16);raw=bytes.fromhex(f['raw_hex'])
   assert raw[:9]==bytes.fromhex('01deadbeef00000002')
   assert int.from_bytes(raw[9:13],'big')==len(raw)-9
   assert raw[13:21]==bytes.fromhex('deadbeef00000003')
   cls,p=self.string(start+21);assert cls=='SquadProperties'
   nested,end=self.fields(p,start+len(raw));assert end==start+len(raw)-1
   field=next(x for x in nested if x['name']=='SpawnPoints');b=bytes.fromhex(field['raw_hex']);version,count=struct.unpack_from('>II',b);assert version==1 and len(b)==8+4*count
   for index,(token,) in enumerate(struct.iter_unpack('>I',b[8:])):
    refs.append({'platoon_id':values['ID'],'team':values['Team'],'squad':f['name'].removeprefix('SquadProps_'),'slot':index,'point_token':f'0x{token:08x}','point_name':self.tokens[token],'property_offset':table['offset'],'vector_offset':field['offset'],'evidence':'structured PlatoonProperties.SquadProps_*.SpawnPoints'})
  return refs

def catalog_settings(text):
 result=[]
 for m in re.finditer(r'GameLevelData\s*=\s*\{([^{}]+)\}',text):
  fields=dict(re.findall(r'(m_\w+)\s*=\s*"([^"]*)"',m[1]));mode=list(re.finditer(r'm_gameType\s*=\s*"([^"]+)"',text[:m.start()]))[-1][1]
  result.append({'map':fields['m_levelName'],'display_name':fields['m_displayName'],'owner_faction_enum':fields['m_owningFaction'],'owner_faction':FACTIONS.get(fields['m_owningFaction']),'mode_enum':mode,'mode':MODES.get(mode),'source':'settings/gameproperties.zpr','source_line':text.count('\n',0,m.start())+1})
 return result

class RetailTokens(dict):
 """Unknown token spellings stay opaque, never borrowed from another build."""
 def __missing__(self, key): return f'0x{key:08x}'
 def get(self, key, default=None): return self[key]

def retail_clv(data):
 tokens=RetailTokens({0:''})
 for cls in ('SpawnPointProperties','InstanceProperties','PlatoonProperties'):
  marker=struct.pack('>I',len(cls)+1)+cls.encode()+b'\0'; pos=0; found=set()
  while (pos:=data.find(marker,pos))>=0:
   if pos>=24 and data[pos-8:pos]==bytes.fromhex('deadbeef00000003') and data[pos-24:pos-20]==struct.pack('>I',7):
    found.add(struct.unpack_from('>I',data,pos-20)[0])
   pos+=1
  if len(found)>1:raise ValueError(f'Ambiguous {cls} token')
  if found:tokens[found.pop()]=cls
 return Clv(data,tokens)

def evidence(table):
 return {k:table[k] for k in ('offset','end','source_sha256')}

def extract_map(path,info):
 archive=path.read_bytes();arc=Psarc(archive);names=arc.names;extract=arc.extract
 members=[(i,n) for i,n in enumerate(names,1) if n.endswith('.clv')]
 if len(members)!=1:raise ValueError(f'Expected one CLV: {path}')
 index,member=members[0];data=extract(index);clv=retail_clv(data)
 definitions=clv.tables('SpawnPointProperties');props={p['name_token']:p for p in definitions}
 assert len(props)==len(definitions)
 instances=clv.tables('InstanceProperties');spawns=[i for i in instances if i['preceding_token'] in props]
 platoons=clv.tables('PlatoonProperties');refs=[r for p in platoons for r in clv.squad_references(p)]
 points=[]
 for ordinal,inst in enumerate(spawns):
  v={f['name']:f['value'] for f in inst['fields']};definition=props[inst['preceding_token']];p={f['name']:f['value'] for f in definition['fields']}
  links=[r for r in refs if r['point_name']==v['Name']]
  if any(r['team']!=p['Team'] for r in links):raise ValueError('Squad/point team mismatch')
  faction=info.get('owner_faction') if info['mode_enum']!='TrainingMission' and (p['Team']==1 or info['mode_enum']=='GameTypeFactionExercise') else None
  points.append({'catalog_id':f"{path.stem}:{ordinal:03}",'name_token':v['Name'],'prefab_token':f"0x{inst['preceding_token']:08x}",'position_xyz':[v['Translate'+a] for a in 'XYZ'],'rotation_xyz':[v['Rotate'+a] for a in 'XYZ'],'scale_xyz':[v['Scale'+a] for a in 'XYZ'],'snap_to_terrain':v['SnapToTerrain_M'],'authored_object_id':v['ObjectID'],'active':p['Active'],'radius':p['Radius'],'team':p['Team'],'match_side':{0:'attacker',1:'defender'}.get(p['Team'],'unknown'),'faction':faction,'spawn_type':p['SPType'],'spawn_type_name':SPAWN_TYPES.get(p['SPType']),'user_class':p['UserClass'],'user_class_name':USERS.get(p['UserClass']),'category':p['Category'],'category_name':CATEGORIES.get(p['Category']),'vehicle_type_token':p['VehicleType'],'vehicle_mountable':p['VehicleIsMountable'],'aircraft_path_token':p['AircraftPath'],'drop_point_index':p['DropPointIndex'],'authored_platoon_token':p['Platoon'],'squad_links':links,'instance_evidence':evidence(inst),'definition_evidence':evidence(definition)})
 nameset={p['name_token'] for p in points};unresolved=[r for r in refs if r['point_name'] not in nameset]
 assert len(nameset)==len(points),'Duplicate spawn names'
 counts={'spawn_definitions':len(props),'spawn_instances':len(points),'all_instance_records':len(instances),'platoon_property_records':len(platoons),'squad_reference_entries':len(refs),'resolved_squad_reference_entries':len(refs)-len(unresolved),'unique_squad_groups':len({(r['team'],r['platoon_id'],r['squad']) for r in refs}),'spawn_instances_without_structured_squad_link':sum(not p['squad_links'] for p in points),'user_classes':dict(collections.Counter(p['user_class_name'] for p in points)),'active_instances':sum(p['active'] for p in points),'inactive_instances':sum(not p['active'] for p in points),'teams':dict(collections.Counter(p['match_side'] for p in points)),'types':dict(collections.Counter(p['spawn_type_name'] for p in points))}
 return {'map':info,'source':{'archive':str(path),'archive_bytes':len(archive),'archive_sha256':sha(archive),'clv_member':member,'clv_bytes':len(data),'clv_sha256':sha(data),'tokenmap_members':[n for n in names if 'tokenmap' in n.lower()]},'counts':counts,'parser_audit':clv.audit,'unresolved_squad_references':unresolved,'points':points}

def main():
 parser=argparse.ArgumentParser(description=__doc__)
 parser.add_argument('--disc',type=Path,required=True,help="original retail disc dump directory containing PS3_GAME")
 parser.add_argument('--game-archive',type=Path,required=True,help='decrypted retail game.psarc (sha256 b49c9435...)')
 parser.add_argument('--output',type=Path,required=True)
 args=parser.parse_args();args.output.mkdir(parents=True,exist_ok=True)
 game=args.game_archive.read_bytes()
 if sha(game)!='b49c9435be729832029ad7019874d478080dc9764ebde1809b2851d353bc769f':raise ValueError('Wrong retail game settings archive')
 arc=Psarc(game);names=arc.names;extract=arc.extract;settings_name='/run/mag/ps3/settings/gameproperties.zpr';settings=extract(names.index(settings_name)+1)
 configured=catalog_settings(settings.decode());byname={m['map']:m for m in configured};maps=[]
 training_name='/run/mag/ps3/scripts/training_missions.zpr';training=extract(names.index(training_name)+1)
 for faction,level in re.findall(r'TrainingMission(FactionType\w+)\s*=\s*"([^"]+)"',training.decode()):
  byname[level]={'map':level,'display_name':'Training','owner_faction':FACTIONS[faction],'owner_faction_enum':faction,'mode':'Training','mode_enum':'TrainingMission','source':training_name,'faction_basis':'Training mission selected for this faction; per-team character faction unverified'}
 localized_name='/run/mag/ps3/scripts/localized_mapnames.zpr';localized=extract(names.index(localized_name)+1)
 for level,tag in re.findall(r'm_mapName\s*=\s*"([^"]+)",\s*m_localizedTag\s*=\s*"([^"]+)"',localized.decode()):
  if level in byname:byname[level]['localized_name_tag']=tag
 for path in sorted((args.disc/'PS3_GAME/USRDIR/run/mag/ps3/levels').glob('*.psarc')):
  info=byname.get(path.stem)
  if info is None:
   info={'map':path.stem,'display_name':None,'owner_faction':None,'owner_faction_enum':None,'mode':'Training (filename inference)','mode_enum':None,'source':'Physical archive; absent from multiplayer GameLevelData configuration','filename_faction_hint':next((f for s,f in [('raven','Raven'),('sver','SVER'),('valor','Valor')] if s in path.stem),None)}
  result=extract_map(path,info);maps.append(result);dump(args.output/(path.stem+'.json'),result);print(path.stem,json.dumps(result['counts']),flush=True)
 groups=[]
 for m in maps:
  grouped=collections.defaultdict(list)
  for p in m['points']:
   for ref in p['squad_links']:
    grouped[ref['team'],ref['platoon_id'],ref['squad']].append({'slot':ref['slot'],'category':p['category'],'category_name':p['category_name'],'catalog_id':p['catalog_id'],'name_token':p['name_token'],'position_xyz':p['position_xyz'],'rotation_xyz':p['rotation_xyz'],'active':p['active'],'spawn_type':p['spawn_type']})
  for (team,platoon,squad),slots in sorted(grouped.items()):
   groups.append({'map':m['map']['map'],'team':team,'platoon_id':platoon,'squad':squad,'points':sorted(slots,key=lambda x:x['slot'])})
 dump(args.output/'squad-spawns.json',{'platoon_ids_are_zero_based':True,'groups':groups})
 with (args.output/'spawn-locations.csv').open('w',newline='') as f:
  columns=['map','display_name','mode','owner_faction','catalog_id','team','faction','name_token','x','y','z','rotate_x','rotate_y','rotate_z','active','radius','spawn_type','user_class','category','squad_links']
  writer=csv.DictWriter(f,fieldnames=columns);writer.writeheader()
  for m in maps:
   for p in m['points']:
    row={k:m['map'][k] for k in ('map','display_name','mode','owner_faction')};row.update({k:p[k] for k in ('catalog_id','team','faction','name_token','active','radius','spawn_type','user_class','category')});row.update(zip(('x','y','z'),p['position_xyz']));row.update(zip(('rotate_x','rotate_y','rotate_z'),p['rotation_xyz']));row['squad_links']=';'.join(f"team{r['team']}/platoon{r['platoon_id']}/{r['squad']}/slot{r['slot']}" for r in p['squad_links']);writer.writerow(row)
 summary={'schema_version':1,'build':'BCUS98110 01.00','scope':'Every SpawnPointProperties-linked instance in all physical retail map archives','source_disc':str(args.disc),'game_archive_sha256':sha(game),'settings_member':settings_name,'settings_sha256':sha(settings),'training_member':training_name,'training_sha256':sha(training),'localized_names_member':localized_name,'localized_names_sha256':sha(localized),'map_count':len(maps),'configured_but_not_bundled':[m for m in configured if m['map'] not in {r['map']['map'] for r in maps}],'total_spawn_instances':sum(len(m['points']) for m in maps),'total_squad_groups':len(groups),'total_squad_references':sum(m['counts']['squad_reference_entries'] for m in maps),'unresolved_squad_references':sum(len(m['unresolved_squad_references']) for m in maps),'maps':[{'metadata':m['map'],'source':m['source'],'counts':m['counts'],'file':m['map']['map']+'.json'} for m in maps],'rotation_units':'Raw RotateX/Y/Z floats; degrees is a prototype-derived interpretation, not retail runtime validation','team_labels_source':'Retail gameproperties.zpr explicitly comments Team 0 is attacker and Team 1 defender; training interpretation not independently confirmed','enum_labels_source':'Prototype DWARF enum declarations; retail numeric values retained and not independently runtime-validated','limitations':['Static authored locations, not runtime activation, collision clearance or network pool IDs.','Retail token spellings are unavailable; identifiers remain opaque hex values.','Attacking faction is match-selected; defending faction follows retail map owner.','Faction Exercise uses its configured owning faction for both teams.','Training map selection is faction-specific in training_missions.zpr; per-team character faction remains unknown.','SPType/Category/UserClass descriptive labels follow prototype DWARF; raw retail numbers remain authoritative.'],'output_files':{p.name:sha(p.read_bytes()) for p in sorted(args.output.glob('*')) if p.is_file() and p.name!='catalog.json'}}
 dump(args.output/'catalog.json',summary)
if __name__=='__main__':main()
