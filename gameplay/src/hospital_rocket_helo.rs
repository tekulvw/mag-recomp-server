//! Source-bound rocket damage-area thresholds for the operational Helo variants.
use crate::{
    domination_structural_world::HeloCollisionAssets, dynamic_collision::Pose,
    hospital_rockets::vehicle_blast_location,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Catalog {
    schema: String,
    source: Source,
    variants: [Variant; 3],
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Source {
    retail_elf_sha256: String,
    retail_main_sha256: String,
    helo_models_sha256: String,
    helo_damage_sha256: String,
    area_proof_sha256: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Variant {
    faction: u8,
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
            return Err("hospital_rocket_helo_areas_limit");
        }
        let mut catalog: Self =
            serde_json::from_slice(raw).map_err(|_| "invalid_hospital_rocket_helo_areas")?;
        let source = &catalog.source;
        catalog.variants.sort_by_key(|row| row.faction);
        if catalog.schema != "mag_domination_hospital_rocket_helo_areas_v1"
            || ![
                &source.retail_elf_sha256,
                &source.retail_main_sha256,
                &source.helo_models_sha256,
                &source.helo_damage_sha256,
                &source.area_proof_sha256,
            ]
            .into_iter()
            .all(|value| digest(value))
            || !catalog.variants.iter().map(|row| row.faction).eq(0..3)
            || catalog.variants.iter().any(|row| {
                !digest(&row.packfile_sha256)
                    || !(-1000. ..=1000.).contains(&row.rear_threshold_z)
                    || !(-1000. ..=1000.).contains(&row.front_threshold_z)
                    || row.rear_threshold_z >= row.front_threshold_z
            })
        {
            return Err("invalid_hospital_rocket_helo_areas");
        }
        Ok(catalog)
    }

    pub(crate) fn validate(
        &self,
        native_elf: &str,
        retail_main: &str,
        damage_raw: &[u8],
        assets: &HeloCollisionAssets,
    ) -> Result<(), &'static str> {
        self.validate_sources(
            native_elf,
            retail_main,
            damage_raw,
            assets.projectile_bindings(),
        )
    }

    fn validate_sources<'a>(
        &self,
        native_elf: &str,
        retail_main: &str,
        damage_raw: &[u8],
        bodies: impl Iterator<Item = &'a crate::domination_projectile_collision::BodyBinding>,
    ) -> Result<(), &'static str> {
        if self.source.retail_elf_sha256 != native_elf
            || self.source.retail_main_sha256 != retail_main
            || self.source.helo_damage_sha256 != format!("{:x}", Sha256::digest(damage_raw))
            || !bodies
                .map(|row| {
                    (
                        row.faction,
                        row.packfile_sha256.as_str(),
                        row.asset_sha256.as_str(),
                    )
                })
                .eq(self.variants.iter().map(|row| {
                    (
                        row.faction,
                        row.packfile_sha256.as_str(),
                        self.source.helo_models_sha256.as_str(),
                    )
                }))
        {
            return Err("hospital_rocket_helo_area_source_mismatch");
        }
        Ok(())
    }

    pub(crate) fn location(
        &self,
        faction: u8,
        pose: Pose,
        blast_origin: [f64; 3],
    ) -> Result<u8, &'static str> {
        pose.validate()?;
        if !blast_origin
            .into_iter()
            .all(|v| v.is_finite() && v.abs() <= 100_000.)
        {
            return Err("invalid_hospital_rocket_helo_origin");
        }
        let row = self
            .variants
            .get(usize::from(faction))
            .ok_or("hospital_rocket_helo_area_unbound")?;
        Ok(vehicle_blast_location(
            pose,
            blast_origin,
            row.rear_threshold_z,
            row.front_threshold_z,
        ))
    }

    #[cfg(test)]
    pub(crate) fn synthetic() -> Self {
        Self::from_bytes(&serde_json::to_vec(&tests::fixture()).unwrap()).unwrap()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domination_projectile_collision::BodyBinding;
    use serde_json::{Value, json};

    pub(super) fn fixture() -> Value {
        let digest = "a".repeat(64);
        json!({
            "schema":"mag_domination_hospital_rocket_helo_areas_v1",
            "source":{"retail_elf_sha256":digest,"retail_main_sha256":digest,
                "helo_models_sha256":digest,"area_proof_sha256":digest,
                "helo_damage_sha256":format!("{:x}",Sha256::digest(b"synthetic"))},
            "variants": (0..3).map(|faction| json!({
                "faction":faction,"packfile_sha256":digest,
                "rear_threshold_z":-2.25,"front_threshold_z":1.,
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn helo_rocket_areas_bind_all_sources_and_compute_rotated_area() {
        let catalog = Catalog::synthetic();
        let digest = "a".repeat(64);
        let mut bodies: Vec<_> = (0..3)
            .map(|faction| BodyBinding {
                asset_sha256: digest.clone(),
                faction,
                packfile_sha256: digest.clone(),
                part_ids: vec!["synthetic-panel".to_owned()],
            })
            .collect();
        assert!(
            catalog
                .validate_sources(&digest, &digest, b"synthetic", bodies.iter())
                .is_ok()
        );
        assert!(
            catalog
                .validate_sources(&digest, &digest, b"other", bodies.iter())
                .is_err()
        );
        bodies[1].packfile_sha256 = "b".repeat(64);
        assert!(
            catalog
                .validate_sources(&digest, &digest, b"synthetic", bodies.iter())
                .is_err()
        );
        let pose = Pose {
            translation: [10., 2., 20.],
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
        };
        for (origin, expected) in [
            ([11.2, 2., 19.], 1),
            ([11., 2., 19.], 2),
            ([7.5, 2., 19.], 3),
            ([11.2, 2., 20.], 4),
            ([7.75, 2., 21.], 5),
            ([7.5, 2., 21.], 6),
        ] {
            assert_eq!(catalog.location(2, pose, origin), Ok(expected));
        }
    }

    #[test]
    fn helo_rocket_areas_reject_duplicate_factions_and_inverted_thresholds() {
        let base = fixture();
        for (path, value) in [
            ("/variants/2/faction", json!(1)),
            ("/variants/0/front_threshold_z", json!(-3.)),
        ] {
            let mut input = base.clone();
            *input.pointer_mut(path).unwrap() = value;
            assert!(Catalog::from_bytes(&serde_json::to_vec(&input).unwrap()).is_err());
        }
    }
}
