"""Bounded, offline Havok6.5 BE32 stored mesh decoder.

Adapted from the earlier deferred extract-green-meshes.py: section/fixup
resolution and its verified storage layout. This produces neutral geometry,
never embeds Havok native trees/shape bytes in an accepted server input.
Unknown classes and unresolved placement/material semantics remain explicit.
"""
import hashlib, math, struct

MAGIC = bytes.fromhex('57e0e05710c0c010')
MAX_PACKFILE = 256 * 1024 * 1024
MAX_VERTICES = 2_000_000
MAX_INDICES = 8_000_000
STORAGE = 'hkpStorageExtendedMeshShapeMeshSubpartStorage'
STRUCTURAL = {'hkpMoppBvTreeShape', 'hkpMoppCode', 'hkpStorageExtendedMeshShape'}

class CollisionFormatError(ValueError):
    pass

class Reader:
    def __init__(self, data): self.data=data
    def span(self, start, size, end=None):
        end=len(self.data) if end is None else end
        if start < 0 or size < 0 or start > end or size > end-start or end > len(self.data):
            raise CollisionFormatError('native bounds outside containing section')
        return self.data[start:start+size]
    def unpack(self, fmt, at, end=None):
        return struct.unpack(fmt,self.span(at,struct.calcsize(fmt),end))
    def u32(self, at, end=None):return self.unpack('>I',at,end)[0]
    def text(self, at, end):
        self.span(at,1,end)
        null=self.data.find(b'\0',at,min(end,at+512))
        if null<0:raise CollisionFormatError('unterminated class name')
        try:return self.data[at:null].decode('ascii')
        except UnicodeError as e:raise CollisionFormatError('non-ASCII class name') from e


def parse_packfile(data, base=0):
    r=Reader(data)
    if r.span(base,8)!=MAGIC:raise CollisionFormatError('missing packfile magic')
    version=r.u32(base+12)
    if version!=6 or r.span(base+16,4)!=bytes([4,0,1,1]):
        raise CollisionFormatError('unsupported Havok version/pointer width/byte order')
    n=r.u32(base+20)
    if n!=3:raise CollisionFormatError('unsupported section count')
    content=r.text(base+40,base+56)
    if content!='Havok-6.5.0-r1':raise CollisionFormatError('unsupported Havok content version')
    sections=[]
    for i in range(n):
        at=base+64+48*i
        tag=r.text(at,at+20)
        absolute,local,glob,virtual,exports,imports,end=r.unpack('>7I',at+20)
        if not 0<=local<=glob<=virtual<=exports<=imports<=end<=MAX_PACKFILE:
            raise CollisionFormatError('invalid section offsets')
        if absolute+end>MAX_PACKFILE:raise CollisionFormatError('oversized packfile')
        r.span(base+absolute,end)
        sections.append({'tag':tag,'absolute':absolute,'local':local,'global':glob,'virtual':virtual,'exports':exports,'imports':imports,'end':end})
    length=max(s['absolute']+s['end'] for s in sections)
    raw=r.span(base,length)
    objects=[];meshes=[];unsupported=[];counts={};fixup_total=0; fixups={}
    for sn,s in enumerate(sections):
        start=base+s['absolute'];data_end=start+s['local'];local={}
        if (s['global']-s['local'])%8:raise CollisionFormatError('invalid local fixup stride')
        for at in range(start+s['local'],start+s['global'],8):
            src,dst=r.unpack('>2I',at)
            if src==0xffffffff:continue
            r.span(start+src,4,data_end);r.span(start+dst,1,data_end)
            if src in local:raise CollisionFormatError('duplicate local fixup')
            local[src]=dst
        global_fixups={}
        for at in range(start+s['global'],start+s['virtual']-11,12):
            src,section,dst=r.unpack('>3I',at)
            if src==0xffffffff:continue
            if section>=n:raise CollisionFormatError('global fixup section out of bounds')
            r.span(start+src,4,data_end)
            target=sections[section]
            r.span(base+target['absolute']+dst,1,base+target['absolute']+target['local'])
            global_fixups[src]=(section,dst)
        fixups[sn]=(local,global_fixups)
        fixup_total+=len(local)
        for at in range(start+s['virtual'],start+s['exports']-11,12):
            obj,section,nameoff=r.unpack('>3I',at)
            if obj==0xffffffff:continue
            if section>=n:raise CollisionFormatError('class section out of bounds')
            r.span(start+obj,1,data_end)
            target=sections[section]
            name=r.text(base+target['absolute']+nameoff,base+target['absolute']+target['local'])
            counts[name]=counts.get(name,0)+1
            source={'section':sn,'offset':obj,'class':name}
            objects.append(source)
            if name in STRUCTURAL:continue
            if name!=STORAGE:
                unsupported.append({'source_object':source,'reason':'unconverted_serialized_class'})
                continue
            r.span(start+obj,52,data_end)
            vc=r.u32(start+obj+12,data_end);i8=r.u32(start+obj+24,data_end)
            ic=r.u32(start+obj+36,data_end);i32=r.u32(start+obj+48,data_end)
            if vc>MAX_VERTICES or max(i8,ic,i32)>MAX_INDICES:
                raise CollisionFormatError('oversized mesh array')
            if i8 or i32:
                unsupported.append({'source_object':source,'reason':'unverified_index_storage_width'})
                continue
            if ic%4:raise CollisionFormatError('invalid triangle storage stride')
            if not vc or not ic:
                unsupported.append({'source_object':source,'reason':'empty_mesh_storage'})
                continue
            if obj+8 not in local or obj+32 not in local:
                raise CollisionFormatError('mesh array missing local fixup')
            vo=local[obj+8];io=local[obj+32]
            vb=r.span(start+vo,vc*16,data_end);ib=r.span(start+io,ic*2,data_end)
            verts=[];w_values=set()
            for v in struct.iter_unpack('>4f',vb):
                if not all(math.isfinite(x) for x in v):raise CollisionFormatError('nonfinite mesh vertex')
                verts.append(list(v[:3]));w_values.add(v[3])
            triangles=[]
            for row in struct.iter_unpack('>4H',ib):
                if row[3]!=0 or max(row[:3])>=vc:raise CollisionFormatError('invalid triangle index/padding')
                triangles.append(list(row[:3]))
            bounds={'min':[min(v[k] for v in verts) for k in range(3)],'max':[max(v[k] for v in verts) for k in range(3)]}
            meshes.append({'shape_id':f'section{sn}-object{obj:x}','vertices_xyz':verts,'triangles':triangles,'aabb_xyz':bounds,'coordinate_space':'stored_mesh_local','transform':None,'materials':None,'collision_filter':None,'classification':'unresolved','source_object':source,'vertex_padding_w_values':sorted(w_values),'gaps':['mesh_subpart_transform_unresolved','material_mapping_unresolved','collision_filter_semantics_unresolved','world_instance_link_unresolved']})
    bound_storage=set()
    mesh_by_object={(m['source_object']['section'],m['source_object']['offset']):m for m in meshes}
    for source in objects:
        if source['class']!='hkpStorageExtendedMeshShape':continue
        sn=source['section'];obj=source['offset'];section=sections[sn]
        start=base+section['absolute'];end=start+section['local'];local,glob=fixups[sn]
        r.span(start+obj,0xf8,end)
        scale=r.unpack('>4f',start+obj+0x80,end)
        if not all(math.isfinite(v) for v in scale):raise CollisionFormatError('nonfinite mesh scale')
        count=r.u32(start+obj+0xb8,end);stored=r.u32(start+obj+0xe4,end)
        shape_subparts=r.u32(start+obj+0xc4,end)
        if shape_subparts:
            unsupported.append({'source_object':source,'reason':'unconverted_shape_subparts','count':shape_subparts})
        if count!=stored or count>100_000:raise CollisionFormatError('triangle/storage subpart count mismatch')
        if not count:continue
        if obj+0xb4 not in local or obj+0xe0 not in local:raise CollisionFormatError('subpart array missing local fixup')
        tri=local[obj+0xb4];storage=local[obj+0xe0]
        r.span(start+tri,count*0x60,end);r.span(start+storage,count*4,end)
        filter_info=r.u32(start+obj+0xd8,end);radius=r.unpack('>f',start+obj+0xdc,end)[0]
        if not math.isfinite(radius) or radius<0:raise CollisionFormatError('invalid collision margin')
        for i in range(count):
            target=glob.get(storage+4*i)
            if target is None:raise CollisionFormatError('mesh storage missing global fixup')
            mesh=mesh_by_object.get(target)
            if mesh is None:
                unsupported.append({'source_object':source,'reason':'missing_converted_storage_subpart','subpart':i});continue
            if target in bound_storage:raise CollisionFormatError('stored mesh shared by multiple parents; transform policy unverified')
            bound_storage.add(target)
            sub=start+tri+0x60*i
            triangles=r.u32(sub+0x14,end);vertex_stride=r.u32(sub+0x1c,end);vertices=r.u32(sub+0x20,end);index_stride=r.u32(sub+0x44,end)
            if triangles!=len(mesh['triangles']) or vertices!=len(mesh['vertices_xyz']) or vertex_stride!=16 or index_stride!=8:
                raise CollisionFormatError('triangle subpart differs from stored mesh')
            extrusion=r.unpack('>4f',sub+0x30,end)
            if not all(math.isfinite(v) for v in extrusion):raise CollisionFormatError('nonfinite triangle extrusion')
            flip=r.span(sub+0x49,1,end)[0]
            if flip:
                unsupported.append({'source_object':source,'reason':'unverified_alternate_triangle_winding','subpart':i});mesh['gaps'].append('alternate_winding_unverified')
            mesh['parent_shape_object']=source;mesh['subpart_index']=i
            # Shape-wide scaling is authored native Vec4@80. Bake only this
            # verified scalar into neutral arrays; world placement stays absent.
            scaled=[]
            for vertex in mesh['vertices_xyz']:
                try:v=[struct.unpack('>f',struct.pack('>f',vertex[k]*scale[k]))[0] for k in range(3)]
                except (OverflowError,struct.error) as e:raise CollisionFormatError('overflowing scaled vertex') from e
                if not all(math.isfinite(x) for x in v):raise CollisionFormatError('nonfinite scaled vertex')
                scaled.append(v)
            mesh['vertices_xyz']=scaled
            mesh['aabb_xyz']={'min':[min(v[k] for v in scaled) for k in range(3)],'max':[max(v[k] for v in scaled) for k in range(3)]}
            mesh['coordinate_space']='collision_shape_local'
            mesh['transform']={'translation_xyz':[0,0,0],'rotation_xyzw':[0,0,0,1],'scale_xyz':[1,1,1],'scope':'identity after baking native shape scaling; no world transform implied'}
            mesh['source_shape_scaling_xyz']=list(scale[:3]);mesh['extrusion_xyz']=list(extrusion[:3]);mesh['collision_margin']=radius
            mesh['collision_filter']={'default_raw_u32':filter_info,'semantics':'unresolved'}
            mesh['materials']={'subpart_material_count':r.unpack('>H',sub+0xa,end)[0],'effective_material_index':None,'source':'runtime external resource material array override; packfile materialBase is not final'}
            mesh['triangle_offset']=r.u32(sub+0x4c,end)
            mesh['gaps'].remove('mesh_subpart_transform_unresolved')
            if any(extrusion[:3]):mesh['gaps'].append('nonzero_triangle_extrusion_query_unimplemented')
    return {'format':'mag_collision_geometry','schema_version':1,'packfile_length':length,'packfile_sha256':hashlib.sha256(raw).hexdigest(),'native_format':{'version':version,'content_version':content,'pointer_bytes':4,'byte_order':'big','section_count':n},'local_fixup_count':fixup_total,'coordinate_space':'per_mesh_coordinate_space','meshes':meshes,'primitives':[],'shape_class_coverage':counts,'source_objects':objects,'unsupported':unsupported,'world_assembly':{'status':'unresolved','instances':[],'gaps':['authored_instance_and_resource_linkage_unresolved','parent_and_subpart_transforms_unresolved','terrain_snapping_unresolved','static_dynamic_destructible_state_unresolved']}}
