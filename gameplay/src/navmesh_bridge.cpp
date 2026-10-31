// MAG C ABI bridge; upstream Recast/Detour sources remain unmodified.
#include "Recast.h"
#include "DetourNavMesh.h"
#include "DetourNavMeshBuilder.h"
#include "DetourNavMeshQuery.h"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <new>
#include <vector>
#include <map>
#include <cstdint>

struct MagNav {
    dtNavMesh* mesh;
    dtNavMeshQuery* query;
    std::map<dtPolyRef,int> component;
    ~MagNav() { dtFreeNavMeshQuery(query); dtFreeNavMesh(mesh); }
};
struct BakeObjects {
    rcHeightfield* hf = nullptr;
    rcCompactHeightfield* chf = nullptr;
    rcContourSet* contours = nullptr;
    rcPolyMesh* poly = nullptr;
    rcPolyMeshDetail* detail = nullptr;
    ~BakeObjects() { rcFreeHeightField(hf); rcFreeCompactHeightfield(chf); rcFreeContourSet(contours); rcFreePolyMesh(poly); rcFreePolyMeshDetail(detail); }
};
// Validate every serialized tile layout before Detour forms any pointers.
static bool valid_blob(const unsigned char* data, int size, bool tiled=false) {
    if(size<static_cast<int>(sizeof(dtMeshHeader))) return false;
    dtMeshHeader h;std::memcpy(&h,data,sizeof(h));
    if(h.magic!=DT_NAVMESH_MAGIC || h.version!=DT_NAVMESH_VERSION || h.polyCount<=0 || h.polyCount>65535 || h.vertCount<=0 || h.vertCount>65535 || h.maxLinkCount<=0 || h.maxLinkCount>1000000 || h.detailMeshCount!=h.polyCount || h.detailVertCount<0 || h.detailVertCount>2000000 || h.detailTriCount<0 || h.detailTriCount>2000000 || h.bvNodeCount<0 || h.bvNodeCount>2*h.polyCount || h.offMeshConCount!=0 || h.offMeshBase!=h.polyCount) return false;
    if(!std::isfinite(h.walkableHeight) || !std::isfinite(h.walkableRadius) || !std::isfinite(h.walkableClimb) || !std::isfinite(h.bvQuantFactor) || h.bvQuantFactor<=0 || std::fabs(h.walkableHeight-1.85f)>.0001f || std::fabs(h.walkableRadius-.6f)>.0001f || std::fabs(h.walkableClimb-.35f)>.0001f) return false;
    for(int i=0;i<3;++i) if(!std::isfinite(h.bmin[i]) || !std::isfinite(h.bmax[i]) || h.bmin[i]>h.bmax[i]) return false;
    const auto aligned=[](size_t n){return (n+3u)&~size_t(3u);};
    size_t offset=aligned(sizeof(h));
    const size_t verts=offset;offset+=aligned(sizeof(float)*3u*h.vertCount);
    const size_t polys=offset;offset+=aligned(sizeof(dtPoly)*h.polyCount);
    offset+=aligned(sizeof(dtLink)*h.maxLinkCount);
    const size_t details=offset;offset+=aligned(sizeof(dtPolyDetail)*h.detailMeshCount);
    const size_t detailverts=offset;offset+=aligned(sizeof(float)*3u*h.detailVertCount);
    const size_t detailtris=offset;offset+=aligned(4u*h.detailTriCount);
    const size_t bv=offset;offset+=aligned(sizeof(dtBVNode)*h.bvNodeCount);
    if(offset!=static_cast<size_t>(size)) return false;
    for(int i=0;i<h.vertCount*3;++i){float v;std::memcpy(&v,data+verts+i*sizeof(float),sizeof(v));if(!std::isfinite(v))return false;}
    for(int i=0;i<h.detailVertCount*3;++i){float v;std::memcpy(&v,data+detailverts+i*sizeof(float),sizeof(v));if(!std::isfinite(v))return false;}
    for(int i=0;i<h.polyCount;++i) {
        dtPoly p;std::memcpy(&p,data+polys+i*sizeof(p),sizeof(p));
        if(p.vertCount<3 || p.vertCount>DT_VERTS_PER_POLYGON || p.getType()!=DT_POLYTYPE_GROUND || p.flags!=1) return false;
        for(int j=0;j<p.vertCount;++j) if(p.verts[j]>=h.vertCount || ((p.neis[j]&DT_EXT_LINK) ? (!tiled || (p.neis[j]&~DT_EXT_LINK)>7 || (p.neis[j]&1)) : p.neis[j]>h.polyCount)) return false;
        dtPolyDetail d;std::memcpy(&d,data+details+i*sizeof(d),sizeof(d));
        if(d.vertBase>static_cast<unsigned>(h.detailVertCount) || d.vertCount>h.detailVertCount-d.vertBase || d.triBase>static_cast<unsigned>(h.detailTriCount) || d.triCount>h.detailTriCount-d.triBase) return false;
        for(int j=0;j<d.triCount;++j) for(int k=0;k<3;++k) if(data[detailtris+4u*(d.triBase+j)+k]>=p.vertCount+d.vertCount) return false;
    }
    for(int i=0;i<h.bvNodeCount;++i) {
        dtBVNode n;std::memcpy(&n,data+bv+i*sizeof(n),sizeof(n));
        for(int j=0;j<3;++j)if(n.bmin[j]>n.bmax[j])return false;
        if(n.i>=h.polyCount || (n.i<0 && (static_cast<long long>(-static_cast<long long>(n.i))>h.bvNodeCount-i)))return false;
    }
    return true;
}
extern "C" {
// 0 success, 1 invalid/config, 2 bake failure, 3 allocation failure.
static int bake_tile(const float* verts, int nv, const int* tris, int nt,
                 const float* min, const float* max, unsigned char** output, int* size, int tx, int ty) {
    *output = nullptr; *size = 0;
    if (!verts || !tris || nv < 3 || nt < 1) return 1;
    rcContext ctx;
    rcConfig cfg{};
    cfg.cs=.1f; cfg.ch=.05f; cfg.walkableSlopeAngle=45.f;
    cfg.walkableHeight=37; cfg.walkableClimb=7; cfg.walkableRadius=6;
    cfg.maxEdgeLen=120; cfg.maxSimplificationError=.5f;
    cfg.minRegionArea=64; cfg.mergeRegionArea=400;
    cfg.maxVertsPerPoly=6; cfg.detailSampleDist=.6f; cfg.detailSampleMaxError=.05f;
    rcVcopy(cfg.bmin,min); rcVcopy(cfg.bmax,max);
    cfg.tileSize=512; cfg.borderSize=cfg.walkableRadius+3;
    cfg.bmin[0]=min[0]+tx*51.2f-cfg.borderSize*cfg.cs; cfg.bmin[2]=min[2]+ty*51.2f-cfg.borderSize*cfg.cs;
    cfg.bmax[0]=min[0]+(tx+1)*51.2f+cfg.borderSize*cfg.cs; cfg.bmax[2]=min[2]+(ty+1)*51.2f+cfg.borderSize*cfg.cs;
    cfg.width=cfg.height=cfg.tileSize+2*cfg.borderSize;
    if (cfg.width<1 || cfg.height<1 || static_cast<long long>(cfg.width)*cfg.height > 16000000) return 1;
    BakeObjects b;
    b.hf=rcAllocHeightfield(); if (!b.hf) return 3;
    if (!rcCreateHeightfield(&ctx,*b.hf,cfg.width,cfg.height,cfg.bmin,cfg.bmax,cfg.cs,cfg.ch)) return 2;
    std::vector<unsigned char> areas(nt,RC_NULL_AREA);
    rcMarkWalkableTriangles(&ctx,cfg.walkableSlopeAngle,verts,nv,tris,nt,areas.data());
    if (!rcRasterizeTriangles(&ctx,verts,nv,tris,areas.data(),nt,*b.hf,cfg.walkableClimb)) return 2;
    rcFilterLowHangingWalkableObstacles(&ctx,cfg.walkableClimb,*b.hf);
    rcFilterLedgeSpans(&ctx,cfg.walkableHeight,cfg.walkableClimb,*b.hf);
    rcFilterWalkableLowHeightSpans(&ctx,cfg.walkableHeight,*b.hf);
    b.chf=rcAllocCompactHeightfield(); if (!b.chf) return 3;
    if (!rcBuildCompactHeightfield(&ctx,cfg.walkableHeight,cfg.walkableClimb,*b.hf,*b.chf)) return 2;
    // Exclude full body envelopes outside the verified vertical domain too.
    for (int i=0;i<b.chf->spanCount;++i) {
        const float y=b.chf->bmin[1]+b.chf->spans[i].y*cfg.ch;
        if (y-.205f<min[1] || y+2.205f>max[1]) b.chf->areas[i]=RC_NULL_AREA;
    }
    // Last tiles extend to the regular grid boundary. Geometry outside the
    // verified input domain cannot contribute standing space or erosion support.
    for(int z=0;z<b.chf->height;++z)for(int x=0;x<b.chf->width;++x){
        const float wx=b.chf->bmin[0]+(x+.5f)*cfg.cs,wz=b.chf->bmin[2]+(z+.5f)*cfg.cs;
        if(wx<min[0] || wx>max[0] || wz<min[2] || wz>max[2]){
            const rcCompactCell& c=b.chf->cells[x+z*b.chf->width];
            for(unsigned int i=c.index;i<c.index+c.count;++i)b.chf->areas[i]=RC_NULL_AREA;
        }
    }
    if (!rcErodeWalkableArea(&ctx,cfg.walkableRadius,*b.chf)) return 2;
    if (!rcBuildDistanceField(&ctx,*b.chf) || !rcBuildRegions(&ctx,*b.chf,cfg.borderSize,cfg.minRegionArea,cfg.mergeRegionArea)) return 2;
    b.contours=rcAllocContourSet(); if (!b.contours) return 3;
    if (!rcBuildContours(&ctx,*b.chf,cfg.maxSimplificationError,cfg.maxEdgeLen,*b.contours)) return 2;
    b.poly=rcAllocPolyMesh(); if (!b.poly) return 3;
    if (!rcBuildPolyMesh(&ctx,*b.contours,cfg.maxVertsPerPoly,*b.poly)) return 2;
    if (b.poly->npolys==0) return 4;
    if(b.poly->npolys>16384) return 2;
    b.detail=rcAllocPolyMeshDetail(); if (!b.detail) return 3;
    if (!rcBuildPolyMeshDetail(&ctx,*b.poly,*b.chf,cfg.detailSampleDist,cfg.detailSampleMaxError,*b.detail)) return 2;
    for(int i=0;i<b.poly->npolys;++i) { b.poly->flags[i]=1; b.poly->areas[i]=0; }
    dtNavMeshCreateParams p{};
    p.verts=b.poly->verts; p.vertCount=b.poly->nverts; p.polys=b.poly->polys;
    p.polyAreas=b.poly->areas; p.polyFlags=b.poly->flags; p.polyCount=b.poly->npolys; p.nvp=b.poly->nvp;
    p.detailMeshes=b.detail->meshes; p.detailVerts=b.detail->verts; p.detailVertsCount=b.detail->nverts;
    p.detailTris=b.detail->tris; p.detailTriCount=b.detail->ntris;
    p.tileX=tx; p.tileY=ty; p.tileLayer=0;
    p.walkableHeight=1.85f; p.walkableRadius=.6f; p.walkableClimb=.35f;
    rcVcopy(p.bmin,b.poly->bmin); rcVcopy(p.bmax,b.poly->bmax); p.cs=cfg.cs;p.ch=cfg.ch;p.buildBvTree=true;
    return dtCreateNavMeshData(&p,output,size) ? 0 : 2;
}
void mag_nav_free_bytes(unsigned char* data) { dtFree(data); }
// Bundle is explicitly bounded and contains only serialized Detour tile bytes.
struct BundleHeader { uint32_t magic, version, count; float min[3], max[3], width; };
static const uint32_t BUNDLE_MAGIC=0x324e474d;
int mag_nav_bake(const float* verts,int nv,const int* tris,int nt,const float* min,const float* max,unsigned char** output,int* size) {
    *output=nullptr;*size=0;
    const int nx=static_cast<int>(std::ceil((max[0]-min[0])/51.2f)), nz=static_cast<int>(std::ceil((max[2]-min[2])/51.2f));
    if(nx<1 || nz<1 || nx*nz>256)return 1;
    BundleHeader header{};header.magic=BUNDLE_MAGIC;header.version=1;header.width=51.2f;rcVcopy(header.min,min);rcVcopy(header.max,max);
    std::vector<unsigned char> bytes(sizeof(header));
    for(int z=0;z<nz;++z)for(int x=0;x<nx;++x){
        unsigned char* tile=nullptr;int n=0;const int result=bake_tile(verts,nv,tris,nt,min,max,&tile,&n,x,z);
        if(result==4)continue;
        if(result!=0){dtFree(tile);return result;}
        if(n<=0 || bytes.size()+4u+n>32u*1024u*1024u){dtFree(tile);return 1;}
        uint32_t length=n;const size_t off=bytes.size();bytes.resize(off+4u+n);std::memcpy(bytes.data()+off,&length,4);std::memcpy(bytes.data()+off+4,tile,n);dtFree(tile);++header.count;
    }
    if(!header.count)return 2;
    std::memcpy(bytes.data(),&header,sizeof(header));
    *output=static_cast<unsigned char*>(dtAlloc(bytes.size(),DT_ALLOC_PERM));if(!*output)return 3;
    std::memcpy(*output,bytes.data(),bytes.size());*size=bytes.size();return 0;
}
MagNav* mag_nav_load(const unsigned char* data,int size,const float* min,const float* max) {
    if(size<4 || size>32*1024*1024)return nullptr;
    uint32_t magic;std::memcpy(&magic,data,4);const bool tiled=magic==BUNDLE_MAGIC;
    std::vector<std::pair<const unsigned char*,int>> tiles;
    BundleHeader bundle{};
    if(tiled){
        if(size<static_cast<int>(sizeof(bundle)))return nullptr;
        std::memcpy(&bundle,data,sizeof(bundle));
        if(bundle.version!=1 || bundle.count<1 || bundle.count>256 || bundle.width!=51.2f)return nullptr;
        for(int k=0;k<3;++k)if(!std::isfinite(bundle.min[k]) || !std::isfinite(bundle.max[k]) || std::fabs(bundle.min[k]-min[k])>.001f || std::fabs(bundle.max[k]-max[k])>.001f)return nullptr;
        size_t off=sizeof(bundle);std::map<std::pair<int,int>,bool> coordinates;
        for(uint32_t i=0;i<bundle.count;++i){
            if(off+4>static_cast<size_t>(size))return nullptr;uint32_t n;std::memcpy(&n,data+off,4);off+=4;
            if(n>static_cast<size_t>(size)-off || n>32u*1024u*1024u || !valid_blob(data+off,n,true))return nullptr;
            dtMeshHeader h;std::memcpy(&h,data+off,sizeof(h));
            if(h.x<0 || h.y<0 || h.layer!=0 || h.polyCount>16384 || h.x>=std::ceil((max[0]-min[0])/bundle.width) || h.y>=std::ceil((max[2]-min[2])/bundle.width) || coordinates.count({h.x,h.y}))return nullptr;
            coordinates[{h.x,h.y}]=true;
            const float expected[3]={min[0]+h.x*bundle.width,min[1],min[2]+h.y*bundle.width};
            for(int k=0;k<3;++k)if(std::fabs(h.bmin[k]-expected[k])>.002f || std::fabs(h.bmax[k]-(k==1?max[k]+1.85f:expected[k]+bundle.width))>.002f)return nullptr;
            tiles.push_back({data+off,static_cast<int>(n)});off+=n;
        }
        if(off!=static_cast<size_t>(size))return nullptr;
    }else{
        if(!valid_blob(data,size))return nullptr;
        dtMeshHeader h;std::memcpy(&h,data,sizeof(h));for(int k=0;k<3;++k)if(std::fabs(h.bmin[k]-min[k])>.002f || std::fabs(h.bmax[k]-(max[k]+(k==1?1.85f:0.f)))>.002f)return nullptr;
        tiles.push_back({data,size});
    }
    MagNav* h=new(std::nothrow) MagNav{dtAllocNavMesh(),dtAllocNavMeshQuery()};
    if(!h || !h->mesh || !h->query){delete h;return nullptr;}
    if(tiled){dtNavMeshParams p{};rcVcopy(p.orig,min);p.tileWidth=p.tileHeight=bundle.width;p.maxTiles=256;p.maxPolys=16384;if(dtStatusFailed(h->mesh->init(&p))){delete h;return nullptr;}}
    for(const auto& tile:tiles){unsigned char* copy=static_cast<unsigned char*>(dtAlloc(tile.second,DT_ALLOC_PERM));if(!copy){delete h;return nullptr;}std::memcpy(copy,tile.first,tile.second);
        dtStatus result=tiled?h->mesh->addTile(copy,tile.second,DT_TILE_FREE_DATA,0,nullptr):h->mesh->init(copy,tile.second,DT_TILE_FREE_DATA);
        if(dtStatusFailed(result)){dtFree(copy);delete h;return nullptr;}
    }
    // Preserve the original runtime pool; legacy metadata incorrectly said 8192.
    if(dtStatusFailed(h->query->init(h->mesh,16384))){delete h;return nullptr;}
    // Tile insertion and polygon iteration are deterministic, yielding stable labels.
    int label=0;
    for(int t=0;t<h->mesh->getMaxTiles();++t){const dtMeshTile* tile=static_cast<const dtNavMesh*>(h->mesh)->getTile(t);if(!tile->header)continue;
        const dtPolyRef base=h->mesh->getPolyRefBase(tile);
        for(int i=0;i<tile->header->polyCount;++i){const dtPolyRef ref=base|i;if(h->component.count(ref))continue;
            std::vector<dtPolyRef> stack(1,ref);h->component[ref]=label;
            while(!stack.empty()){const dtPolyRef current=stack.back();stack.pop_back();const dtMeshTile* ct;const dtPoly* cp;if(dtStatusFailed(h->mesh->getTileAndPolyByRef(current,&ct,&cp))){delete h;return nullptr;}
                for(unsigned int link=cp->firstLink;link!=DT_NULL_LINK;link=ct->links[link].next){const dtPolyRef next=ct->links[link].ref;if(next && !h->component.count(next)){h->component[next]=label;stack.push_back(next);}}
            }++label;
        }
    }return h;
}
void mag_nav_destroy(MagNav* h) { delete h; }
// status:0 complete,1 partial,2 disconnected,3 out of nodes,4 invalid start,5 invalid goal,6 query failure.
int mag_nav_route(MagNav* h,const float* start,const float* goal,float radius,float* projected_start,float* projected_goal,
                  float* points,int* point_count,unsigned long long* refs,int* ref_count) {
    *point_count=0;*ref_count=0;
    dtQueryFilter filter; filter.setIncludeFlags(1);filter.setExcludeFlags(0);
    const float se[3]={.75f,2.f,.75f}, ge[3]={radius,8.f,radius};
    dtPolyRef sr=0,gr=0;
    if(dtStatusFailed(h->query->findNearestPoly(start,se,&filter,&sr,projected_start)) || !sr) return 4;
    const float sdx=projected_start[0]-start[0], sdz=projected_start[2]-start[2];
    if(sdx*sdx+sdz*sdz>.15f*.15f) return 4;
    dtPolyRef candidates[512];int candidate_count=0;
    dtStatus candidate_status=h->query->queryPolygons(goal,ge,&filter,candidates,&candidate_count,512);
    if(dtStatusFailed(candidate_status)) return 6;
    struct Candidate { dtPolyRef ref;float p[3];float distance;bool reachable; };
    std::vector<Candidate> goals;
    for(int i=0;i<candidate_count;++i) {
        Candidate c{};c.ref=candidates[i];c.reachable=h->component.at(c.ref)==h->component.at(sr);
        if(dtStatusFailed(h->query->closestPointOnPoly(c.ref,goal,c.p,nullptr))) continue;
        const float dx=c.p[0]-goal[0],dy=c.p[1]-goal[1],dz=c.p[2]-goal[2];c.distance=dx*dx+dy*dy+dz*dz;
        if(c.distance<=radius*radius+.0001f)goals.push_back(c);
    }
    std::sort(goals.begin(),goals.end(),[](const Candidate& a,const Candidate& b){return a.reachable!=b.reachable ? a.reachable>b.reachable : a.distance==b.distance ? a.ref<b.ref : a.distance<b.distance;});
    if(goals.empty()) return 5;
    dtPolyRef path[1024];int count=0;dtStatus status=DT_FAILURE;
    // Prefer the nearest complete reachable standing point; alternatives avoid
    // reporting a roof/island as the whole objective's connectivity result.
    const int attempts=1;
    for(int i=0;i<attempts;++i) {
        dtPolyRef trial[1024];int trial_count=0;
        dtStatus trial_status=h->query->findPath(sr,goals[i].ref,projected_start,goals[i].p,&filter,trial,&trial_count,1024);
        if(dtStatusFailed(trial_status))return 6;
        if(i==0 || (trial_count>0 && trial[trial_count-1]==goals[i].ref && !dtStatusDetail(trial_status,DT_PARTIAL_RESULT|DT_OUT_OF_NODES|DT_BUFFER_TOO_SMALL))) {
            status=trial_status;count=trial_count;std::memcpy(path,trial,count*sizeof(dtPolyRef));gr=goals[i].ref;rcVcopy(projected_goal,goals[i].p);
        }
        if(trial_count>0 && trial[trial_count-1]==goals[i].ref && !dtStatusDetail(trial_status,DT_PARTIAL_RESULT|DT_OUT_OF_NODES|DT_BUFFER_TOO_SMALL))break;
    }
    if(dtStatusFailed(status)) return 6;
    if(!count) return 2;
    float endpoint[3];rcVcopy(endpoint,projected_goal);
    const bool partial=path[count-1]!=gr || dtStatusDetail(status,DT_PARTIAL_RESULT) || dtStatusDetail(status,DT_BUFFER_TOO_SMALL);
    if(partial && dtStatusFailed(h->query->closestPointOnPoly(path[count-1],projected_goal,endpoint,nullptr))) return 6;
    unsigned char flags[1024];dtPolyRef pointrefs[1024];
    dtStatus straight=h->query->findStraightPath(projected_start,endpoint,path,count,points,flags,pointrefs,point_count,1024,0);
    if(dtStatusFailed(straight)) return 6;
    *ref_count=count;for(int i=0;i<count;++i) refs[i]=static_cast<unsigned long long>(path[i]);
    if(dtStatusDetail(status,DT_OUT_OF_NODES)) return 3;
    if(dtStatusDetail(straight,DT_BUFFER_TOO_SMALL)) return 1;
    if(partial) return count==1 ? 2 : 1;
    return 0;
}
int mag_nav_component_id(MagNav* h,const float* position) {
    dtQueryFilter f;f.setIncludeFlags(1);const float e[3]={.75f,2.f,.75f};dtPolyRef ref=0;float p[3];
    if(dtStatusFailed(h->query->findNearestPoly(position,e,&f,&ref,p)) || !ref)return -1;
    if(std::hypot(p[0]-position[0],p[2]-position[2])>.15f)return -1;
    return h->component.at(ref);
}
int mag_nav_goals(MagNav* h,const float* goal,float radius,int* components,float* points,int* count) {
    *count=0;dtQueryFilter f;f.setIncludeFlags(1);const float e[3]={radius,radius,radius};dtPolyRef refs[512];int n=0;
    dtStatus result=h->query->queryPolygons(goal,e,&f,refs,&n,512);if(dtStatusFailed(result) || dtStatusDetail(result,DT_BUFFER_TOO_SMALL))return 0;
    float distances[128];
    for(int i=0;i<n;++i){float p[3];if(dtStatusFailed(h->query->closestPointOnPoly(refs[i],goal,p,nullptr)))continue;float d=0;for(int k=0;k<3;++k)d+=(p[k]-goal[k])*(p[k]-goal[k]);if(d>radius*radius+.0001f)continue;
        const int component=h->component.at(refs[i]);int slot=0;while(slot<*count && components[slot]!=component)++slot;
        if(slot==*count){if(*count==128)return 0;components[slot]=component;distances[slot]=1e30f;++*count;}
        if(d<distances[slot]){distances[slot]=d;rcVcopy(points+3*slot,p);}
    }return 1;
}
// Offline diagnostics only: exact closest point in the start's static component.
int mag_nav_component(MagNav* h,const float* start,const float* goal,float* closest,float* min,float* max,int* count,int* total) {
    dtQueryFilter f;f.setIncludeFlags(1);const float e[3]={.75f,2.f,.75f};dtPolyRef r=0;float ps[3];
    if(dtStatusFailed(h->query->findNearestPoly(start,e,&f,&r,ps)) || !r)return 0;
    const int label=h->component.at(r);float best=1e30f;*count=0;*total=h->component.size();
    for(int k=0;k<3;++k){min[k]=1e30f;max[k]=-1e30f;}
    for(int t=0;t<h->mesh->getMaxTiles();++t){const dtMeshTile* tile=static_cast<const dtNavMesh*>(h->mesh)->getTile(t);if(!tile->header)continue;
    const dtPolyRef base=h->mesh->getPolyRefBase(tile);
    for(int i=0;i<tile->header->polyCount;++i)if(h->component.at(base|i)==label){
        ++*count;float p[3];if(!dtStatusFailed(h->query->closestPointOnPoly(base|i,goal,p,nullptr))){const float dx=p[0]-goal[0],dy=p[1]-goal[1],dz=p[2]-goal[2],distance=dx*dx+dy*dy+dz*dz;if(distance<best){best=distance;rcVcopy(closest,p);}}
        const dtPoly& poly=tile->polys[i];for(int j=0;j<poly.vertCount;++j)for(int k=0;k<3;++k){float v=tile->verts[poly.verts[j]*3+k];min[k]=std::min(min[k],v);max[k]=std::max(max[k],v);}
    }
    }return 1;
}
// Cheap local corridor membership; height tolerance supports the conservative body solver.
int mag_nav_contains(MagNav* h,const float* p,const unsigned long long* refs,int count) {
    dtQueryFilter f;f.setIncludeFlags(1);const float e[3]={.16f,1.f,.16f};
    dtPolyRef nearby[128];int n=0;
    if(dtStatusFailed(h->query->queryPolygons(p,e,&f,nearby,&n,128)))return 0;
    for(int j=0;j<n;++j) {
        bool in_corridor=false;for(int i=0;i<count;++i)if(refs[i]==static_cast<unsigned long long>(nearby[j])){in_corridor=true;break;}
        if(!in_corridor)continue;
        float nearest[3];if(dtStatusFailed(h->query->closestPointOnPoly(nearby[j],p,nearest,nullptr)))continue;
        if(std::fabs(nearest[1]-p[1])<=1.f && std::hypot(nearest[0]-p[0],nearest[2]-p[2])<=.15f)return 1;
    }
    return 0;
}
}
