use serde::Deserialize;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Read},
    path::Path,
    time::Duration,
};

const MAX_CATALOG_BYTES: u64 = 65_536;

/// Source identity, mode and qualified provider count for one retail map.
struct MapPins {
    map: &'static str,
    mode: i8,
    level_sha256: &'static str,
    clv_sha256: &'static str,
    qualification_sha256: &'static str,
    providers: usize,
}

/// Maps whose supply placements have a private qualification witness: the
/// three Sabotage maps (mode 0, visible, unfrozen, unsnapped placements) and
/// the Acquisition (mode 1) and Domination (mode 2) maps (every authored
/// placement under an identity parent). Catalogs for any other map, or for a
/// map in another mode, are rejected.
const QUALIFIED_MAPS: [MapPins; 9] = [
    MapPins {
        map: "mag_cosmodrome_final",
        mode: 0,
        level_sha256: "8157f5abc55ddd2ca9eaad3379e87ae909d0b8f7c9b08fa04f1f1e2d8018d96e",
        clv_sha256: "d336caea88351dda5f3204a34e64fdde8eabdf0532b3133e2338cdd150488f4b",
        qualification_sha256: "f24127b06399d1353e4bb1c1a73943278227cd31a1488eda409096099b089b0d",
        providers: 3,
    },
    MapPins {
        map: "mag_kennicott_final",
        mode: 0,
        level_sha256: "399a38f568f4cd4c9c1d787b8ebbe857506b807638dc19f7e2e893ef051332ec",
        clv_sha256: "f17814c8eea43f636541059c8eb910d3707ced53ff9d1cc87d0ad83300825b56",
        qualification_sha256: "39f2839c01b2251b84acac85b11cbcc0e66fae460afde11017abc03ff3840d06",
        providers: 2,
    },
    MapPins {
        map: "mag_remote_comms_final",
        mode: 0,
        level_sha256: "f066f00a74a58d36a531526f4216538300362d5f358a0f40c6b75f2edc226313",
        clv_sha256: "71f5dd11198a5d8d34055276a0fa031f8b8e833f2b8a6c6b9edb9fc4f3a1c888",
        qualification_sha256: "0198100a8c9bae0ae20b9df17a3cfa4b0ee4b3f689a00828abd47a1f744c3a1c",
        providers: 3,
    },
    MapPins {
        map: "mag_alaska_subpen_final",
        mode: 1,
        level_sha256: "fb0e9c12653dcecd9c6f444ec6c5a997aec7f99f481f7951065bf4b64a39f7a0",
        clv_sha256: "b4c2cfe7db94901b98c10a11f1d57a52cfed4941e061196627b3a753a3fe3a7e",
        qualification_sha256: "4b4aedc1ac7efea8ac319783ce52b61ba26c86950e953bdf7323765bcf366d6a",
        providers: 10,
    },
    MapPins {
        map: "mag_panama_military_base_final",
        mode: 1,
        level_sha256: "232e7abf68e89c669bcf6b3335830748975ce4216d3f5ed6b8b3317910e6b1f2",
        clv_sha256: "c054fe57e5fc8b81f2769368ac756cff36eafc8bf071cccc64ff912e330f1c6e",
        qualification_sha256: "9ab8eaf4351a2832534a79f20eca934bdde28276c94125dd579e50fa817b0a7b",
        providers: 9,
    },
    MapPins {
        map: "mag_voz_final",
        mode: 1,
        level_sha256: "e8bf8760097604ef7186583ef9f1208f5d8dc5fddc4f972f30990a79590cead4",
        clv_sha256: "19f7fc0bcf9877633776ca5b1ae4910404499f79315003f5f0671e536c355a65",
        qualification_sha256: "93fba73764fad8c88230d999f5646e1e877fa4a93cce442d94a70e8d3408fde9",
        providers: 11,
    },
    MapPins {
        map: "mag_baku_final",
        mode: 2,
        level_sha256: "e355622ac2b42c91d9aa22c0b46f80cbbc190dc61cc75c710157d08b4ba674e7",
        clv_sha256: "64add78c0f427360d58f82afa09eacf3abe743eb6bb5f57ae236a1c733f39e6f",
        qualification_sha256: "56b3915d59e7d05a31e8f375a48eb65db6aee97977dc60eeadad4f158bc63181",
        providers: 20,
    },
    MapPins {
        map: "mag_canal_final",
        mode: 2,
        level_sha256: "90295495a150c06762284b15de411fca11657557cb228fba8f854ee68ee6059e",
        clv_sha256: "7eff93236d9c5d2f56aadcb406cb7095ea9d69a18d96407e0801343ee859fdce",
        qualification_sha256: "8d85fa6ee3720c69f1dd5f9541bf248d2b5f9d0022ee18b6e04dbf928d757c5a",
        providers: 20,
    },
    MapPins {
        map: "mag_unalaska_final",
        mode: 2,
        level_sha256: "bbd2f66f3664aca428b04f879e9268df121b6d7fb4ca155f3e0b53a7a241b8ed",
        clv_sha256: "bdc4a33b6f3542ac84756f1bd22f6babc30f150aa07dbcc8ccf7e62a94aff0c3",
        qualification_sha256: "5214ed98dd87a769c2c9718630fe67ccbe8d2d54926f00c1fda7ca25264814fd",
        providers: 21,
    },
];

fn map_pins(map: &str) -> Option<&'static MapPins> {
    QUALIFIED_MAPS.iter().find(|pins| pins.map == map)
}

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

pub(crate) fn authored_axes(angles: [f64; 3]) -> [[f64; 3]; 3] {
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
    #[serde(skip)]
    source_sha256: String,
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
        let pins = &QUALIFIED_MAPS[0];
        Self {
            source_sha256: "e".repeat(64),
            schema_version: 1,
            application_id: 20231,
            map: pins.map.into(),
            mode: 0,
            spawn_sha256: "c".repeat(64),
            collision_sha256: "d".repeat(64),
            source: Source {
                archive_sha256: pins.level_sha256.into(),
                clv_sha256: pins.clv_sha256.into(),
                qualification_sha256: pins.qualification_sha256.into(),
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
        let mut catalog: Self = serde_json::from_slice(bytes)?;
        catalog.source_sha256 = crate::navmesh::sha256(bytes);
        if !catalog.valid() {
            return Err(io::Error::other("bot_resupply_catalog_invalid"));
        }
        Ok(catalog)
    }

    fn valid(&self) -> bool {
        let Some(pins) = map_pins(&self.map) else {
            return false;
        };
        let mut names = BTreeSet::new();
        self.schema_version == 1
            && self.application_id == 20231
            && self.mode == pins.mode
            && is_digest(&self.spawn_sha256)
            && is_digest(&self.collision_sha256)
            && self.source.archive_sha256 == pins.level_sha256
            && self.source.clv_sha256 == pins.clv_sha256
            && self.source.qualification_sha256 == pins.qualification_sha256
            && self.providers.len() == pins.providers
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

    pub fn source_sha256(&self) -> &str {
        &self.source_sha256
    }

    pub fn map(&self) -> &str {
        &self.map
    }

    pub fn mode(&self) -> i8 {
        self.mode
    }

    pub fn keys(&self) -> impl Iterator<Item = ProviderKey> + '_ {
        self.providers.iter().map(Provider::key)
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

    const MAP: &str = "mag_cosmodrome_final";

    fn fixture() -> Value {
        fixture_for(&QUALIFIED_MAPS[0])
    }

    fn fixture_for(pins: &MapPins) -> Value {
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
        let providers: Vec<Value> = (0..pins.providers)
            .map(|index| profile(120 + 4 * index as u32, 9 + index as u32))
            .collect();
        json!({
            "schema_version": 1, "application_id": 20231, "map": pins.map, "mode": pins.mode,
            "spawn_sha256": "c".repeat(64), "collision_sha256": "d".repeat(64),
            "source": {"archive_sha256": pins.level_sha256, "clv_sha256": pins.clv_sha256,
                       "qualification_sha256": pins.qualification_sha256},
            "providers": providers,
        })
    }

    fn parses(raw: &Value) -> bool {
        Catalog::from_bytes(&serde_json::to_vec(raw).unwrap()).is_ok()
    }

    #[test]
    fn bot_resupply_catalog_accepts_each_qualified_map_with_its_own_pins() {
        assert_eq!(QUALIFIED_MAPS[0].map, MAP);
        assert_eq!(QUALIFIED_MAPS[0].providers, 3);
        let counts: Vec<_> = QUALIFIED_MAPS
            .iter()
            .map(|pins| (pins.map, pins.mode, pins.providers))
            .collect();
        assert_eq!(
            counts,
            [
                ("mag_cosmodrome_final", 0, 3),
                ("mag_kennicott_final", 0, 2),
                ("mag_remote_comms_final", 0, 3),
                ("mag_alaska_subpen_final", 1, 10),
                ("mag_panama_military_base_final", 1, 9),
                ("mag_voz_final", 1, 11),
                ("mag_baku_final", 2, 20),
                ("mag_canal_final", 2, 20),
                ("mag_unalaska_final", 2, 21),
            ]
        );
        for pins in &QUALIFIED_MAPS {
            let raw = fixture_for(pins);
            assert!(parses(&raw), "{}", pins.map);
            let catalog = Catalog::from_bytes(&serde_json::to_vec(&raw).unwrap()).unwrap();
            assert_eq!(catalog.keys().count(), pins.providers);
            assert_eq!((catalog.map(), catalog.mode()), (pins.map, pins.mode));
            let (spawn, collision) = ("c".repeat(64), "d".repeat(64));
            assert!(
                catalog
                    .validate_binding(20231, pins.map, pins.mode, &spawn, &collision)
                    .is_ok()
            );
            // A map is qualified in exactly one mode.
            for mode in (-1..=3).filter(|mode| *mode != pins.mode) {
                assert!(
                    catalog
                        .validate_binding(20231, pins.map, mode, &spawn, &collision)
                        .is_err()
                );
                let mut moved = raw.clone();
                moved["mode"] = json!(mode);
                assert!(!parses(&moved), "{} mode {mode}", pins.map);
            }
            for other in QUALIFIED_MAPS.iter().filter(|other| other.map != pins.map) {
                assert!(
                    catalog
                        .validate_binding(20231, other.map, other.mode, &spawn, &collision)
                        .is_err()
                );
                // Another map's pins never qualify this map's name.
                let mut renamed = fixture_for(other);
                renamed["map"] = json!(pins.map);
                assert!(!parses(&renamed), "{} as {}", other.map, pins.map);
                // Another map's source identity or witness never qualifies this map.
                for (field, value) in [
                    ("archive_sha256", other.level_sha256),
                    ("clv_sha256", other.clv_sha256),
                    ("qualification_sha256", other.qualification_sha256),
                ] {
                    let mut mixed = raw.clone();
                    mixed["source"][field] = json!(value);
                    assert!(!parses(&mixed), "{} {field}", pins.map);
                }
            }
            // The provider count is exact per map.
            let mut fewer = raw.clone();
            fewer["providers"].as_array_mut().unwrap().pop();
            assert!(!parses(&fewer), "{}", pins.map);
            let mut more = raw.clone();
            let extra = json!({
                "source_offset": 900, "name_token": 99,
                "instance_sha256": "a".repeat(64),
                "definition_offset": 100, "definition_token": 33, "definition_sha256": "b".repeat(64),
                "center": [0.0, 0.0, 0.0], "euler_xyz_degrees": [0.0, 0.0, 0.0],
                "scales_xyz": [1.0, 1.0, 1.0], "axes": [[1.0,0.0,0.0], [0.0,1.0,0.0], [0.0,0.0,1.0]],
                "tick_rate_seconds": 1.0, "tolerance_m": 1.0,
            });
            more["providers"].as_array_mut().unwrap().push(extra);
            assert!(!parses(&more), "{}", pins.map);
        }
    }

    #[test]
    fn bot_resupply_catalog_rejects_unqualified_maps() {
        for map in ["mag_sabotage_unknown_final", "synthetic_map", ""] {
            let mut raw = fixture();
            raw["map"] = json!(map);
            assert!(!parses(&raw), "{map}");
        }
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
            ("name_token", json!(10)),
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
