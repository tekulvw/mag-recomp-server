#!/usr/bin/env python3
"""Offline prototype compiled world + native identity terrain assembly.

Runtime output contains only neutral world-space triangle arrays. Authored
prefab translation/snap is not used: embedded compiled collision records bind
final matrices. The matrix already contains uniform scale; do not apply twice.
"""
import argparse,collections,gzip,hashlib,importlib.util,json,math,pathlib,struct,zipfile
from archive import Psarc
ROOT=pathlib.Path(__file__).resolve().parents[2]

def sha(data):return hashlib.sha256(data).hexdigest()
def require(ok,message):
 if not ok:raise ValueError(message)
def f32(value):return struct.unpack('>f',struct.pack('>f',value))[0]
def decode_collision_record(data,offset):
 require(0<=offset<=len(data)-5,'truncated compiled collision header')
 version=struct.unpack_from('>I',data,offset)[0];present=data[offset+4]
 require(version==0x10001 and present in (0,1),'unsupported compiled collision record')
 if not present:return None
 require(offset+77<=len(data),'truncated compiled collision placement')
 values=struct.unpack_from('>17f',data,offset+9)
 require(all(math.isfinite(v) for v in values),'nonfinite compiled transform')
 matrix=[list(values[i:i+4]) for i in range(0,16,4)];scale=values[16]
 require(0<scale<=100 and matrix[3]==[0,0,0,scale],'invalid affine scale/last row')
 error=max(abs(sum(matrix[k][i]*matrix[k][j] for k in range(3))/scale**2-(1 if i==j else 0)) for i in range(3) for j in range(3))
 determinant=(matrix[0][0]*(matrix[1][1]*matrix[2][2]-matrix[1][2]*matrix[2][1])-matrix[0][1]*(matrix[1][0]*matrix[2][2]-matrix[1][2]*matrix[2][0])+matrix[0][2]*(matrix[1][0]*matrix[2][1]-matrix[1][1]*matrix[2][0]))/scale**3
 require(error<1e-4 and abs(determinant-1)<1e-4,'compiled matrix is not scaled proper rotation')
 return dict(token=data[offset+5:offset+9].hex(),matrix=matrix,scale=scale,offset=offset,sha256=sha(data[offset:offset+77]))
def bake_vertex(vertex,placement):
 return [f32(sum(row[i]*vertex[i] for i in range(3))+row[3]) for row in placement['matrix'][:3]]
def nondegenerate_triangles(vertices,triangles):
 kept=[];removed=[]
 for index,face in enumerate(triangles):
  a,b,c=[vertices[i] for i in face]
  u=[b[i]-a[i] for i in range(3)];v=[c[i]-a[i] for i in range(3)]
  n=[u[1]*v[2]-u[2]*v[1],u[2]*v[0]-u[0]*v[2],u[0]*v[1]-u[1]*v[0]]
  require(all(math.isfinite(x) for x in n),'nonfinite baked triangle')
  if any(n):kept.append(face)
  else:removed.append(index)
 return kept,removed
