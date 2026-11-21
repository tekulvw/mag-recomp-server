use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::Path,
    time::Duration,
};

const MAP: &str = "mag_cosmodrome_final";
const LEVEL_SHA: &str = "8157f5abc55ddd2ca9eaad3379e87ae909d0b8f7c9b08fa04f1f1e2d8018d96e";
const CLV_SHA: &str = "d336caea88351dda5f3204a34e64fdde8eabdf0532b3133e2338cdd150488f4b";
const QUALIFICATION_SHA: &str = "f24127b06399d1353e4bb1c1a73943278227cd31a1488eda409096099b089b0d";
const MAX_CATALOG_BYTES: u64 = 65_536;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProviderKey {
    pub source_offset: u32,
    pub name_token: u32,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    archive_sha256: String,
    clv_sha256: String,
    qualification_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Provider {
    source_offset: u32,
    name_token: u32,
    instance_sha256: String,
    definition_offset: u32,
    definition_token: u32,
    definition_sha256: String,
    center: [f64; 3],
    euler_xyz_degrees: [f64; 3],
    scales_xyz: [f64; 3],
    axes: [[f64; 3]; 3],
    tick_rate_seconds: f32,
    tolerance_m: f64,
}

impl Provider {
    fn key(&self) -> ProviderKey {
        ProviderKey {
            source_offset: self.source_offset,
            name_token: self.name_token,
        }
    }

    pub fn tick_rate_seconds(&self) -> f32 {
        self.tick_rate_seconds
    }

    pub fn contains(&self, anchor: [f64; 3]) -> bool {
        let offset: [f64; 3] = std::array::from_fn(|lane| anchor[lane] - self.center[lane]);
        self.axes.iter().zip(self.scales_xyz).all(|(axis, scale)| {
            let along_axis: f64 = axis.iter().zip(offset).map(|(a, b)| a * b).sum();
            along_axis.abs() <= scale * 0.5 + self.tolerance_m
        })
    }

    fn valid(&self) -> bool {
        self.source_offset > 0
            && self.name_token > 0
            && self.definition_offset > 0
            && self.definition_token > 0
            && is_digest(&self.instance_sha256)
            && is_digest(&self.definition_sha256)
            && self.tolerance_m.is_finite()
            && self.tolerance_m >= 0.0
            && self.center.iter().all(|value| value.is_finite())
            && self
                .scales_xyz
                .iter()
                .all(|value| value.is_finite() && *value > 0.0)
            && self.euler_xyz_degrees.iter().all(|value| value.is_finite())
            && Duration::try_from_secs_f32(self.tick_rate_seconds)
                .is_ok_and(|duration| !duration.is_zero())
            && self
                .axes
                .iter()
                .flatten()
                .zip(authored_axes(self.euler_xyz_degrees).iter().flatten())
                .all(|(axis, expected)| (axis - expected).abs() <= 1e-10)
    }
}

fn authored_axes(angles: [f64; 3]) -> [[f64; 3]; 3] {
    let [(sx, cx), (sy, cy), (sz, cz)] = angles.map(|angle| angle.to_radians().sin_cos());
    [
        [cz * cy, sz * cy, -sy],
        [cz * sy * sx - sz * cx, sz * sy * sx + cz * cx, cy * sx],
        [cz * sy * cx + sz * sx, sz * sy * cx - cz * sx, cy * cx],
    ]
}

fn is_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Catalog {
    schema_version: u32,
    application_id: u32,
    map: String,
    mode: i8,
    spawn_sha256: String,
    collision_sha256: String,
    source: Source,
    providers: Vec<Provider>,
}

impl Catalog {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = Vec::new();
        fs::File::open(path)?
            .take(MAX_CATALOG_BYTES + 1)
            .read_to_end(&mut bytes)?;
        Self::from_bytes(&bytes)
    }

    #[cfg(test)]
    pub(crate) fn synthetic(anchor: [f64; 3], tick_rate_seconds: f32) -> Self {
        Self {
            schema_version: 1,
            application_id: 20231,
            map: MAP.into(),
            mode: 0,
            spawn_sha256: "c".repeat(64),
            collision_sha256: "d".repeat(64),
            source: Source {
                archive_sha256: LEVEL_SHA.into(),
                clv_sha256: CLV_SHA.into(),
                qualification_sha256: QUALIFICATION_SHA.into(),
            },
            providers: vec![Provider {
                source_offset: 120,
                name_token: 9,
                instance_sha256: "a".repeat(64),
                definition_offset: 100,
                definition_token: 33,
                definition_sha256: "b".repeat(64),
                center: anchor,
                euler_xyz_degrees: [0.0; 3],
                scales_xyz: [2.0; 3],
                axes: authored_axes([0.0; 3]),
                tick_rate_seconds,
                tolerance_m: 1.0,
            }],
        }
    }

    fn from_bytes(bytes: &[u8]) -> io::Result<Self> {
        if bytes.len() as u64 > MAX_CATALOG_BYTES {
            return Err(io::Error::other("bot_resupply_catalog_too_large"));
        }
        let catalog: Self = serde_json::from_slice(bytes)?;
        if !catalog.valid() {
            return Err(io::Error::other("bot_resupply_catalog_invalid"));
        }
        Ok(catalog)
    }

    fn valid(&self) -> bool {
        let mut names = BTreeSet::new();
        self.schema_version == 1
            && self.application_id == 20231
            && self.map == MAP
            && self.mode == 0
            && is_digest(&self.spawn_sha256)
            && is_digest(&self.collision_sha256)
            && self.source.archive_sha256 == LEVEL_SHA
            && self.source.clv_sha256 == CLV_SHA
            && self.source.qualification_sha256 == QUALIFICATION_SHA
            && self.providers.len() == 3
            && self
                .providers
                .windows(2)
                .all(|rows| rows[0].source_offset < rows[1].source_offset)
            && self
                .providers
                .iter()
                .all(|provider| provider.valid() && names.insert(provider.name_token))
    }

    pub fn validate_binding(
        &self,
        application_id: u32,
        map: &str,
        mode: i8,
        spawn_sha: &str,
        collision_sha: &str,
    ) -> Result<(), &'static str> {
        if self.application_id != application_id
            || self.map != map
            || self.mode != mode
            || self.spawn_sha256 != spawn_sha
            || self.collision_sha256 != collision_sha
        {
            return Err("bot_resupply_catalog_binding_mismatch");
        }
        Ok(())
    }

    pub fn select(&self, anchor: [f64; 3]) -> Option<ProviderKey> {
        self.providers
            .iter()
            .find(|provider| provider.contains(anchor))
            .map(Provider::key)
    }

    pub fn provider(&self, key: ProviderKey) -> Option<&Provider> {
        self.providers.iter().find(|provider| provider.key() == key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    fn fixture() -> Value {
        let profile = |source_offset, name_token| {
            json!({
                "source_offset": source_offset, "name_token": name_token,
                "instance_sha256": "a".repeat(64),
                "definition_offset": 100, "definition_token": 33, "definition_sha256": "b".repeat(64),
                "center": [4.0, 5.0, 6.0], "euler_xyz_degrees": [0.0, 90.0, 0.0],
                "scales_xyz": [6.0, 4.0, 2.0], "axes": [[0.0,0.0,-1.0], [0.0,1.0,0.0], [1.0,0.0,0.0]],
                "tick_rate_seconds": 2.5, "tolerance_m": 2.0,
            })
        };
        json!({
            "schema_version": 1, "application_id": 20231, "map": MAP, "mode": 0,
            "spawn_sha256": "c".repeat(64), "collision_sha256": "d".repeat(64),
            "source": {"archive_sha256": LEVEL_SHA, "clv_sha256": CLV_SHA,
                       "qualification_sha256": QUALIFICATION_SHA},
            "providers": [profile(120, 9), profile(124, 8), profile(130, 7)],
        })
    }

    #[test]
    fn bot_resupply_catalog_binds_context_and_selects_inclusive_authored_box() {
        let catalog = Catalog::from_bytes(&serde_json::to_vec(&fixture()).unwrap()).unwrap();
        let spawn = "c".repeat(64);
        let collision = "d".repeat(64);
        assert!(
            catalog
                .validate_binding(20231, MAP, 0, &spawn, &collision)
                .is_ok()
        );
        for (application_id, map, mode, spawn_sha, collision_sha) in [
            (20230, MAP, 0, spawn.as_str(), collision.as_str()),
            (20231, "elsewhere", 0, spawn.as_str(), collision.as_str()),
            (20231, MAP, 1, spawn.as_str(), collision.as_str()),
            (20231, MAP, 0, "", collision.as_str()),
            (20231, MAP, 0, spawn.as_str(), ""),
        ] {
            assert!(
                catalog
                    .validate_binding(application_id, map, mode, spawn_sha, collision_sha)
                    .is_err()
            );
        }
        let key = catalog.select([7.0, 9.0, 1.0]).unwrap();
        assert_eq!(key.source_offset, 120);
        assert_eq!(key.name_token, 9);
        assert_eq!(catalog.provider(key).unwrap().tick_rate_seconds(), 2.5);
        assert!(catalog.select([7.001, 9.0, 1.0]).is_none());
        assert!(catalog.select([f64::NAN, 5.0, 6.0]).is_none());
    }

    #[test]
    fn bot_resupply_catalog_rejects_ambiguous_or_malformed_source() {
        for (field, value) in [
            ("name_token", json!(8)),
            ("source_offset", json!(124)),
            ("scales_xyz", json!([6, 0, 2])),
            ("axes", json!([[1, 0, 0], [0, 1, 0], [0, 0, 1]])),
            ("tick_rate_seconds", json!(0.0)),
            ("tolerance_m", json!(-1.0)),
            ("tolerance_m", Value::Null),
        ] {
            let mut raw = fixture();
            raw["providers"][0][field] = value;
            assert!(
                Catalog::from_bytes(&serde_json::to_vec(&raw).unwrap()).is_err(),
                "{field}"
            );
        }
        let mut raw = fixture();
        raw["source"]["clv_sha256"] = json!("f".repeat(64));
        assert!(Catalog::from_bytes(&serde_json::to_vec(&raw).unwrap()).is_err());
    }
}
