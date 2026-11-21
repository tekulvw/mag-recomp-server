use crate::{
    bot_resupply_catalog::{Catalog, ProviderKey},
    collision::CollisionWorld,
    navmesh::NavMesh,
};
use serde::Deserialize;
use std::{
    fs,
    io::{self, Read},
    path::Path,
    time::Duration,
};

type Point = [f64; 3];
const MAX_POLICY_BYTES: u64 = 65_536;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Candidates {
    source_offset: u32,
    name_token: u32,
    anchors: Vec<Point>,
}

impl Candidates {
    fn key(&self) -> ProviderKey {
        ProviderKey {
            source_offset: self.source_offset,
            name_token: self.name_token,
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    schema_version: u32,
    application_id: u32,
    map: String,
    mode: i8,
    provider_catalog_sha256: String,
    collision_sha256: String,
    detour_sha256: String,
    #[serde(default)]
    allowed_drop_id: Option<u32>,
    #[serde(default)]
    planning_window_ms: Option<u32>,
    query_budget_per_tick: u8,
    retry_ms: u32,
    goal_radius_m: f64,
    max_route_length_m: f64,
    seek_max_duration_ms: u32,
    providers: Vec<Candidates>,
}

impl Policy {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_POLICY_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() as u64 > MAX_POLICY_BYTES {
            return Err(io::Error::other("bot_resupply_approach_policy_too_large"));
        }
        let value: Self = serde_json::from_slice(bytes)?;
        if !value.valid() {
            return Err(io::Error::other("bot_resupply_approach_policy_invalid"));
        }
        Ok(value)
    }

    fn valid(&self) -> bool {
        self.schema_version == 1
            && self.application_id == 20231
            && self.mode == 0
            && !self.map.is_empty()
            && [
                &self.provider_catalog_sha256,
                &self.collision_sha256,
                &self.detour_sha256,
            ]
            .into_iter()
            .all(|sha| {
                sha.len() == 64
                    && sha
                        .bytes()
                        .all(|x| x.is_ascii_digit() || (b'a'..=b'f').contains(&x))
            })
            && match (self.allowed_drop_id, self.planning_window_ms) {
                (None, None) => true,
                (Some(id), Some(window)) => {
                    id > 0
                        && (1_000..=10_000).contains(&window)
                        && window <= self.seek_max_duration_ms
                        && self.retry_ms < window
                }
                _ => false,
            }
            && (1..=16).contains(&self.query_budget_per_tick)
            && self.retry_ms > 0
            && (0.05..=5.0).contains(&self.goal_radius_m)
            && self.max_route_length_m.is_finite()
            && self.max_route_length_m > 0.0
            && self.max_route_length_m <= 192.0
            && (1_000..=120_000).contains(&self.seek_max_duration_ms)
            && (1..=16).contains(&self.providers.len())
            && self
                .providers
                .windows(2)
                .all(|pair| pair[0].source_offset < pair[1].source_offset)
            && self.providers.iter().all(|row| {
                row.source_offset > 0
                    && row.name_token > 0
                    && (1..=16).contains(&row.anchors.len())
                    && row.anchors.iter().enumerate().all(|(index, anchor)| {
                        anchor
                            .iter()
                            .all(|coord| coord.is_finite() && coord.abs() <= 100_000.0)
                            && !row.anchors[..index].contains(anchor)
                    })
            })
    }

    pub fn validate_binding(
        &self,
        catalog: &Catalog,
        mesh: &NavMesh,
        world: &CollisionWorld,
    ) -> Result<(), &'static str> {
        if !self.valid() {
            return Err("bot_resupply_approach_policy_invalid");
        }
        let artifact = mesh.artifact();
        if self.map != artifact.map_name
            || self.map != world.metadata().map_name
            || self.collision_sha256 != world.source_sha256()
            || self.collision_sha256 != artifact.world_sha256
            || self.detour_sha256 != artifact.detour_sha256
            || mesh.validate_coverage(world).is_err()
        {
            return Err("bot_resupply_approach_world_binding_mismatch");
        }
        if self
            .allowed_drop_id
            .is_some_and(|id| !artifact.drop_links.iter().any(|link| link.id == id))
        {
            return Err("bot_resupply_approach_unknown_drop");
        }
        self.validate_providers(catalog)
    }

    pub fn allowed_drop_id(&self) -> Option<u32> {
        self.allowed_drop_id
    }

    pub fn planning_window(&self) -> Option<Duration> {
        self.planning_window_ms
            .map(|ms| Duration::from_millis(u64::from(ms)))
    }

    pub fn validate_drop_context(
        &self,
        autonomous: bool,
        has_drop_policy: bool,
        has_fall_rules: bool,
    ) -> Result<(), &'static str> {
        if self.allowed_drop_id.is_none() {
            return Ok(());
        }
        if !autonomous {
            return Err("bot_resupply_drop_requires_autonomous_population");
        }
        if !has_drop_policy {
            return Err("bot_resupply_drop_requires_drop_policy");
        }
        if !has_fall_rules {
            return Err("bot_resupply_drop_requires_fall_rules");
        }
        Ok(())
    }

    fn validate_providers(&self, catalog: &Catalog) -> Result<(), &'static str> {
        if self.provider_catalog_sha256 != catalog.source_sha256()
            || !self.providers().eq(catalog.keys())
        {
            return Err("bot_resupply_approach_catalog_binding_mismatch");
        }
        for row in &self.providers {
            let provider = catalog
                .provider(row.key())
                .ok_or("bot_resupply_approach_catalog_binding_mismatch")?;
            if !row.anchors.iter().all(|anchor| provider.contains(*anchor)) {
                return Err("bot_resupply_approach_anchor_outside_provider");
            }
        }
        Ok(())
    }

    pub fn providers(&self) -> impl Iterator<Item = ProviderKey> + '_ {
        self.providers.iter().map(Candidates::key)
    }

    pub fn candidates(&self, key: ProviderKey) -> &[Point] {
        self.providers
            .iter()
            .find(|row| row.key() == key)
            .map_or(&[], |row| row.anchors.as_slice())
    }

    pub fn query_budget_per_tick(&self) -> usize {
        usize::from(self.query_budget_per_tick)
    }

    pub fn retry(&self) -> Duration {
        Duration::from_millis(u64::from(self.retry_ms))
    }

    pub fn max_duration(&self) -> Duration {
        Duration::from_millis(u64::from(self.seek_max_duration_ms))
    }

    pub fn goal_radius_m(&self) -> f64 {
        self.goal_radius_m
    }

    pub fn max_route_length_m(&self) -> f64 {
        self.max_route_length_m
    }

    #[cfg(test)]
    pub(crate) fn with_synthetic_drop(mut self, id: u32) -> Self {
        self.allowed_drop_id = Some(id);
        self.planning_window_ms = Some(2_000);
        self
    }

    #[cfg(test)]
    pub(crate) fn synthetic(key: ProviderKey, anchor: Point) -> Self {
        Self {
            schema_version: 1,
            application_id: 20231,
            map: "synthetic_arena".into(),
            mode: 0,
            provider_catalog_sha256: "e".repeat(64),
            collision_sha256: "d".repeat(64),
            detour_sha256: "a".repeat(64),
            allowed_drop_id: None,
            planning_window_ms: None,
            query_budget_per_tick: 1,
            retry_ms: 500,
            goal_radius_m: 0.75,
            max_route_length_m: 15.0,
            seek_max_duration_ms: 8_000,
            providers: vec![Candidates {
                source_offset: key.source_offset,
                name_token: key.name_token,
                anchors: vec![anchor],
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn bot_resupply_approach_policy_requires_bounded_explicit_fields() {
        let mut input = json!({
            "schema_version": 1, "application_id": 20231, "map": "synthetic_map", "mode": 0,
            "provider_catalog_sha256": "e".repeat(64), "collision_sha256": "d".repeat(64),
            "detour_sha256": "a".repeat(64), "query_budget_per_tick": 2, "retry_ms": 75,
            "goal_radius_m": 0.5, "max_route_length_m": 39, "seek_max_duration_ms": 1_900,
            "providers": [{"source_offset": 42, "name_token": 5, "anchors": [[1,2,3],[2,2,3]]}],
        });
        let load =
            |input: &serde_json::Value| Policy::from_bytes(&serde_json::to_vec(input).unwrap());
        let policy = load(&input).unwrap();
        assert_eq!(policy.query_budget_per_tick(), 2);
        assert_eq!(policy.max_duration(), Duration::from_millis(1_900));
        input["query_budget_per_tick"] = json!(17);
        assert!(load(&input).is_err());
        input["query_budget_per_tick"] = json!(1);
        input["providers"][0]["anchors"][1] = json!([1, 2, 3]);
        assert!(load(&input).is_err());
    }

    #[test]
    fn bot_resupply_approach_policy_accepts_only_one_positive_optional_drop() {
        let mut input = json!({
            "schema_version":1,"application_id":20231,"map":"synthetic_map","mode":0,
            "provider_catalog_sha256":"e".repeat(64),"collision_sha256":"d".repeat(64),
            "detour_sha256":"a".repeat(64),"query_budget_per_tick":1,"retry_ms":1000,
            "goal_radius_m":0.5,"max_route_length_m":160,"seek_max_duration_ms":90000,
            "providers":[{"source_offset":42,"name_token":5,"anchors":[[1,2,3]]}],
        });
        let load =
            |input: &serde_json::Value| Policy::from_bytes(&serde_json::to_vec(input).unwrap());
        let walking = load(&input).unwrap();
        assert_eq!(walking.allowed_drop_id(), None);
        assert_eq!(walking.validate_drop_context(false, false, false), Ok(()));
        input["allowed_drop_id"] = json!(57);
        assert!(
            load(&input).is_err(),
            "drop opt-in requires a bounded planning window"
        );
        input["planning_window_ms"] = json!(2000);
        let enabled = load(&input).expect("an explicitly selected single drop is supported");
        assert_eq!(enabled.planning_window(), Some(Duration::from_millis(2000)));
        assert_eq!(enabled.allowed_drop_id(), Some(57));
        assert!(enabled.validate_drop_context(false, true, true).is_err());
        assert!(enabled.validate_drop_context(true, false, true).is_err());
        assert!(enabled.validate_drop_context(true, true, false).is_err());
        assert_eq!(enabled.validate_drop_context(true, true, true), Ok(()));
        for window in [json!(999), json!(10001), json!(true), json!("2000")] {
            input["planning_window_ms"] = window;
            assert!(load(&input).is_err());
        }
        input["planning_window_ms"] = json!(2000);
        input["retry_ms"] = json!(2000);
        assert!(load(&input).is_err());
        input["retry_ms"] = json!(1000);
        input["seek_max_duration_ms"] = json!(1000);
        assert!(load(&input).is_err());
        input["seek_max_duration_ms"] = json!(90000);
        for value in [
            json!(0),
            json!(-1),
            json!(true),
            json!("57"),
            json!(4294967296_u64),
            json!([57, 58]),
        ] {
            input["allowed_drop_id"] = value;
            assert!(load(&input).is_err());
        }
    }

    #[test]
    fn bot_resupply_approach_policy_binds_exact_provider_and_contains_anchors() {
        let catalog = Catalog::synthetic([1., 2., 3.], 1.);
        let key = catalog.keys().next().unwrap();
        let mut policy = Policy::synthetic(key, [3., 2., 3.]);
        assert!(policy.validate_providers(&catalog).is_ok());
        policy.providers[0].anchors[0][0] += 0.01;
        assert_eq!(
            policy.validate_providers(&catalog),
            Err("bot_resupply_approach_anchor_outside_provider")
        );
        policy.providers[0].name_token += 1;
        assert_eq!(
            policy.validate_providers(&catalog),
            Err("bot_resupply_approach_catalog_binding_mismatch")
        );
    }
}
