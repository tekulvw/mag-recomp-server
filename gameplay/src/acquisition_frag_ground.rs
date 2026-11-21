//! Source-bound retail Ground frag damage areas and overlap roots.
use crate::{
    acquisition_volume_damage::{Target, TargetKey, TargetKind},
    dynamic_collision::{self, Geometry, Pose},
    dynamic_collision_runtime::RuntimeBindings,
    ground_vehicles::{self, Creation},
};
use serde::Deserialize;
use std::collections::BTreeSet;

const ROLES: [u8; 3] = [3, 7, 9];

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
pub(crate) enum Scope {
    #[serde(rename = "mag_acquisition_frag_ground_areas_v1")]
    Acquisition,
    #[serde(rename = "mag_domination_frag_ground_areas_v1")]
    Domination,
}

impl Scope {
    fn roles(self) -> &'static [u8] {
        match self {
            Self::Acquisition => &ROLES,
            Self::Domination => &[3, 7],
        }
    }

    fn placement_counts(self) -> [usize; 3] {
        match self {
            Self::Acquisition => [4, 8, 4],
            Self::Domination => [8, 8, 0],
        }
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    schema: Scope,
    source: Source,
    variants: Vec<Variant>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    retail_main_sha256: String,
    dynamic_catalog_sha256: String,
    bounds_proof_sha256: String,
    wheel_overlap_proof_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    spawn_type: u8,
    vehicle_type_token: u32,
    faction: u8,
    template: String,
    packfile_sha256: String,
    rear_threshold_z: f32,
    front_threshold_z: f32,
    chassis_root: String,
    wheel_root: String,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Catalog {
    pub(crate) fn from_bytes(raw: &[u8]) -> Result<Self, &'static str> {
        Self::from_bytes_for(raw, Scope::Acquisition)
    }

    pub(crate) fn from_bytes_for(raw: &[u8], scope: Scope) -> Result<Self, &'static str> {
        if raw.len() > 16 * 1024 {
            return Err("acquisition_frag_ground_areas_limit");
        }
        let mut catalog: Self =
            serde_json::from_slice(raw).map_err(|_| "invalid_acquisition_frag_ground_areas")?;
        catalog
            .variants
            .sort_by_key(|row| (row.spawn_type, row.faction));
        let source = &catalog.source;
        let mut templates = BTreeSet::new();
        if catalog.schema != scope
            || ![
                &source.retail_elf_sha256,
                &source.retail_main_sha256,
                &source.dynamic_catalog_sha256,
                &source.bounds_proof_sha256,
                &source.wheel_overlap_proof_sha256,
            ]
            .into_iter()
            .all(|value| digest(value))
            || !catalog
                .variants
                .iter()
                .map(|row| (row.spawn_type, row.faction))
                .eq(scope
                    .roles()
                    .iter()
                    .copied()
                    .flat_map(|role| (0..3).map(move |faction| (role, faction))))
            || catalog.variants.iter().any(|row| {
                row.vehicle_type_token == 0
                    || !templates.insert(row.template.as_str())
                    || row.template.is_empty()
                    || row.template.len() > 128
                    || !row
                        .template
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
                    || !digest(&row.packfile_sha256)
                    || !(-1000. ..=1000.).contains(&row.rear_threshold_z)
                    || !(-1000. ..=1000.).contains(&row.front_threshold_z)
                    || row.rear_threshold_z >= row.front_threshold_z
                    || row.chassis_root != "chassis"
                    || row.wheel_root != "wheel"
            })
        {
            return Err("invalid_acquisition_frag_ground_areas");
        }
        let mut tokens = BTreeSet::new();
        for rows in catalog.variants.as_chunks::<3>().0 {
            let token = rows[0].vehicle_type_token;
            if !tokens.insert(token) || rows.iter().any(|row| row.vehicle_type_token != token) {
                return Err("invalid_acquisition_frag_ground_role");
            }
        }
        Ok(catalog)
    }

    pub(crate) fn validate_main(&self, main: &str) -> Result<(), &'static str> {
        if self.source.retail_main_sha256 != main {
            return Err("acquisition_frag_ground_area_source_mismatch");
        }
        Ok(())
    }

    pub(crate) fn validate(
        &self,
        native_elf: &str,
        ground: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        if self.source.retail_elf_sha256 != native_elf {
            return Err("acquisition_frag_ground_area_source_mismatch");
        }
        let mut counts = [0; 3];
        for creation in ground.creations() {
            let source = ground
                .vehicle_source(creation.index)
                .ok_or("acquisition_frag_ground_area_role_unbound")?;
            let row = self.variant(creation)?;
            if source.spawn_type != row.spawn_type
                || source.vehicle_type_token != row.vehicle_type_token
            {
                return Err("acquisition_frag_ground_area_role_unbound");
            }
            let role = ROLES
                .iter()
                .position(|&role| role == row.spawn_type)
                .ok_or("acquisition_frag_ground_area_role_unbound")?;
            counts[role] += 1;
        }
        if counts != self.schema.placement_counts() {
            return Err("acquisition_frag_ground_area_role_unbound");
        }
        Ok(())
    }

    pub(crate) fn validate_dynamic_parts(
        &self,
        catalog: &dynamic_collision::Catalog,
        bindings: &RuntimeBindings,
        ground: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        if self.source.dynamic_catalog_sha256 != catalog.fingerprint() {
            return Err("acquisition_frag_ground_dynamic_source_mismatch");
        }
        for creation in ground.creations() {
            self.variant(creation)?;
        }
        for row in &self.variants {
            let mut matching = bindings.vehicles.iter().filter(|source| {
                source.vehicle_type_token == row.vehicle_type_token && source.faction == row.faction
            });
            if matching
                .next()
                .is_none_or(|source| source.template != row.template)
                || matching.next().is_some()
            {
                return Err("acquisition_frag_ground_dynamic_template_mismatch");
            }
            let target = Target {
                key: TargetKey {
                    id: 1,
                    generation: 1,
                },
                kind: TargetKind::GroundVehicle,
                team: 0,
            };
            let template = catalog.instantiate(&row.template, target, Pose::at([0.; 3]), false)?;
            if template.bodies.len() != 2
                || !template.bodies.iter().any(|root| {
                    root.root == row.chassis_root
                        && root.layer == 22
                        && matches!(*root.shape, Geometry::Hull { .. })
                })
                || !template.bodies.iter().any(|root| {
                    root.root == row.wheel_root
                        && root.layer == 22
                        && matches!(*root.shape, Geometry::Box { .. })
                })
            {
                return Err("acquisition_frag_ground_dynamic_roots_mismatch");
            }
        }
        Ok(())
    }

    fn variant(&self, creation: &Creation) -> Result<&Variant, &'static str> {
        self.variants
            .iter()
            .find(|row| {
                row.vehicle_type_token == creation.vehicle_type_token
                    && row.faction == creation.faction
            })
            .ok_or("acquisition_frag_ground_variant_unbound")
    }

    pub(crate) fn accepts_root(
        &self,
        creation: &Creation,
        root: &str,
    ) -> Result<bool, &'static str> {
        let variant = self.variant(creation)?;
        Ok(root == variant.chassis_root || root == variant.wheel_root)
    }

    pub(crate) fn location(
        &self,
        creation: &Creation,
        pose: Pose,
        origin: [f64; 3],
    ) -> Result<u8, &'static str> {
        pose.validate()?;
        if !origin
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= 100_000.)
        {
            return Err("invalid_acquisition_frag_ground_origin");
        }
        let row = self.variant(creation)?;
        Ok(crate::hospital_rockets::vehicle_blast_location(
            pose,
            origin,
            row.rear_threshold_z,
            row.front_threshold_z,
        ))
    }

    #[cfg(test)]
    pub(crate) fn synthetic_domination(apc_token: u32, truck_token: u32) -> Self {
        Self::from_bytes_for(
            &serde_json::to_vec(&tests::fixture_for(
                Scope::Domination,
                [apc_token, truck_token, 0],
            ))
            .unwrap(),
            Scope::Domination,
        )
        .unwrap()
    }

    #[cfg(test)]
    pub(crate) fn synthetic(apc_token: u32, truck_token: u32, escort_token: u32) -> Self {
        Self::from_bytes(
            &serde_json::to_vec(&tests::fixture([apc_token, truck_token, escort_token])).unwrap(),
        )
        .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    pub(super) fn fixture(tokens: [u32; 3]) -> Value {
        fixture_for(Scope::Acquisition, tokens)
    }

    pub(super) fn fixture_for(scope: Scope, tokens: [u32; 3]) -> Value {
        let hash = "a".repeat(64);
        let variants: Vec<_> =
            scope
                .roles()
                .iter()
                .copied()
                .zip(tokens)
                .flat_map(|(role, token)| {
                    (0..3).map({
                let hash = hash.clone();
                move |faction| json!({
                    "spawn_type":role,"vehicle_type_token":token,"faction":faction,
                    "template":format!("source-{role}-{faction}"),"packfile_sha256":hash,
                    "rear_threshold_z":-1.25,"front_threshold_z":1.,
                    "chassis_root":"chassis","wheel_root":"wheel",
                })
            })
                })
                .collect();
        json!({
            "schema":match scope {
                Scope::Acquisition => "mag_acquisition_frag_ground_areas_v1",
                Scope::Domination => "mag_domination_frag_ground_areas_v1",
            },
            "source":{"retail_elf_sha256":hash,"retail_main_sha256":hash,"dynamic_catalog_sha256":hash,
                "bounds_proof_sha256":hash,"wheel_overlap_proof_sha256":hash},
            "variants":variants,
        })
    }

    #[test]
    fn frag_ground_area_catalog_rejects_incomplete_roles_nonchassis_root_and_bad_proof() {
        let valid = fixture([3, 7, 9]);
        assert!(Catalog::from_bytes(&serde_json::to_vec(&valid).unwrap()).is_ok());
        for (path, value) in [
            ("/variants/8/faction", json!(1)),
            ("/variants/8/spawn_type", json!(7)),
            ("/variants/8/vehicle_type_token", json!(7)),
            ("/variants/0/wheel_root", json!("hulk")),
            ("/source/wheel_overlap_proof_sha256", json!("wrong")),
        ] {
            let mut changed = valid.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(Catalog::from_bytes(&serde_json::to_vec(&changed).unwrap()).is_err());
        }
    }

    #[test]
    fn frag_ground_area_catalog_requires_exact_mode_and_source() {
        let acquisition = fixture([3, 7, 9]);
        let mut domination = acquisition.clone();
        domination["schema"] = json!("mag_domination_frag_ground_areas_v1");
        domination["variants"].as_array_mut().unwrap().truncate(6);
        let read = |value: &Value, scope| {
            Catalog::from_bytes_for(&serde_json::to_vec(value).unwrap(), scope)
        };
        let catalog = read(&domination, Scope::Domination).unwrap();
        assert!(catalog.validate_main(&"a".repeat(64)).is_ok());
        assert!(catalog.validate_main(&"b".repeat(64)).is_err());
        assert!(read(&domination, Scope::Acquisition).is_err());
        assert!(read(&acquisition, Scope::Domination).is_err());
        domination["variants"][5]["spawn_type"] = json!(9);
        assert!(read(&domination, Scope::Domination).is_err());
    }

    #[test]
    fn frag_ground_area_catalog_admits_either_bound_root_and_uses_current_pose() {
        let catalog = Catalog::synthetic(3, 7, 9);
        let creation: Creation = serde_json::from_value(json!({
            "index":0,"spawn_point_index":1,"spawn_point_token":2,"vehicle_type_token":9,
            "faction":0,"team":0,"modifier":0,"position":{"x":0.,"y":0.,"z":0.},
            "rotation":[0.,0.,0.,1.],"health_quarter_points":600,
        }))
        .unwrap();
        assert_eq!(catalog.accepts_root(&creation, "chassis"), Ok(true));
        assert_eq!(catalog.accepts_root(&creation, "wheel"), Ok(true));
        assert_eq!(catalog.accepts_root(&creation, "hulk"), Ok(false));
        let pose = Pose {
            translation: [10., 2., 20.],
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
        };
        assert_eq!(catalog.location(&creation, pose, [11.1, 2., 19.]), Ok(1));
        assert_eq!(catalog.location(&creation, pose, [11., 2., 19.]), Ok(2));
        assert_eq!(catalog.location(&creation, pose, [8.5, 2., 21.]), Ok(6));
    }
}
