// MAG C ABI bridge; upstream Recast/Detour sources remain unmodified.
#include "Recast.h"
#include "DetourNavMesh.h"
#include "DetourNavMeshBuilder.h"
#include "DetourNavMeshQuery.h"
#include "DetourNode.h"
#include <algorithm>
#include <cmath>
#include <cstring>
#include <new>
#include <vector>
#include <map>
#include <cstdint>
#include <functional>
#include <queue>

// One directed crossing between two tile clusters, represented by a single
// point on the shared edge of a linked polygon pair.
struct StagePortal {
    int from, to;
    dtPolyRef from_ref, to_ref;
    float point[3];
};
// Placed-structure gating of one polygon (computed by the caller from the
// structures standing now). The polygon's free standing space is sampled on
// a grid and split into connected regions (labels 1..64; 0 is blocked or
// outside the polygon). Each link lists the regions touching its shared
// edge (`full`) and those of them that also reach another link (`through`).
struct StructureGate {
    float ox, oz, cell;
    int nx, nz;
    std::vector<unsigned char> labels;
    std::map<dtPolyRef, std::pair<uint64_t, uint64_t>> links;
};
struct MagNav {
    dtNavMesh* mesh;
    dtNavMeshQuery* query;
    std::map<dtPolyRef,int> component;
    bool reference;
    bool component_partition_separates_links;
    // Staged routing hierarchy. A cluster is a set of polygons in one tile
    // that are connected without leaving the tile; portals join clusters.
    std::vector<int> tile_offset;
    std::vector<int> poly_cluster;
    std::vector<int> cluster_portals;
    std::vector<StagePortal> portals;
    std::map<dtPolyRef, StructureGate> gates;
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
static constexpr float TileWidth = 51.2f;
static constexpr float TileBorder = 9 * .1f;
static constexpr size_t RuntimeByteCap = 32u * 1024u * 1024u;
static constexpr size_t WideRuntimeByteCap = 128u * 1024u * 1024u;
static constexpr size_t ReferenceByteCap = 192u * 1024u * 1024u;
// Mode bits: 1 reference, 2 movement-profiled slope, 4 wide runtime. Wide is a
// runtime layout only; reference bundles already use the wide grid.
struct ModeLimits {
    bool reference, wide;
    int tile_cap, poly_cap;
    size_t byte_cap;
};
static bool mode_limits(int mode, ModeLimits* limits) {
    if (mode < 0 || mode > 7 || (mode & 5) == 5) return false;
    limits->reference = (mode & 1) != 0;
    limits->wide = (mode & 4) != 0;
    // Detour needs at least ten salt bits in a 32-bit reference: 256 tiles
    // allow 16,384 polygons per tile and 512 tiles allow 8,192.
    const bool grid512 = limits->reference || limits->wide;
    limits->tile_cap = grid512 ? 512 : 256;
    limits->poly_cap = grid512 ? 8192 : 16384;
    limits->byte_cap = limits->reference ? ReferenceByteCap : limits->wide ? WideRuntimeByteCap : RuntimeByteCap;
    return true;
}
struct BoundsXZ {
    float min_x, min_z, max_x, max_z;
};
static BoundsXZ tile_bounds(const float* min, int x, int z) {
    return {min[0] + x * TileWidth - TileBorder, min[2] + z * TileWidth - TileBorder,
            min[0] + (x + 1) * TileWidth + TileBorder, min[2] + (z + 1) * TileWidth + TileBorder};
}
static std::vector<BoundsXZ> triangle_bounds(const float* verts, const int* tris, int count) {
    std::vector<BoundsXZ> bounds;
    bounds.reserve(count);
    for (int i = 0; i < count; ++i) {
        const float* a = verts + 3 * tris[3*i];
        const float* b = verts + 3 * tris[3*i + 1];
        const float* c = verts + 3 * tris[3*i + 2];
        bounds.push_back({std::min({a[0], b[0], c[0]}), std::min({a[2], b[2], c[2]}),
                          std::max({a[0], b[0], c[0]}), std::max({a[2], b[2], c[2]})});
    }
    return bounds;
}
static void select_tile_faces(const std::vector<BoundsXZ>& bounds, const int* tris,
                              BoundsXZ tile, std::vector<int>& selected) {
    selected.clear();
    for (size_t i = 0; i < bounds.size(); ++i) {
        const BoundsXZ& face = bounds[i];
        if (face.min_x <= tile.max_x && face.max_x >= tile.min_x
            && face.min_z <= tile.max_z && face.max_z >= tile.min_z) {
            selected.insert(selected.end(), tris + 3*i, tris + 3*i + 3);
        }
    }
}
// Staged routing. A long ground route can need more search nodes than the
// query pool holds. The hierarchy below plans a coarse order of tile clusters
// first and then searches the polygon corridor in bounded pieces between
// crossings on that order. Every piece is an ordinary complete Detour path.
static int cluster_of(const MagNav* h, dtPolyRef ref) {
    unsigned int salt = 0, tile = 0, poly = 0;
    h->mesh->decodePolyId(ref, salt, tile, poly);
    if (tile >= h->tile_offset.size() || h->tile_offset[tile] < 0) return -1;
    const size_t index = static_cast<size_t>(h->tile_offset[tile]) + poly;
    return index < h->poly_cluster.size() ? h->poly_cluster[index] : -1;
}
// Midpoint of the part of polygon edge `link.edge` that the link covers.
static void link_midpoint(const dtMeshTile* tile, const dtPoly& poly, const dtLink& link, float* out) {
    const float* a = &tile->verts[poly.verts[link.edge] * 3];
    const float* b = &tile->verts[poly.verts[(link.edge + 1) % poly.vertCount] * 3];
    float low = 0.f, high = 1.f;
    if (link.side != 0xff) {
        low = link.bmin / 255.f;
        high = link.bmax / 255.f;
    }
    const float t = (low + high) * .5f;
    for (int k = 0; k < 3; ++k) out[k] = a[k] + (b[k] - a[k]) * t;
}
static void poly_center(const dtNavMesh* mesh, dtPolyRef ref, float* center) {
    const dtMeshTile* tile = nullptr;
    const dtPoly* poly = nullptr;
    mesh->getTileAndPolyByRefUnsafe(ref, &tile, &poly);
    center[0] = center[1] = center[2] = 0.f;
    for (int i = 0; i < poly->vertCount; ++i)
        for (int k = 0; k < 3; ++k) center[k] += tile->verts[poly->verts[i] * 3 + k];
    for (int k = 0; k < 3; ++k) center[k] /= static_cast<float>(poly->vertCount);
}
static float distance3(const float* a, const float* b) {
    const float dx = b[0] - a[0], dy = b[1] - a[1], dz = b[2] - a[2];
    return std::sqrt(dx * dx + dy * dy + dz * dz);
}
static bool build_stages(MagNav* h) {
    const dtNavMesh* mesh = h->mesh;
    const int max_tiles = mesh->getMaxTiles();
    h->tile_offset.assign(max_tiles, -1);
    size_t total = 0;
    for (int t = 0; t < max_tiles; ++t) {
        const dtMeshTile* tile = mesh->getTile(t);
        if (!tile->header) continue;
        h->tile_offset[t] = static_cast<int>(total);
        total += tile->header->polyCount;
    }
    h->poly_cluster.assign(total, -1);
    int clusters = 0;
    std::vector<int> stack;
    for (int t = 0; t < max_tiles; ++t) {
        const dtMeshTile* tile = mesh->getTile(t);
        if (!tile->header) continue;
        const int base = h->tile_offset[t];
        for (int i = 0; i < tile->header->polyCount; ++i) {
            if (h->poly_cluster[base + i] >= 0) continue;
            h->poly_cluster[base + i] = clusters;
            stack.assign(1, i);
            while (!stack.empty()) {
                const int current = stack.back();
                stack.pop_back();
                const dtPoly& poly = tile->polys[current];
                for (unsigned int l = poly.firstLink; l != DT_NULL_LINK; l = tile->links[l].next) {
                    const dtPolyRef next = tile->links[l].ref;
                    unsigned int salt = 0, next_tile = 0, next_poly = 0;
                    if (!next) continue;
                    mesh->decodePolyId(next, salt, next_tile, next_poly);
                    if (static_cast<int>(next_tile) != t || h->poly_cluster[base + next_poly] >= 0) continue;
                    h->poly_cluster[base + next_poly] = clusters;
                    stack.push_back(static_cast<int>(next_poly));
                }
            }
            ++clusters;
        }
    }
    std::vector<StagePortal> crossings;
    for (int t = 0; t < max_tiles; ++t) {
        const dtMeshTile* tile = mesh->getTile(t);
        if (!tile->header) continue;
        const dtPolyRef base_ref = mesh->getPolyRefBase(tile);
        for (int i = 0; i < tile->header->polyCount; ++i) {
            const int from = h->poly_cluster[h->tile_offset[t] + i];
            const dtPoly& poly = tile->polys[i];
            for (unsigned int l = poly.firstLink; l != DT_NULL_LINK; l = tile->links[l].next) {
                const dtLink& link = tile->links[l];
                if (!link.ref) continue;
                const int to = cluster_of(h, link.ref);
                if (to < 0 || to == from) continue;
                StagePortal crossing{from, to, base_ref | static_cast<dtPolyRef>(i), link.ref, {0.f, 0.f, 0.f}};
                link_midpoint(tile, poly, link, crossing.point);
                crossings.push_back(crossing);
            }
        }
    }
    std::stable_sort(crossings.begin(), crossings.end(), [](const StagePortal& a, const StagePortal& b) {
        return a.from != b.from ? a.from < b.from : a.to < b.to;
    });
    // One representative crossing per ordered cluster pair: the actual
    // crossing nearest the mean of all of them.
    h->portals.clear();
    for (size_t begin = 0; begin < crossings.size();) {
        size_t end = begin;
        float mean[3] = {0.f, 0.f, 0.f};
        while (end < crossings.size() && crossings[end].from == crossings[begin].from
               && crossings[end].to == crossings[begin].to) {
            for (int k = 0; k < 3; ++k) mean[k] += crossings[end].point[k];
            ++end;
        }
        for (int k = 0; k < 3; ++k) mean[k] /= static_cast<float>(end - begin);
        size_t best = begin;
        for (size_t i = begin + 1; i < end; ++i)
            if (distance3(crossings[i].point, mean) < distance3(crossings[best].point, mean)) best = i;
        h->portals.push_back(crossings[best]);
        begin = end;
    }
    h->cluster_portals.assign(clusters + 1, 0);
    for (const StagePortal& portal : h->portals) ++h->cluster_portals[portal.from + 1];
    for (int c = 0; c < clusters; ++c) h->cluster_portals[c + 1] += h->cluster_portals[c];
    return true;
}
// Least-cost order of portals from the start cluster to the goal cluster,
// costing straight lines between consecutive crossing points.
static bool coarse_route(const MagNav* h, int start_cluster, const float* start,
                         int goal_cluster, const float* goal, std::vector<int>& route) {
    route.clear();
    const int n = static_cast<int>(h->portals.size());
    std::vector<float> cost(n + 1, 3.0e38f);
    std::vector<int> parent(n + 1, -1);
    std::vector<char> closed(n + 1, 0);
    typedef std::pair<float, int> Item;
    std::priority_queue<Item, std::vector<Item>, std::greater<Item>> open;
    const auto relax = [&](int from, const float* at, int cluster, float base) {
        if (cluster == goal_cluster) {
            const float total = base + distance3(at, goal);
            if (total < cost[n]) { cost[n] = total; parent[n] = from; open.push(Item(total, n)); }
        }
        for (int q = h->cluster_portals[cluster]; q < h->cluster_portals[cluster + 1]; ++q) {
            const float total = base + distance3(at, h->portals[q].point);
            if (total < cost[q]) {
                cost[q] = total;
                parent[q] = from;
                open.push(Item(total + distance3(h->portals[q].point, goal), q));
            }
        }
    };
    relax(-1, start, start_cluster, 0.f);
    while (!open.empty()) {
        const int current = open.top().second;
        open.pop();
        if (closed[current]) continue;
        closed[current] = 1;
        if (current == n) {
            for (int p = parent[n]; p >= 0; p = parent[p]) route.push_back(p);
            std::reverse(route.begin(), route.end());
            return true;
        }
        const StagePortal& portal = h->portals[current];
        relax(current, portal.point, portal.to, cost[current]);
    }
    return false;
}
// Corridor polygons joined at shared polygons; a revisited polygon closes a
// loop, which is cut so the corridor stays a simple chain of linked polygons.
static void append_without_loops(std::vector<dtPolyRef>& corridor, std::map<dtPolyRef, size_t>& seen,
                                 const dtPolyRef* path, int count) {
    for (int i = 0; i < count; ++i) {
        const auto known = seen.find(path[i]);
        if (known == seen.end()) {
            seen.emplace(path[i], corridor.size());
            corridor.push_back(path[i]);
            continue;
        }
        for (size_t j = known->second + 1; j < corridor.size(); ++j) seen.erase(corridor[j]);
        corridor.resize(known->second + 1);
    }
}
static constexpr float StageSpan = 96.f;
// Polygons a staged route's corridor and straight path may hold. Long routes on
// finely divided meshes (slopes up to the retail limit add polygons) need more
// than a direct search's 1,024; the caller's buffers hold this many.
static constexpr int MaxRouteCorridor = 4096;
// Search nodes one staged route may use in total, in query pools, including
// the direct attempt that ran out of nodes first. A piece is only started
// while a whole pool still fits, so this is a hard bound.
static constexpr int StagePools = 4;
static constexpr size_t StageCorridorCap = 8192;
// status as mag_nav_route: 0 complete, 1 partial, 3 out of nodes, 6 failure.
static int staged_corridor(MagNav* h, const dtQueryFilter& filter, dtPolyRef start_ref, dtPolyRef goal_ref,
                           const float* start, const float* goal, int node_budget,
                           std::vector<dtPolyRef>& corridor, int* nodes, int* stages) {
    corridor.clear();
    const int start_cluster = cluster_of(h, start_ref), goal_cluster = cluster_of(h, goal_ref);
    std::vector<int> coarse;
    if (start_cluster < 0 || goal_cluster < 0 || start_cluster == goal_cluster
        || !coarse_route(h, start_cluster, start, goal_cluster, goal, coarse)) return 3;
    const int targets = static_cast<int>(coarse.size()) + 1;
    const auto target_ref = [&](int k) { return k + 1 == targets ? goal_ref : h->portals[coarse[k]].to_ref; };
    const auto target_point = [&](int k) { return k + 1 == targets ? goal : h->portals[coarse[k]].point; };
    std::vector<float> along(targets, 0.f);
    for (int k = 0; k < targets; ++k)
        along[k] = (k ? along[k - 1] : 0.f) + distance3(k ? target_point(k - 1) : start, target_point(k));
    std::map<dtPolyRef, size_t> seen;
    dtPolyRef piece[1024];
    float centers[1024];
    dtPolyRef current_ref = start_ref;
    float current[3];
    rcVcopy(current, start);
    // `next` is the first target still ahead and `to_next` the distance to it.
    int next = 0;
    float to_next = along[0];
    // Each piece advances at least one polygon; the cap only guards the loop.
    for (int iteration = 0; iteration < 4 * targets + 64; ++iteration) {
        int k = next;
        while (k + 1 < targets && to_next + along[k + 1] - along[next] <= StageSpan) ++k;
        int count = 0;
        while (true) {
            if (*nodes + h->query->getNodePool()->getMaxNodes() > node_budget) return 3;
            const dtStatus status = h->query->findPath(current_ref, target_ref(k), current, target_point(k),
                                                       &filter, piece, &count, 1024);
            *nodes += h->query->getNodePool()->getNodeCount();
            if (dtStatusFailed(status)) return 6;
            if (count > 0 && piece[count - 1] == target_ref(k)
                && !dtStatusDetail(status, DT_OUT_OF_NODES | DT_PARTIAL_RESULT | DT_BUFFER_TOO_SMALL)) break;
            if (k == next) return dtStatusDetail(status, DT_OUT_OF_NODES) ? 3 : 1;
            k = next + (k - next) / 2;
        }
        ++*stages;
        if (k + 1 == targets) {
            append_without_loops(corridor, seen, piece, count);
            return corridor.size() > StageCorridorCap ? 1 : 0;
        }
        // Keep only the first half of a piece that ends at a crossing: the
        // next piece starts inside this locally shortest path, so the route
        // is not pinned to the representative crossing points.
        float previous[3], walked = 0.f;
        poly_center(h->mesh, piece[0], previous);
        centers[0] = 0.f;
        for (int i = 1; i < count; ++i) {
            float center[3];
            poly_center(h->mesh, piece[i], center);
            walked += distance3(previous, center);
            centers[i] = walked;
            rcVcopy(previous, center);
        }
        int keep = count;
        if (count > 2 && walked > StageSpan * .25f) {
            keep = 2;
            while (keep < count && centers[keep - 1] < walked * .5f) ++keep;
        }
        append_without_loops(corridor, seen, piece, keep);
        if (corridor.size() > StageCorridorCap) return 1;
        if (keep == count) {
            current_ref = target_ref(k);
            rcVcopy(current, target_point(k));
            next = k + 1;
            to_next = along[next] - along[k];
        } else {
            // Still heading for target k, over the rest of this piece.
            current_ref = piece[keep - 1];
            poly_center(h->mesh, current_ref, current);
            next = k;
            to_next = walked - centers[keep - 1];
        }
    }
    return 3;
}
static int staged_route(MagNav* h, const dtQueryFilter& filter, dtPolyRef start_ref, dtPolyRef goal_ref,
                        const float* start, const float* goal, float* points, int* point_count,
                        unsigned long long* refs, int* ref_count, int* nodes, int* stages) {
    std::vector<dtPolyRef> corridor;
    const int budget = StagePools * h->query->getNodePool()->getMaxNodes();
    const int result = staged_corridor(h, filter, start_ref, goal_ref, start, goal, budget, corridor, nodes, stages);
    if (result != 0) return result;
    if (corridor.size() > static_cast<size_t>(MaxRouteCorridor)) return 1;
    std::vector<unsigned char> flags(MaxRouteCorridor);
    std::vector<dtPolyRef> pointrefs(MaxRouteCorridor);
    const dtStatus straight = h->query->findStraightPath(start, goal, corridor.data(), static_cast<int>(corridor.size()),
                                                         points, flags.data(), pointrefs.data(), point_count,
                                                         MaxRouteCorridor, 0);
    if (dtStatusFailed(straight)) return 6;
    *ref_count = static_cast<int>(corridor.size());
    for (size_t i = 0; i < corridor.size(); ++i) refs[i] = static_cast<unsigned long long>(corridor[i]);
    return dtStatusDetail(straight, DT_BUFFER_TOO_SMALL) ? 1 : 0;
}
// Regions of a gated polygon a point stands in: its cell's region, else the
// nearest labelled cell within four cells. No region nearby: every region,
// so a point inside a wall never seals a route by itself.
static uint64_t gate_point_regions(const StructureGate& g, const float* p) {
    const int cx = static_cast<int>(std::floor((p[0] - g.ox) / g.cell));
    const int cz = static_cast<int>(std::floor((p[2] - g.oz) / g.cell));
    int best = -1; long best_d = 1L << 40;
    for (int dz = -4; dz <= 4; ++dz) for (int dx = -4; dx <= 4; ++dx) {
        const int x = cx + dx, z = cz + dz;
        if (x < 0 || z < 0 || x >= g.nx || z >= g.nz) continue;
        const int label = g.labels[x + z * g.nx];
        const long d = static_cast<long>(dx) * dx + static_cast<long>(dz) * dz;
        if (label && d < best_d) { best_d = d; best = label; }
    }
    return best > 0 ? (uint64_t(1) << (best - 1)) : ~uint64_t(0);
}
// Whether a route may cross polygon `cur`, entering over the link from `prev`
// (or starting at `pa` when prev is 0) and leaving over the link to `next`
// (or ending at `pb`). The entry and exit must share a region; an entry edge
// touching several through regions (a wall crossing that edge) is not known
// to be in any one of them, so every one must reach the exit.
static bool gate_passes(const MagNav* h, dtPolyRef prev, dtPolyRef cur, dtPolyRef next,
                        const float* pa, const float* pb) {
    const auto found = h->gates.find(cur);
    if (found == h->gates.end()) return true;
    const StructureGate& g = found->second;
    const auto side = [&](dtPolyRef ref, const float* point, bool* ambiguous_through, uint64_t* through) {
        *ambiguous_through = false; *through = 0;
        if (!ref) return gate_point_regions(g, point);
        const auto link = g.links.find(ref);
        if (link == g.links.end()) return ~uint64_t(0);
        *through = link->second.second;
        *ambiguous_through = (*through & (*through - 1)) != 0;
        return link->second.first;
    };
    bool ambiguous = false, unused = false; uint64_t through = 0, exit_through = 0;
    const uint64_t entry = side(prev, pa, &ambiguous, &through);
    const uint64_t exit = side(next, pb, &unused, &exit_through);
    if ((entry & exit) == 0) return false;
    return !(prev && ambiguous && (through & ~exit) != 0);
}
static constexpr float GateBlockedCost = 1.0e7f;
// Detour's default costs, plus a prohibitive cost on a crossing the standing
// structures seal. A search takes one only when nothing else reaches the
// goal; `gate_truncate` then cuts the corridor there.
class GateFilter : public dtQueryFilter {
public:
    GateFilter(const MagNav* h, bool enabled) : h(h), enabled(enabled) { setIncludeFlags(1); setExcludeFlags(0); }
    float getCost(const float* pa, const float* pb,
                  const dtPolyRef prevRef, const dtMeshTile* prevTile, const dtPoly* prevPoly,
                  const dtPolyRef curRef, const dtMeshTile* curTile, const dtPoly* curPoly,
                  const dtPolyRef nextRef, const dtMeshTile* nextTile, const dtPoly* nextPoly) const override {
        float cost = dtQueryFilter::getCost(pa, pb, prevRef, prevTile, prevPoly, curRef, curTile, curPoly,
                                            nextRef, nextTile, nextPoly);
        if (enabled && !h->gates.empty() && !gate_passes(h, prevRef, curRef, nextRef, pa, pb)) cost += GateBlockedCost;
        return cost;
    }
private:
    const MagNav* h;
    bool enabled;
};
// First corridor index whose crossing a standing structure seals, or -1.
static int gate_violation(const MagNav* h, const unsigned long long* refs, int count,
                          const float* start, const float* goal) {
    if (h->gates.empty()) return -1;
    for (int i = 0; i < count; ++i) {
        const dtPolyRef prev = i ? refs[i - 1] : 0, next = i + 1 < count ? refs[i + 1] : 0;
        if (!gate_passes(h, prev, refs[i], next, start, goal)) return i;
    }
    return -1;
}
// The point of polygon `ref`'s entry regions nearest `goal`, for a corridor
// cut at that polygon; the closest polygon point when it is not gated.
static bool gate_endpoint(const MagNav* h, dtPolyRef prev, dtPolyRef ref, const float* start,
                          const float* goal, float* out) {
    if (dtStatusFailed(h->query->closestPointOnPoly(ref, goal, out, nullptr))) return false;
    const auto found = h->gates.find(ref);
    if (found == h->gates.end()) return true;
    const StructureGate& g = found->second;
    uint64_t entry = ~uint64_t(0);
    if (!prev) entry = gate_point_regions(g, start);
    else { const auto link = g.links.find(prev); if (link != g.links.end()) entry = link->second.first; }
    float best = 1e30f; bool any = false; float point[3];
    for (int z = 0; z < g.nz; ++z) for (int x = 0; x < g.nx; ++x) {
        const int label = g.labels[x + z * g.nx];
        if (!label || !(entry & (uint64_t(1) << (label - 1)))) continue;
        const float px = g.ox + (x + .5f) * g.cell, pz = g.oz + (z + .5f) * g.cell;
        const float d = (px - goal[0]) * (px - goal[0]) + (pz - goal[2]) * (pz - goal[2]);
        if (d < best) { best = d; point[0] = px; point[1] = out[1]; point[2] = pz; any = true; }
    }
    if (!any) return true;
    float height = point[1];
    if (dtStatusSucceed(h->query->getPolyHeight(ref, point, &height))) point[1] = height;
    rcVcopy(out, point);
    return true;
}
extern "C" {
// 0 success, 1 invalid/config, 2 bake failure, 3 allocation failure.
static int bake_tile(const float* verts, int nv, const int* tris, int nt,
                 const float* min, const float* max, unsigned char** output, int* size,
                 int tx, int ty, int max_polys, float slope_degrees) {
    *output = nullptr; *size = 0;
    if (!verts || !tris || nv < 3 || nt < 1) return 1;
    rcContext ctx;
    rcConfig cfg{};
    cfg.cs=.1f; cfg.ch=.05f; cfg.walkableSlopeAngle=slope_degrees;
    cfg.walkableHeight=37; cfg.walkableClimb=7; cfg.walkableRadius=6;
    cfg.maxEdgeLen=120; cfg.maxSimplificationError=.5f;
    cfg.minRegionArea=64; cfg.mergeRegionArea=400;
    cfg.maxVertsPerPoly=6; cfg.detailSampleDist=.6f; cfg.detailSampleMaxError=.05f;
    rcVcopy(cfg.bmin,min); rcVcopy(cfg.bmax,max);
    cfg.tileSize=512; cfg.borderSize=cfg.walkableRadius+3;
    const BoundsXZ tile = tile_bounds(min, tx, ty);
    cfg.bmin[0]=tile.min_x; cfg.bmin[2]=tile.min_z;
    cfg.bmax[0]=tile.max_x; cfg.bmax[2]=tile.max_z;
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
    if(b.poly->npolys>max_polys) return 5;
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
static uint32_t bundle_version(int mode) {
    // Legacy runtime and reference versions are 1 and 2. The profiled
    // equivalents use 3 and 4, with the same respective tile capacities.
    // Wide runtime bundles use 5 and, when profiled, 7.
    return static_cast<uint32_t>(mode) + 1;
}
int mag_nav_bake(const float* verts,int nv,const int* tris,int nt,const float* min,const float* max,
                 unsigned char** output,int* size,int mode,float slope_degrees) {
    *output=nullptr;*size=0;
    ModeLimits limits;
    if (!mode_limits(mode, &limits) || !std::isfinite(slope_degrees) || slope_degrees<=0 || slope_degrees>60
        || (!(mode&2) && slope_degrees!=45.f)) return 1;
    const int tile_cap = limits.tile_cap;
    const int poly_cap = limits.poly_cap;
    const size_t byte_cap = limits.byte_cap;
    // Face culling yields the same tile bytes; it only bounds per-tile work.
    const bool cull = limits.reference || limits.wide;
    const int nx=static_cast<int>(std::ceil((max[0]-min[0])/TileWidth)), nz=static_cast<int>(std::ceil((max[2]-min[2])/TileWidth));
    if(nx<1 || nz<1 || nx*nz>tile_cap)return 1;
    BundleHeader header{};header.magic=BUNDLE_MAGIC;header.version=bundle_version(mode);header.width=TileWidth;rcVcopy(header.min,min);rcVcopy(header.max,max);
    const auto bounds = cull ? triangle_bounds(verts, tris, nt) : std::vector<BoundsXZ>{};
    std::vector<int> selected;
    std::vector<unsigned char> bytes(sizeof(header));
    for(int z=0;z<nz;++z)for(int x=0;x<nx;++x){
        if (cull) {
            select_tile_faces(bounds, tris, tile_bounds(min, x, z), selected);
            if (selected.empty()) continue;
        }
        unsigned char* tile=nullptr;int n=0;
        const int result=bake_tile(verts,nv,cull ? selected.data() : tris,
            cull ? static_cast<int>(selected.size()/3) : nt,min,max,&tile,&n,x,z,poly_cap,slope_degrees);
        if(result==4)continue;
        if(result!=0){dtFree(tile);return result;}
        if(n<=0 || bytes.size()+4u+n>byte_cap){dtFree(tile);return 6;}
        uint32_t length=n;const size_t off=bytes.size();bytes.resize(off+4u+n);std::memcpy(bytes.data()+off,&length,4);std::memcpy(bytes.data()+off+4,tile,n);dtFree(tile);++header.count;
    }
    if(!header.count)return 2;
    std::memcpy(bytes.data(),&header,sizeof(header));
    *output=static_cast<unsigned char*>(dtAlloc(bytes.size(),DT_ALLOC_PERM));if(!*output)return 3;
    std::memcpy(*output,bytes.data(),bytes.size());*size=bytes.size();return 0;
}
MagNav* mag_nav_load(const unsigned char* data,int size,const float* min,const float* max,
                    int mode,int node_limit) {
    ModeLimits limits;
    if (!mode_limits(mode, &limits)) return nullptr;
    const bool reference = limits.reference;
    const int tile_cap = limits.tile_cap;
    const int poly_cap = limits.poly_cap;
    const size_t byte_cap = limits.byte_cap;
    if(size<4 || static_cast<size_t>(size)>byte_cap || node_limit<1 || node_limit>16384)return nullptr;
    uint32_t magic;std::memcpy(&magic,data,4);const bool tiled=magic==BUNDLE_MAGIC;
    if (mode!=0 && !tiled) return nullptr;
    std::vector<std::pair<const unsigned char*,int>> tiles;
    BundleHeader bundle{};
    if(tiled){
        if(size<static_cast<int>(sizeof(bundle)))return nullptr;
        std::memcpy(&bundle,data,sizeof(bundle));
        if(bundle.version!=bundle_version(mode) || bundle.count<1 || bundle.count>static_cast<unsigned>(tile_cap) || bundle.width!=TileWidth)return nullptr;
        for(int k=0;k<3;++k)if(!std::isfinite(bundle.min[k]) || !std::isfinite(bundle.max[k]) || std::fabs(bundle.min[k]-min[k])>.001f || std::fabs(bundle.max[k]-max[k])>.001f)return nullptr;
        const double grid_x = std::ceil((max[0]-min[0])/bundle.width);
        const double grid_z = std::ceil((max[2]-min[2])/bundle.width);
        if ((reference || limits.wide) && (grid_x < 1 || grid_z < 1 || grid_x*grid_z > tile_cap)) return nullptr;
        size_t off=sizeof(bundle);std::map<std::pair<int,int>,bool> coordinates;
        for(uint32_t i=0;i<bundle.count;++i){
            if(off+4>static_cast<size_t>(size))return nullptr;uint32_t n;std::memcpy(&n,data+off,4);off+=4;
            if(n>static_cast<size_t>(size)-off || n>byte_cap || !valid_blob(data+off,n,true))return nullptr;
            dtMeshHeader h;std::memcpy(&h,data+off,sizeof(h));
            if(h.x<0 || h.y<0 || h.layer!=0 || h.polyCount>poly_cap || h.x>=grid_x || h.y>=grid_z || coordinates.count({h.x,h.y}))return nullptr;
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
    MagNav* h=new(std::nothrow) MagNav{dtAllocNavMesh(),dtAllocNavMeshQuery(),{},reference,true};
    if(!h || !h->mesh || !h->query){delete h;return nullptr;}
    if(tiled){dtNavMeshParams p{};rcVcopy(p.orig,min);p.tileWidth=p.tileHeight=bundle.width;p.maxTiles=tile_cap;p.maxPolys=poly_cap;if(dtStatusFailed(h->mesh->init(&p))){delete h;return nullptr;}}
    for(const auto& tile:tiles){unsigned char* copy=static_cast<unsigned char*>(dtAlloc(tile.second,DT_ALLOC_PERM));if(!copy){delete h;return nullptr;}std::memcpy(copy,tile.first,tile.second);
        dtStatus result=tiled?h->mesh->addTile(copy,tile.second,DT_TILE_FREE_DATA,0,nullptr):h->mesh->init(copy,tile.second,DT_TILE_FREE_DATA);
        if(dtStatusFailed(result)){dtFree(copy);delete h;return nullptr;}
    }
    // Preserve the original runtime pool; legacy metadata incorrectly said 8192.
    if(dtStatusFailed(h->query->init(h->mesh,node_limit))){delete h;return nullptr;}
    // Tile insertion and polygon iteration are deterministic, yielding stable labels.
    int label=0;
    for(int t=0;t<h->mesh->getMaxTiles();++t){const dtMeshTile* tile=static_cast<const dtNavMesh*>(h->mesh)->getTile(t);if(!tile->header)continue;
        const dtPolyRef base=h->mesh->getPolyRefBase(tile);
        for(int i=0;i<tile->header->polyCount;++i){const dtPolyRef ref=base|i;if(h->component.count(ref))continue;
            std::vector<dtPolyRef> stack(1,ref);h->component[ref]=label;
            while(!stack.empty()){const dtPolyRef current=stack.back();stack.pop_back();const dtMeshTile* ct;const dtPoly* cp;if(dtStatusFailed(h->mesh->getTileAndPolyByRef(current,&ct,&cp))){delete h;return nullptr;}
                for(unsigned int link=cp->firstLink;link!=DT_NULL_LINK;link=ct->links[link].next) {
                    const dtPolyRef next=ct->links[link].ref;
                    if (!next) continue;
                    const auto known = h->component.find(next);
                    if (known == h->component.end()) {
                        h->component.emplace(next, label);
                        stack.push_back(next);
                    } else if (known->second != label) {
                        // A directed edge into an earlier label means labels
                        // alone cannot prove the two endpoints disconnected.
                        h->component_partition_separates_links = false;
                    }
                }
            }++label;
        }
    }
    if(!reference && !build_stages(h)){delete h;return nullptr;}
    return h;
}
void mag_nav_destroy(MagNav* h) { delete h; }
void mag_nav_tile_stats(const MagNav* h, int* occupied, int* max_polygons) {
    *occupied = *max_polygons = 0;
    const dtNavMesh* mesh = h->mesh;
    for (int i = 0; i < mesh->getMaxTiles(); ++i) {
        const dtMeshHeader* header = mesh->getTile(i)->header;
        if (!header) continue;
        ++*occupied;
        *max_polygons = std::max(*max_polygons, header->polyCount);
    }
}
static bool has_forward_link(const dtNavMesh* mesh, dtPolyRef from, dtPolyRef to) {
    const dtMeshTile* tile;
    const dtPoly* poly;
    mesh->getTileAndPolyByRefUnsafe(from, &tile, &poly);
    for (unsigned link = poly->firstLink; link != DT_NULL_LINK; link = tile->links[link].next) {
        if (tile->links[link].ref == to) return true;
    }
    return false;
}

static bool reverse_reference_route(MagNav* h, const dtQueryFilter& filter,
    dtPolyRef start, dtPolyRef goal, const float* projected_start, const float* projected_goal,
    float* points, int* point_count, unsigned long long* refs, int* ref_count) {
    dtPolyRef reverse[1024];
    int count = 0;
    const dtStatus search = h->query->findPath(goal, start, projected_goal, projected_start,
                                              &filter, reverse, &count, 1024);
    if (dtStatusFailed(search) || dtStatusDetail(search, DT_OUT_OF_NODES | DT_PARTIAL_RESULT | DT_BUFFER_TOO_SMALL)
        || count == 0 || reverse[0] != goal || reverse[count - 1] != start) return false;
    std::reverse(reverse, reverse + count);
    for (int i = 1; i < count; ++i) {
        if (!has_forward_link(h->mesh, reverse[i-1], reverse[i])) return false;
    }

    unsigned char flags[1024];
    dtPolyRef pointrefs[1024];
    int trial_points = 0;
    const dtStatus straight = h->query->findStraightPath(projected_start, projected_goal, reverse,
        count, points, flags, pointrefs, &trial_points, 1024, 0);
    if (dtStatusFailed(straight) || dtStatusDetail(straight, DT_BUFFER_TOO_SMALL | DT_PARTIAL_RESULT)
        || trial_points == 0 || !(flags[trial_points - 1] & DT_STRAIGHTPATH_END)) return false;
    *point_count = trial_points;
    *ref_count = count;
    for (int i = 0; i < count; ++i) refs[i] = static_cast<unsigned long long>(reverse[i]);
    return true;
}

// Every polygon a goal box touches whose closest point lies within `radius`
// of the goal, in Detour's visiting order. Detour hands polygons over in
// batches, so there is no result cap: layered floors and narrow ledges can
// put more polygons in one goal box than a fixed buffer holds, and a
// truncated set can silently lose the nearest polygon or fail the query.
// With a positive `height` the region is instead a vertical column: within
// `radius` horizontally and `height` vertically of the goal.
class GoalPolygons : public dtPolyQuery {
public:
    struct Hit { dtPolyRef ref; float p[3]; float distance; };
    GoalPolygons(const dtNavMeshQuery* query,const float* goal,float radius,float height=0):query(query),goal(goal),limit(radius*radius+.0001f),height(height) {}
    void process(const dtMeshTile*,dtPoly**,dtPolyRef* refs,int count) override {
        for(int i=0;i<count;++i){Hit hit{};hit.ref=refs[i];
            if(dtStatusFailed(query->closestPointOnPoly(hit.ref,goal,hit.p,nullptr)))continue;
            for(int k=0;k<3;++k)hit.distance+=(hit.p[k]-goal[k])*(hit.p[k]-goal[k]);
            const float dx=hit.p[0]-goal[0],dz=hit.p[2]-goal[2];
            const bool inside=height>0 ? dx*dx+dz*dz<=limit && std::fabs(hit.p[1]-goal[1])<=height+.0001f : hit.distance<=limit;
            if(inside)hits.push_back(hit);}
    }
    std::vector<Hit> hits;
private:
    const dtNavMeshQuery* query;const float* goal;float limit;float height;
};
// status:0 complete,1 partial,2 disconnected,3 out of nodes,4 invalid start,5 invalid goal,6 query failure.
// `nodes` reports search nodes used and `stages` the corridor pieces of a staged route (0 when direct).
static int route_impl(MagNav* h,const float* start,const float* goal,float radius,float* projected_start,float* projected_goal,
                  float* points,int* point_count,unsigned long long* refs,int* ref_count,int* nodes,int* stages,bool gated) {
    *point_count=0;*ref_count=0;*nodes=0;*stages=0;
    GateFilter filter(h,gated);
    const float se[3]={.75f,2.f,.75f}, ge[3]={radius,8.f,radius};
    dtPolyRef sr=0;
    if(dtStatusFailed(h->query->findNearestPoly(start,se,&filter,&sr,projected_start)) || !sr) return 4;
    const float sdx=projected_start[0]-start[0], sdz=projected_start[2]-start[2];
    if(sdx*sdx+sdz*sdz>.15f*.15f) return 4;
    GoalPolygons candidates(h->query,goal,radius);
    if(dtStatusFailed(h->query->queryPolygons(goal,ge,&filter,&candidates))) return 6;
    struct Candidate { dtPolyRef ref;float p[3];float distance;bool reachable; };
    std::vector<Candidate> goals;
    for(const GoalPolygons::Hit& hit:candidates.hits) {
        Candidate c{};c.ref=hit.ref;c.reachable=h->component.at(c.ref)==h->component.at(sr);
        rcVcopy(c.p,hit.p);c.distance=hit.distance;goals.push_back(c);
    }
    std::sort(goals.begin(),goals.end(),[](const Candidate& a,const Candidate& b){return a.reachable!=b.reachable ? a.reachable>b.reachable : a.distance==b.distance ? a.ref<b.ref : a.distance<b.distance;});
    if(goals.empty()) return 5;
    const Candidate& selected = goals[0];
    const dtPolyRef gr = selected.ref;
    rcVcopy(projected_goal, selected.p);
    if (h->reference && h->component_partition_separates_links && !selected.reachable) return 2;

    dtPolyRef path[1024];
    int count = 0;
    const dtStatus status = h->query->findPath(sr, gr, projected_start, projected_goal,
                                               &filter, path, &count, 1024);
    if(dtStatusFailed(status)) return 6;
    *nodes = h->query->getNodePool()->getNodeCount();
    if(!count) return h->reference ? 1 : 2;
    if (h->reference && selected.reachable && dtStatusDetail(status, DT_OUT_OF_NODES)
        && dtStatusDetail(status, DT_PARTIAL_RESULT)
        && reverse_reference_route(h, filter, sr, gr, projected_start, projected_goal,
                                   points, point_count, refs, ref_count)) return 0;
    // A runtime route that ran out of nodes, or whose path outgrew the direct
    // search's 1,024 polygons, inside one walking component is retried in
    // stages. If that fails too, the direct partial result stands.
    if (!h->reference && selected.reachable
        && dtStatusDetail(status, DT_OUT_OF_NODES | DT_BUFFER_TOO_SMALL)) {
        if (staged_route(h, filter, sr, gr, projected_start, projected_goal, points, point_count,
                         refs, ref_count, nodes, stages) == 0) return 0;
        *point_count = 0;
        *ref_count = 0;
    }
    float endpoint[3];rcVcopy(endpoint,projected_goal);
    const bool partial=path[count-1]!=gr || dtStatusDetail(status,DT_PARTIAL_RESULT) || dtStatusDetail(status,DT_BUFFER_TOO_SMALL);
    if(partial && dtStatusFailed(h->query->closestPointOnPoly(path[count-1],projected_goal,endpoint,nullptr))) return 6;
    unsigned char flags[1024];dtPolyRef pointrefs[1024];
    dtStatus straight=h->query->findStraightPath(projected_start,endpoint,path,count,points,flags,pointrefs,point_count,1024,0);
    if(dtStatusFailed(straight)) return 6;
    *ref_count=count;for(int i=0;i<count;++i) refs[i]=static_cast<unsigned long long>(path[i]);
    if(dtStatusDetail(status,DT_OUT_OF_NODES)) return 3;
    if(dtStatusDetail(straight,DT_BUFFER_TOO_SMALL)) return 1;
    if(partial) return !h->reference && count==1 ? 2 : 1;
    return 0;
}
// As route_impl, then cut at the first crossing a standing structure seals:
// the corridor keeps the polygons up to it and ends in its entry regions,
// nearest the goal, as a partial route.
// `gated` 0 plans on the static mesh alone (a start inside a structure's
// enclosed room, which no opening leaves).
int mag_nav_route(MagNav* h,const float* start,const float* goal,float radius,float* projected_start,float* projected_goal,
                  float* points,int* point_count,unsigned long long* refs,int* ref_count,int* nodes,int* stages,int gated) {
    const int status=route_impl(h,start,goal,radius,projected_start,projected_goal,points,point_count,refs,ref_count,nodes,stages,gated!=0);
    if(!gated || status==4 || status==5 || status==6 || *ref_count<=0) return status;
    const float* reached = *point_count>0 ? points+3*(*point_count-1) : projected_goal;
    const int cut=gate_violation(h,refs,*ref_count,projected_start,reached);
    if(cut<0) return status;
    const dtPolyRef prev = cut ? static_cast<dtPolyRef>(refs[cut-1]) : 0;
    float endpoint[3];
    if(!gate_endpoint(h,prev,static_cast<dtPolyRef>(refs[cut]),projected_start,projected_goal,endpoint)) return 6;
    std::vector<dtPolyRef> corridor(refs,refs+cut+1);
    // A staged corridor can exceed a direct one; the caller's buffers hold MaxRouteCorridor.
    std::vector<unsigned char> flags(MaxRouteCorridor);std::vector<dtPolyRef> pointrefs(MaxRouteCorridor);
    const dtStatus straight=h->query->findStraightPath(projected_start,endpoint,corridor.data(),static_cast<int>(corridor.size()),
        points,flags.data(),pointrefs.data(),point_count,MaxRouteCorridor,0);
    if(dtStatusFailed(straight)) return 6;
    *ref_count=cut+1;
    return 1;
}
// Polygons whose bounds overlap the box; -1 when more than `cap`.
int mag_nav_polys_in_box(MagNav* h,const float* center,const float* half,unsigned long long* out,int cap) {
    dtQueryFilter f;f.setIncludeFlags(1);
    std::vector<dtPolyRef> refs(cap>0?cap:1);int n=0;
    const dtStatus status=h->query->queryPolygons(center,half,&f,refs.data(),&n,cap);
    if(dtStatusFailed(status)) return -2;
    if(dtStatusDetail(status,DT_BUFFER_TOO_SMALL)) return -1;
    for(int i=0;i<n;++i) out[i]=static_cast<unsigned long long>(refs[i]);
    return n;
}
// A polygon's vertices (up to 6) and its links: neighbour and shared edge
// (left then right endpoint, as Detour orders portals). 0 on failure.
int mag_nav_poly_info(MagNav* h,unsigned long long ref,float* verts,int* vert_count,
                      unsigned long long* links,float* portals,int cap,int* link_count) {
    *vert_count=0;*link_count=0;
    const dtMeshTile* tile=nullptr;const dtPoly* poly=nullptr;
    if(dtStatusFailed(h->mesh->getTileAndPolyByRef(static_cast<dtPolyRef>(ref),&tile,&poly))) return 0;
    for(int i=0;i<poly->vertCount;++i) for(int k=0;k<3;++k) verts[3*i+k]=tile->verts[poly->verts[i]*3+k];
    *vert_count=poly->vertCount;
    for(unsigned int l=poly->firstLink;l!=DT_NULL_LINK;l=tile->links[l].next){
        const dtLink& link=tile->links[l];const dtPolyRef next=link.ref;if(!next) continue;
        if(*link_count>=cap) return 0;
        // The part of edge `link.edge` this link covers (a tile-border link may cover less).
        const float* a=&tile->verts[poly->verts[link.edge]*3];
        const float* b=&tile->verts[poly->verts[(link.edge+1)%poly->vertCount]*3];
        float low=0.f,high=1.f;
        if(link.side!=0xff){low=link.bmin/255.f;high=link.bmax/255.f;}
        float* out=portals+6**link_count;
        for(int k=0;k<3;++k){out[k]=a[k]+(b[k]-a[k])*low;out[3+k]=a[k]+(b[k]-a[k])*high;}
        links[*link_count]=static_cast<unsigned long long>(next);++*link_count;
    }
    return 1;
}
// Detail-mesh height of polygon `ref` at each XZ pair; NaN outside it.
void mag_nav_poly_heights(MagNav* h,unsigned long long ref,const float* xz,int n,float* y) {
    for(int i=0;i<n;++i){
        const float p[3]={xz[2*i],0.f,xz[2*i+1]};float height=0.f;
        y[i]=dtStatusSucceed(h->query->getPolyHeight(static_cast<dtPolyRef>(ref),p,&height))?height:NAN;
    }
}
void mag_nav_clear_gates(MagNav* h) { h->gates.clear(); }
// Install one polygon's structure gate (see StructureGate). 0 on invalid input.
int mag_nav_add_gate(MagNav* h,unsigned long long ref,float ox,float oz,float cell,int nx,int nz,
                     const unsigned char* labels,const unsigned long long* link_refs,
                     const unsigned long long* full,const unsigned long long* through,int link_count) {
    if(nx<1 || nz<1 || nx>512 || nz>512 || !(cell>0.f) || link_count<0 || link_count>64) return 0;
    StructureGate g;g.ox=ox;g.oz=oz;g.cell=cell;g.nx=nx;g.nz=nz;
    g.labels.assign(labels,labels+static_cast<size_t>(nx)*nz);
    for(unsigned char label:g.labels) if(label>64) return 0;
    for(int i=0;i<link_count;++i) g.links[static_cast<dtPolyRef>(link_refs[i])]={full[i],through[i]};
    h->gates[static_cast<dtPolyRef>(ref)]=std::move(g);
    return 1;
}
int mag_nav_component_id(MagNav* h,const float* position) {
    dtQueryFilter f;f.setIncludeFlags(1);const float e[3]={.75f,2.f,.75f};dtPolyRef ref=0;float p[3];
    if(dtStatusFailed(h->query->findNearestPoly(position,e,&f,&ref,p)) || !ref)return -1;
    if(std::hypot(p[0]-position[0],p[2]-position[2])>.15f)return -1;
    return h->component.at(ref);
}
// The nearest point of each walking component among `q`'s hits, in the order
// Detour first reaches each component. More than 128 components keep the 128
// nearest, still in that order.
static void nearest_per_component(const MagNav* h,const GoalPolygons& q,int* components,float* points,int* count) {
    std::map<int,size_t> slot;std::vector<const GoalPolygons::Hit*> nearest;std::vector<int> found;
    for(const GoalPolygons::Hit& hit:q.hits){const int component=h->component.at(hit.ref);
        const auto known=slot.emplace(component,nearest.size());
        if(known.second){nearest.push_back(&hit);found.push_back(component);}
        else if(hit.distance<nearest[known.first->second]->distance)nearest[known.first->second]=&hit;
    }
    std::vector<size_t> keep(nearest.size());for(size_t i=0;i<keep.size();++i)keep[i]=i;
    if(keep.size()>128){
        std::stable_sort(keep.begin(),keep.end(),[&](size_t a,size_t b){return nearest[a]->distance<nearest[b]->distance;});
        keep.resize(128);std::sort(keep.begin(),keep.end());
    }
    for(const size_t i:keep){components[*count]=found[i];rcVcopy(points+3**count,nearest[i]->p);++*count;}
}
// Goal candidates within `radius` of the goal.
int mag_nav_goals(MagNav* h,const float* goal,float radius,int* components,float* points,int* count) {
    *count=0;dtQueryFilter f;f.setIncludeFlags(1);const float e[3]={radius,radius,radius};
    GoalPolygons q(h->query,goal,radius);
    if(dtStatusFailed(h->query->queryPolygons(goal,e,&f,&q)))return 0;
    nearest_per_component(h,q,components,points,count);
    return 1;
}
// Goal candidates in a vertical column: every floor within `radius`
// horizontally and `height` above or below the goal.
int mag_nav_column_goals(MagNav* h,const float* goal,float radius,float height,int* components,float* points,int* count) {
    *count=0;dtQueryFilter f;f.setIncludeFlags(1);const float e[3]={radius,height,radius};
    GoalPolygons q(h->query,goal,radius,height);
    if(dtStatusFailed(h->query->queryPolygons(goal,e,&f,&q)))return 0;
    nearest_per_component(h,q,components,points,count);
    return 1;
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
int mag_nav_contains_settlement_column(MagNav* h,const float* p,
                                         const unsigned long long* refs,int count) {
    dtQueryFilter filter;
    filter.setIncludeFlags(1);
    const float depth=1.5f;
    const float center[3]={p[0],p[1]-depth*.5f,p[2]};
    const float extent[3]={.16f,depth*.5f,.16f};
    dtPolyRef nearby[128];
    int n=0;
    const dtStatus status=h->query->queryPolygons(center,extent,&filter,nearby,&n,128);
    if(dtStatusFailed(status)||dtStatusDetail(status,DT_BUFFER_TOO_SMALL))return 0;
    for(int j=0;j<n;++j) {
        if(std::find(refs,refs+count,static_cast<unsigned long long>(nearby[j]))==refs+count)continue;
        float nearest[3];
        if(dtStatusFailed(h->query->closestPointOnPoly(nearby[j],p,nearest,nullptr)))continue;
        const float below=p[1]-nearest[1];
        if(below>=0.f&&below<=depth&&std::hypot(nearest[0]-p[0],nearest[2]-p[2])<=.15f)return 1;
    }
    return 0;
}
}
