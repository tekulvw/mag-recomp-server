//! Raw C ABI of `src/navmesh_bridge.cpp`, which wraps the unmodified Recast and
//! Detour sources under `vendor/recastnavigation`. `build.rs` compiles both into
//! the static `mag_navmesh` library. Use the safe API in `mag_gameplay::navmesh`.
//!
//! A loaded mesh (`mag_nav_load`) is read-only once loaded and may be shared
//! by threads. Queries take a query context (`mag_nav_query_new`), which one
//! thread uses at a time, and routes take an immutable gate set
//! (`mag_nav_gates_new`, filled by `mag_nav_add_gate` before first use).

use std::ffi::c_int;

unsafe extern "C" {
    pub fn mag_nav_bake(
        verts: *const f32,
        nv: c_int,
        tris: *const c_int,
        nt: c_int,
        min: *const f32,
        max: *const f32,
        out: *mut *mut u8,
        size: *mut c_int,
        mode: c_int,
        slope_degrees: f32,
    ) -> c_int;
    pub fn mag_nav_free_bytes(data: *mut u8);
    pub fn mag_nav_load(
        data: *const u8,
        size: c_int,
        min: *const f32,
        max: *const f32,
        mode: c_int,
    ) -> *mut std::ffi::c_void;
    pub fn mag_nav_destroy(handle: *mut std::ffi::c_void);
    /// A query context over a loaded mesh, which must outlive it. Each
    /// context holds its own search state; use one from one thread at a time.
    pub fn mag_nav_query_new(mesh: *const std::ffi::c_void, node_limit: c_int)
    -> *mut std::ffi::c_void;
    pub fn mag_nav_query_free(query: *mut std::ffi::c_void);
    /// Bytes Detour allocates for one query context of `node_limit` nodes.
    pub fn mag_nav_query_bytes(node_limit: c_int) -> usize;
    pub fn mag_nav_gates_new() -> *mut std::ffi::c_void;
    pub fn mag_nav_gates_free(gates: *mut std::ffi::c_void);
    pub fn mag_nav_tile_stats(
        handle: *const std::ffi::c_void,
        occupied: *mut c_int,
        max_polygons: *mut c_int,
    );
    pub fn mag_nav_route(
        handle: *mut std::ffi::c_void,
        gates: *const std::ffi::c_void,
        start: *const f32,
        goal: *const f32,
        radius: f32,
        ps: *mut f32,
        pg: *mut f32,
        points: *mut f32,
        npoints: *mut c_int,
        refs: *mut u64,
        nrefs: *mut c_int,
        nodes: *mut c_int,
        stages: *mut c_int,
        options: c_int,
    ) -> c_int;
    pub fn mag_nav_polys_in_box(
        handle: *mut std::ffi::c_void,
        center: *const f32,
        half: *const f32,
        out: *mut u64,
        cap: c_int,
    ) -> c_int;
    pub fn mag_nav_poly_info(
        handle: *mut std::ffi::c_void,
        poly: u64,
        verts: *mut f32,
        vert_count: *mut c_int,
        links: *mut u64,
        portals: *mut f32,
        cap: c_int,
        link_count: *mut c_int,
    ) -> c_int;
    pub fn mag_nav_poly_heights(
        handle: *mut std::ffi::c_void,
        poly: u64,
        xz: *const f32,
        count: c_int,
        y: *mut f32,
    );
    #[allow(clippy::too_many_arguments)]
    pub fn mag_nav_add_gate(
        gates: *mut std::ffi::c_void,
        poly: u64,
        ox: f32,
        oz: f32,
        cell: f32,
        nx: c_int,
        nz: c_int,
        labels: *const u8,
        link_refs: *const u64,
        full: *const u64,
        through: *const u64,
        link_count: c_int,
    ) -> c_int;
    pub fn mag_nav_component(
        handle: *mut std::ffi::c_void,
        start: *const f32,
        goal: *const f32,
        closest: *mut f32,
        min: *mut f32,
        max: *mut f32,
        count: *mut c_int,
        total: *mut c_int,
    ) -> c_int;
    pub fn mag_nav_component_id(handle: *mut std::ffi::c_void, position: *const f32) -> c_int;
    pub fn mag_nav_goals(
        handle: *mut std::ffi::c_void,
        goal: *const f32,
        radius: f32,
        components: *mut c_int,
        points: *mut f32,
        count: *mut c_int,
    ) -> c_int;
    pub fn mag_nav_column_goals(
        handle: *mut std::ffi::c_void,
        goal: *const f32,
        radius: f32,
        height: f32,
        components: *mut c_int,
        points: *mut f32,
        count: *mut c_int,
    ) -> c_int;
    pub fn mag_nav_contains(
        handle: *mut std::ffi::c_void,
        p: *const f32,
        refs: *const u64,
        nrefs: c_int,
    ) -> c_int;
    pub fn mag_nav_contains_settlement_column(
        handle: *mut std::ffi::c_void,
        p: *const f32,
        refs: *const u64,
        nrefs: c_int,
    ) -> c_int;
}
