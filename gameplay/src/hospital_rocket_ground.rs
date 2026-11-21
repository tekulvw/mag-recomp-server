//! External Domination Ground rocket damage areas and exact source role binding.
use crate::{
    dynamic_collision::{self, Pose},
    dynamic_collision_runtime::{Resources, RuntimeBindings},
    ground_vehicles,
};

use serde::Deserialize;
use std::collections::BTreeSet;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    schema: String,
    source: Source,
    variants: Vec<Variant>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    retail_main_sha256: String,
    dynamic_catalog_sha256: String,
    ground_damage_policy_sha256: String,
    bounds_proof_sha256: String,
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
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

impl Catalog {
    pub(crate) fn from_bytes(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() > 16 * 1024 {
            return Err("hospital_rocket_ground_rules_limit");
        }
        let catalog: Self =
            serde_json::from_slice(raw).map_err(|_| "invalid_hospital_rocket_ground_rules")?;
        let source = &catalog.source;
        if catalog.schema != "mag_domination_hospital_rocket_ground_v1"
            || ![
                &source.retail_elf_sha256,
                &source.retail_main_sha256,
                &source.dynamic_catalog_sha256,
                &source.ground_damage_policy_sha256,
                &source.bounds_proof_sha256,
            ]
            .into_iter()
            .all(|value| digest(value))
            || catalog.variants.len() != 6
        {
            return Err("invalid_hospital_rocket_ground_rules");
        }

        let mut roles = [None; 2];
        let mut keys = BTreeSet::new();
        let mut templates = BTreeSet::new();
        for row in &catalog.variants {
            let role = match row.spawn_type {
                3 => 0,
                7 => 1,
                _ => return Err("invalid_hospital_rocket_ground_role"),
            };
            let token = roles[role].get_or_insert(row.vehicle_type_token);
            if row.faction > 2
                || row.vehicle_type_token == 0
                || *token != row.vehicle_type_token
                || !keys.insert((row.spawn_type, row.faction))
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
            {
                return Err("invalid_hospital_rocket_ground_variant");
            }
        }
        if roles[0] == roles[1] {
            return Err("invalid_hospital_rocket_ground_role");
        }
        Ok(catalog)
    }

    pub(crate) fn validate(
        &self,
        retail_elf_sha256: &str,
        retail_main_sha256: &str,
        ground_damage_policy_sha256: &str,
        ground: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        if self.source.retail_elf_sha256 != retail_elf_sha256
            || self.source.retail_main_sha256 != retail_main_sha256
            || self.source.ground_damage_policy_sha256 != ground_damage_policy_sha256
        {
            return Err("hospital_rocket_ground_source_mismatch");
        }
        let mut counts = [0; 2];
        for creation in ground.creations() {
            let source = ground
                .vehicle_source(creation.index)
                .ok_or("hospital_rocket_ground_role_unbound")?;
            let row = self.variant(creation)?;
            if row.spawn_type != source.spawn_type
                || source.vehicle_type_token != creation.vehicle_type_token
            {
                return Err("hospital_rocket_ground_role_unbound");
            }
            counts[usize::from(row.spawn_type == 7)] += 1;
        }
        if counts != [8, 8] {
            return Err("hospital_rocket_ground_role_unbound");
        }
        Ok(())
    }

    pub(crate) fn validate_dynamic(
        &self,
        resources: &Resources,
        ground: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        self.validate_dynamic_templates(&resources.catalog, ground, |creation| {
            resources.vehicle_template(creation)
        })
    }

    pub(crate) fn validate_dynamic_parts(
        &self,
        catalog: &dynamic_collision::Catalog,
        bindings: &RuntimeBindings,
        ground: &ground_vehicles::Catalog,
    ) -> Result<(), &'static str> {
        self.validate_dynamic_templates(catalog, ground, |creation| {
            let mut matching = bindings.vehicles.iter().filter(|row| {
                row.vehicle_type_token == creation.vehicle_type_token
                    && row.faction == creation.faction
            });
            let row = matching
                .next()
                .ok_or("hospital_rocket_ground_dynamic_template_mismatch")?;
            if matching.next().is_some() {
                return Err("hospital_rocket_ground_dynamic_template_mismatch");
            }
            Ok(row.template.as_str())
        })
    }

    fn validate_dynamic_templates<'a>(
        &self,
        catalog: &dynamic_collision::Catalog,
        ground: &ground_vehicles::Catalog,
        template: impl Fn(&ground_vehicles::Creation) -> Result<&'a str, &'static str>,
    ) -> Result<(), &'static str> {
        if self.source.dynamic_catalog_sha256 != catalog.fingerprint() {
            return Err("hospital_rocket_ground_dynamic_source_mismatch");
        }
        for creation in ground.creations() {
            if self.variant(creation)?.template != template(creation)? {
                return Err("hospital_rocket_ground_dynamic_template_mismatch");
            }
        }
        Ok(())
    }

    fn variant(&self, creation: &ground_vehicles::Creation) -> Result<&Variant, &'static str> {
        self.variants
            .iter()
            .find(|row| {
                row.vehicle_type_token == creation.vehicle_type_token
                    && row.faction == creation.faction
            })
            .ok_or("hospital_rocket_ground_variant_unbound")
    }

    pub(crate) fn location(
        &self,
        creation: &ground_vehicles::Creation,
        pose: Pose,
        blast_origin: [f64; 3],
    ) -> Result<u8, &'static str> {
        pose.validate()?;
        if !blast_origin
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= 100_000.)
        {
            return Err("invalid_hospital_rocket_ground_origin");
        }
        let row = self.variant(creation)?;
        Ok(crate::hospital_rockets::vehicle_blast_location(
            pose,
            blast_origin,
            row.rear_threshold_z,
            row.front_threshold_z,
        ))
    }

    #[cfg(test)]
    pub(crate) fn synthetic(apc_token: u32, motor_pool_token: u32) -> Self {
        Self::from_bytes(&serde_json::to_vec(&tests::rules(apc_token, motor_pool_token)).unwrap())
            .unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    pub(super) fn rules(apc_token: u32, motor_pool_token: u32) -> Value {
        let sha = "a".repeat(64);
        let variants: Vec<_> = [(3, apc_token), (7, motor_pool_token)]
            .into_iter()
            .flat_map(|(spawn_type, vehicle_type_token)| {
                (0..3).map({
                    let sha = sha.clone();
                    move |faction| json!({
                        "spawn_type":spawn_type,"vehicle_type_token":vehicle_type_token,"faction":faction,
                        "template":format!("synthetic-{spawn_type}-{faction}"),"packfile_sha256":sha,
                        "rear_threshold_z":-1.25,"front_threshold_z":1.0,
                    })
                })
            })
            .collect();
        json!({
            "schema":"mag_domination_hospital_rocket_ground_v1",
            "source":{"retail_elf_sha256":sha,"retail_main_sha256":sha,"dynamic_catalog_sha256":sha,
                "ground_damage_policy_sha256":sha,"bounds_proof_sha256":sha},
            "variants":variants,
        })
    }

    #[test]
    fn rocket_ground_location_uses_current_asymmetric_model_axes_and_inclusive_middle() {
        let catalog = Catalog::synthetic(123, 456);
        let creation: ground_vehicles::Creation = serde_json::from_value(json!({
            "index":0,"spawn_point_index":1,"spawn_point_token":2,"vehicle_type_token":123,
            "faction":0,"team":0,"modifier":0,"position":{"x":0.,"y":0.,"z":0.},
            "rotation":[0.,0.,0.,1.],"health_quarter_points":600,
        }))
        .unwrap();
        let pose = Pose {
            translation: [10., 2., 20.],
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
        };
        let hit = |x, z| {
            catalog
                .location(&creation, pose, [10. + z, 2., 20. - x])
                .unwrap()
        };
        assert_eq!([hit(1., 1.01), hit(1., 1.), hit(1., -1.26)], [1, 2, 3]);
        assert_eq!([hit(0., 1.01), hit(-1., -1.25), hit(-1., -1.26)], [4, 5, 6]);
    }

    #[test]
    fn rocket_ground_rules_reject_role_duplicates_foreign_rows_and_missing_sources() {
        let parse = |value| Catalog::from_bytes(&serde_json::to_vec(&value).unwrap()).is_ok();
        let original = rules(123, 456);
        assert!(parse(original.clone()));
        for (path, value) in [
            ("/variants/0/spawn_type", json!(9)),
            ("/variants/0/vehicle_type_token", json!(456)),
            ("/variants/1/faction", json!(0)),
            ("/variants/0/rear_threshold_z", json!(2.0)),
            ("/source/dynamic_catalog_sha256", json!("absent")),
        ] {
            let mut changed = original.clone();
            *changed.pointer_mut(path).unwrap() = value;
            assert!(!parse(changed), "{path}");
        }
    }
}
