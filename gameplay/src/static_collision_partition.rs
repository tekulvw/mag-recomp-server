//! Source-bound filter summary of the configured immutable initial compiled and
//! type1-terrain partition. Hashes identify evidence; they do not certify it.
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{io::Read, path::Path};
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    native_binding_sha256: String,
    root_inventory_sha256: String,
    level_archive_sha256: String,
    compiled_catalog_sha256: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RootFilter {
    filter_info: u32,
    root_count: usize,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Document {
    format: String,
    schema_version: u32,
    map_name: String,
    scope: String,
    source: Source,
    root_count: usize,
    root_filters: Vec<RootFilter>,
}
pub struct Partition {
    map: String,
    root_count: usize,
    filters: Vec<u32>,
    fingerprint: String,
}
pub(crate) enum StaticResources {
    Geometry(Box<crate::collision::CollisionWorld>),
    Excluded(std::sync::Arc<Partition>),
}
impl StaticResources {
    pub(crate) fn audit(&self) -> serde_json::Value {
        match self {
            Self::Geometry(g) => {
                serde_json::json!({"kind":"geometry","map":g.metadata().map_name,"sha256":g.source_sha256()})
            }
            Self::Excluded(p) => {
                serde_json::json!({"kind":"root_excluded_initial_partition","map":p.map_name(),"sha256":p.fingerprint(),"roots":p.root_count()})
            }
        }
    }
    pub(crate) fn for_engine<W: std::io::Write>(
        engine: &crate::Engine<W>,
    ) -> Result<Self, &'static str> {
        let map = engine
            .spawn_catalog
            .as_ref()
            .ok_or("spawn_catalog_unavailable")?
            .map_name();
        if let Some(partition) = engine.acquisition_static_partitions.get(map) {
            if partition.map_name() != map {
                return Err("world_cast_map_mismatch");
            }
            return Ok(Self::Excluded(std::sync::Arc::clone(partition)));
        }
        let geometry = engine
            .collision_world
            .clone()
            .ok_or("world_cast_static_geometry_unavailable")?;
        if geometry.metadata().map_name != map {
            return Err("world_cast_map_mismatch");
        }
        Ok(Self::Geometry(Box::new(geometry)))
    }
    pub(crate) fn world<'a>(
        &'a self,
        snapshot: &'a crate::dynamic_collision::Snapshot,
        rows: [u32; 32],
        settings: crate::acquisition_world::Settings,
    ) -> Result<crate::acquisition_world::World<'a>, &'static str> {
        use crate::acquisition_world::{Filters, World};
        match self {
            Self::Geometry(geometry) => World::new(
                snapshot,
                geometry,
                Filters::owned_entities(snapshot, geometry, rows)?,
                settings,
            ),
            Self::Excluded(partition) => World::with_excluded_statics(
                snapshot,
                partition,
                Filters::owned_entities_for_partition(snapshot, partition, rows)?,
                settings,
            ),
        }
    }
}
impl Partition {
    pub fn validate_collection(partitions: &[Self], base_map: &str) -> Result<(), &'static str> {
        if partitions.is_empty() || partitions.len() > 64 {
            return Err("invalid_static_partition_collection");
        }
        let mut maps = std::collections::BTreeSet::new();
        for partition in partitions {
            if !maps.insert(partition.map_name()) {
                return Err("duplicate_static_partition_map");
            }
        }
        if !maps.contains(base_map) {
            return Err("static_partition_base_map_missing");
        }
        Ok(())
    }
    pub fn load(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let path = path.as_ref();
        let info = std::fs::metadata(path)?;
        if !info.is_file() || info.len() > 65_536 {
            return Err(std::io::Error::other("invalid_static_partition_file"));
        }
        let mut text = String::new();
        std::fs::File::open(path)?
            .take(65_537)
            .read_to_string(&mut text)?;
        Self::from_json(&text).map_err(std::io::Error::other)
    }
    pub fn from_json(text: &str) -> Result<Self, &'static str> {
        if text.len() > 65_536 {
            return Err("static_partition_too_large");
        }
        let d: Document = serde_json::from_str(text).map_err(|_| "invalid_static_partition")?;
        if d.format != "mag_static_filter_partition"
            || d.schema_version != 1
            || d.scope != "initial_compiled_collision_and_type1_terrain"
            || d.map_name.trim().is_empty()
            || d.map_name.len() > 128
            || !(1..=1_000_000).contains(&d.root_count)
            || d.root_filters.is_empty()
            || d.root_filters.len() > 32
        {
            return Err("invalid_static_partition");
        }
        for h in [
            &d.source.native_binding_sha256,
            &d.source.root_inventory_sha256,
            &d.source.level_archive_sha256,
            &d.source.compiled_catalog_sha256,
        ] {
            if h.len() != 64 || !h.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("invalid_static_partition_source");
            }
        }
        let mut filters = std::collections::BTreeSet::new();
        let mut total = 0usize;
        for root in d.root_filters {
            if root.filter_info > 31 || root.root_count == 0 || !filters.insert(root.filter_info) {
                return Err("invalid_static_partition_root_filters");
            }
            total = total
                .checked_add(root.root_count)
                .ok_or("static_partition_count_overflow")?;
        }
        if total != d.root_count {
            return Err("static_partition_count_mismatch");
        }
        Ok(Self {
            map: d.map_name,
            root_count: d.root_count,
            filters: filters.into_iter().collect(),
            fingerprint: format!("{:x}", Sha256::digest(text.as_bytes())),
        })
    }
    pub fn map_name(&self) -> &str {
        &self.map
    }
    pub fn root_count(&self) -> usize {
        self.root_count
    }
    pub fn fingerprint(&self) -> &str {
        &self.fingerprint
    }
    /// Reevaluate every full filter against current authority for each query.
    /// Filter-enabled roots require real geometry/child data, never no-hit.
    pub fn require_excluded(
        &self,
        query_filter: u32,
        rows: &[u32; 32],
    ) -> Result<(), &'static str> {
        if self
            .filters
            .iter()
            .any(|&filter| crate::acquisition_world::Filters::allows(rows, query_filter, filter))
        {
            return Err("static_partition_not_excluded");
        }
        Ok(())
    }
}
impl<W: std::io::Write> crate::Engine<W> {
    /// Trusted extraction inputs for immutable configured initial partitions.
    /// Other entity-created bodies remain owned by the dynamic authority.
    pub fn with_acquisition_static_partitions(
        mut self,
        partitions: Vec<Partition>,
    ) -> Result<Self, &'static str> {
        if self.active.is_some()
            || !self.peers.is_empty()
            || self.wire_profile.application_id() != 20231
            || self.match_profile().game_type != 1
            || partitions.is_empty()
            || partitions.len() > 64
        {
            return Err("static_partitions_require_idle_retail_acquisition");
        }
        Partition::validate_collection(
            &partitions,
            self.spawn_catalog
                .as_ref()
                .ok_or("spawn_catalog_unavailable")?
                .map_name(),
        )?;
        let mut loaded = std::collections::BTreeMap::new();
        for partition in partitions {
            if loaded
                .insert(
                    partition.map_name().to_owned(),
                    std::sync::Arc::new(partition),
                )
                .is_some()
            {
                return Err("duplicate_static_partition_map");
            }
        }
        if !self
            .spawn_catalog
            .as_ref()
            .is_some_and(|s| loaded.contains_key(s.map_name()))
        {
            return Err("static_partition_base_map_missing");
        }
        self.acquisition_static_partitions = loaded;
        Ok(self)
    }
}
