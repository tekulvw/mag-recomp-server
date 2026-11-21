//! Source-bound blast damage areas for operational hospitals.
use crate::{
    domination_structural_world::HospitalHulkAssets, dynamic_collision::Pose,
    hospital_rockets::vehicle_blast_location,
};
use serde::Deserialize;

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
    hospital_collision_sha256: String,
    bounds_proof_sha256: String,
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
            return Err("hospital_blast_areas_limit");
        }
        let mut catalog: Self =
            serde_json::from_slice(raw).map_err(|_| "invalid_hospital_blast_areas")?;
        let source = &catalog.source;
        catalog.variants.sort_by_key(|row| row.faction);
        if catalog.schema != "mag_domination_hospital_blast_areas_v1"
            || ![
                &source.retail_elf_sha256,
                &source.retail_main_sha256,
                &source.hospital_collision_sha256,
                &source.bounds_proof_sha256,
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
            return Err("invalid_hospital_blast_areas");
        }
        Ok(catalog)
    }

    pub(crate) fn validate(
        &self,
        native_elf: &str,
        retail_main: &str,
        assets: &HospitalHulkAssets,
    ) -> Result<(), &'static str> {
        self.validate_sources(native_elf, retail_main, assets.projectile_bindings().iter())
    }

    fn validate_sources<'a>(
        &self,
        native_elf: &str,
        retail_main: &str,
        bodies: impl Iterator<Item = &'a crate::domination_projectile_collision::BodyBinding>,
    ) -> Result<(), &'static str> {
        if self.source.retail_elf_sha256 != native_elf
            || self.source.retail_main_sha256 != retail_main
            || !bodies
                .map(|body| {
                    (
                        body.faction,
                        body.packfile_sha256.as_str(),
                        body.asset_sha256.as_str(),
                        !body.part_ids.is_empty(),
                    )
                })
                .eq(self.variants.iter().map(|row| {
                    (
                        row.faction,
                        row.packfile_sha256.as_str(),
                        self.source.hospital_collision_sha256.as_str(),
                        true,
                    )
                }))
        {
            return Err("hospital_blast_area_source_mismatch");
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
            return Err("invalid_hospital_blast_origin");
        }
        let row = self
            .variants
            .get(usize::from(faction))
            .ok_or("hospital_blast_area_unbound")?;
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
        let hash = "a".repeat(64);
        json!({
            "schema":"mag_domination_hospital_blast_areas_v1",
            "source":{"retail_elf_sha256":hash, "retail_main_sha256":hash,
                "hospital_collision_sha256":hash, "bounds_proof_sha256":hash},
            "variants":(0..3).map(|faction| json!({
                "faction":faction,"packfile_sha256":hash,
                "rear_threshold_z":-1.,"front_threshold_z":1.,
            })).collect::<Vec<_>>(),
        })
    }

    #[test]
    fn hospital_blast_areas_check_source_and_complete_operational_packs() {
        let catalog = Catalog::synthetic();
        let hash = "a".repeat(64);
        let mut bodies: Vec<_> = (0..3)
            .map(|faction| BodyBinding {
                faction,
                asset_sha256: hash.clone(),
                packfile_sha256: hash.clone(),
                part_ids: vec!["body".into()],
            })
            .collect();
        assert_eq!(
            catalog.validate_sources(&hash, &hash, bodies.iter()),
            Ok(())
        );
        assert!(
            catalog
                .validate_sources(&hash, "wrong", bodies.iter())
                .is_err()
        );
        bodies[1].part_ids.clear();
        assert!(
            catalog
                .validate_sources(&hash, &hash, bodies.iter())
                .is_err()
        );
        bodies[1].part_ids.push("body".into());
        bodies[1].packfile_sha256 = "b".repeat(64);
        assert!(
            catalog
                .validate_sources(&hash, &hash, bodies.iter())
                .is_err()
        );
    }

    #[test]
    fn hospital_blast_areas_reject_duplicate_faction_and_use_current_pose() {
        let mut value = fixture();
        value["variants"][2]["faction"] = json!(1);
        assert!(Catalog::from_bytes(&serde_json::to_vec(&value).unwrap()).is_err());
        let catalog = Catalog::synthetic();
        let pose = Pose {
            translation: [10., 2., 20.],
            rotation: [[0., 0., 1.], [0., 1., 0.], [-1., 0., 0.]],
        };
        assert_eq!(catalog.location(0, pose, [11.1, 2., 19.]), Ok(1));
        assert_eq!(catalog.location(1, pose, [11., 2., 19.]), Ok(2));
        assert_eq!(catalog.location(2, pose, [8.5, 2., 21.]), Ok(6));
        assert!(catalog.location(3, pose, [8.5, 2., 21.]).is_err());
    }
}
