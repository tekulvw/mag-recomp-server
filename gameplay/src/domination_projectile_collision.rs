//! Source bindings for represented projectile contact overlays.
use serde::Deserialize;
use std::collections::BTreeSet;

use crate::{
    domination_structural_world::{
        HeloCollisionAssets, HospitalHulkAssets, SequenceCollisionAssets,
    },
    projectile_contact::Coverage,
};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContactBindings {
    schema: String,
    #[serde(skip)]
    scope: Scope,
    map_name: String,
    retail_elf_sha256: String,
    proof_sha256: String,
    static_world_sha256: String,
    dynamic_catalog_sha256: String,
    ground_phantom_roots: BTreeSet<String>,
    static_coverage_policy: StaticCoveragePolicy,
    sequence: Vec<SequenceRow>,
    hospital: Vec<BodyRow>,
    helo: Vec<BodyRow>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Scope {
    #[default]
    Domination,
    Acquisition {
        filter_row23: u32,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AcquisitionBindings {
    schema: String,
    mode: u8,
    map_name: String,
    retail_elf_sha256: String,
    proof_sha256: String,
    static_world_sha256: String,
    dynamic_catalog_sha256: String,
    ground_phantom_roots: BTreeSet<String>,
    static_coverage_policy: StaticCoveragePolicy,
    filter_row23: u32,
    sequence: Vec<SequenceRow>,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
enum StaticCoveragePolicy {
    RepresentedReferencePose,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
pub(crate) struct SequenceBinding {
    pub(crate) source_sha256: String,
    pub(crate) neutral_sha256: String,
    pub(crate) collision_tag: u8,
    pub(crate) part_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd)]
#[serde(deny_unknown_fields)]
pub(crate) struct BodyBinding {
    pub(crate) asset_sha256: String,
    pub(crate) faction: u8,
    pub(crate) packfile_sha256: String,
    pub(crate) part_ids: Vec<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct SequenceRow {
    binding: SequenceBinding,
    contact: Properties,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct BodyRow {
    binding: BodyBinding,
    contact: Properties,
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Properties {
    pub(crate) root_filter_info: u8,
    pub(crate) child_filter_info: u8,
    pub(crate) convex_radius: f64,
}

fn digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|v| v.is_ascii_digit() || (b'a'..=b'f').contains(&v))
}

fn part_ids_valid(ids: &[String]) -> bool {
    !ids.is_empty()
        && ids.iter().all(|id| !id.trim().is_empty())
        && ids.windows(2).all(|pair| pair[0] < pair[1])
}

impl Properties {
    fn valid(self, root: u8) -> bool {
        self.root_filter_info == root && self.child_filter_info == 0 && self.convex_radius == 0.
    }
}

impl ContactBindings {
    pub(crate) fn from_acquisition_json(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() > 512 * 1024 {
            return Err("projectile_contact_bindings_limit");
        }
        let input: AcquisitionBindings =
            serde_json::from_slice(raw).map_err(|_| "invalid_acquisition_projectile_contacts")?;
        if input.schema != "mag_acquisition_projectile_contacts_v1" || input.mode != 1 {
            return Err("invalid_acquisition_projectile_contacts");
        }
        let mut bindings = Self {
            schema: input.schema,
            scope: Scope::Acquisition {
                filter_row23: input.filter_row23,
            },
            map_name: input.map_name,
            retail_elf_sha256: input.retail_elf_sha256,
            proof_sha256: input.proof_sha256,
            static_world_sha256: input.static_world_sha256,
            dynamic_catalog_sha256: input.dynamic_catalog_sha256,
            ground_phantom_roots: input.ground_phantom_roots,
            static_coverage_policy: input.static_coverage_policy,
            sequence: input.sequence,
            hospital: Vec::new(),
            helo: Vec::new(),
        };
        bindings.sort_sequence();
        if !bindings.common_valid() {
            return Err("invalid_acquisition_projectile_contacts");
        }
        Ok(bindings)
    }

    pub(crate) fn from_json(raw: &[u8]) -> Result<Self, &'static str> {
        if raw.len() > 512 * 1024 {
            return Err("projectile_contact_bindings_limit");
        }
        let mut parsed: Self =
            serde_json::from_slice(raw).map_err(|_| "invalid_projectile_contact_bindings")?;
        parsed.sort_sequence();
        for rows in [&mut parsed.hospital, &mut parsed.helo] {
            rows.sort_by(|left, right| left.binding.cmp(&right.binding));
        }
        if parsed.schema != "mag_domination_projectile_contacts_v1"
            || !parsed.common_valid()
            || [&parsed.hospital, &parsed.helo].into_iter().any(|rows| {
                !rows.is_empty()
                    && (rows.len() != 3
                        || rows
                            .iter()
                            .map(|row| row.binding.faction)
                            .collect::<BTreeSet<_>>()
                            .len()
                            != 3
                        || rows.iter().any(|row| {
                            row.binding.faction > 2
                                || !digest(&row.binding.asset_sha256)
                                || !digest(&row.binding.packfile_sha256)
                                || !part_ids_valid(&row.binding.part_ids)
                                || !row.contact.valid(22)
                        }))
            })
            || parsed.hospital.is_empty()
        {
            return Err("invalid_projectile_contact_bindings");
        }
        Ok(parsed)
    }

    fn sort_sequence(&mut self) {
        self.sequence
            .sort_by(|left, right| left.binding.cmp(&right.binding));
    }

    fn common_valid(&self) -> bool {
        !self.map_name.is_empty()
            && digest(&self.retail_elf_sha256)
            && digest(&self.proof_sha256)
            && digest(&self.static_world_sha256)
            && digest(&self.dynamic_catalog_sha256)
            && self
                .ground_phantom_roots
                .iter()
                .map(String::as_str)
                .eq(["wheel"])
            && !self.sequence.is_empty()
            && !self
                .sequence
                .windows(2)
                .any(|pair| pair[0].binding == pair[1].binding)
            && self.sequence.iter().all(|row| {
                let binding = &row.binding;
                digest(&binding.source_sha256)
                    && digest(&binding.neutral_sha256)
                    && matches!(binding.collision_tag, 9 | 11 | 29)
                    && part_ids_valid(&binding.part_ids)
                    && row.contact.valid(binding.collision_tag)
            })
    }

    pub(crate) fn validate_acquisition(
        &self,
        map: &str,
        retail_elf_sha256: &str,
        filter_row23: u32,
        assets: &SequenceCollisionAssets,
    ) -> Result<(), &'static str> {
        self.validate_acquisition_identity(map, retail_elf_sha256, filter_row23)?;
        if !self.sequence_matches(assets)? {
            return Err("projectile_contact_binding_mismatch");
        }
        Ok(())
    }

    fn validate_acquisition_identity(
        &self,
        map: &str,
        retail_elf_sha256: &str,
        filter_row23: u32,
    ) -> Result<(), &'static str> {
        if self.scope != (Scope::Acquisition { filter_row23 })
            || self.map_name != map
            || self.retail_elf_sha256 != retail_elf_sha256
        {
            return Err("projectile_contact_binding_mismatch");
        }
        Ok(())
    }

    fn sequence_matches(&self, assets: &SequenceCollisionAssets) -> Result<bool, &'static str> {
        Ok(assets
            .projectile_bindings()?
            .iter()
            .eq(self.sequence.iter().map(|row| &row.binding)))
    }

    pub(crate) fn validate_scene_filter(&self, row: u32) -> Result<(), &'static str> {
        if matches!(self.scope, Scope::Acquisition { filter_row23 } if filter_row23 != row) {
            return Err("projectile_contact_filter_mismatch");
        }
        Ok(())
    }

    pub(crate) fn validate(
        &self,
        map: &str,
        retail_elf_sha256: &str,
        sequence: &SequenceCollisionAssets,
        hospital: &HospitalHulkAssets,
        helo: Option<&HeloCollisionAssets>,
    ) -> Result<(), &'static str> {
        if self.scope != Scope::Domination
            || self.map_name != map
            || self.retail_elf_sha256 != retail_elf_sha256
            || !self.sequence_matches(sequence)?
            || !hospital
                .projectile_bindings()
                .iter()
                .eq(self.hospital.iter().map(|row| &row.binding))
            || !helo
                .into_iter()
                .flat_map(HeloCollisionAssets::projectile_bindings)
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .eq(self.helo.iter().map(|row| &row.binding))
        {
            return Err("projectile_contact_binding_mismatch");
        }
        Ok(())
    }

    pub(crate) fn validate_static(
        &self,
        world: &crate::collision::CollisionWorld,
    ) -> Result<(), &'static str> {
        if self.static_world_sha256 != world.source_sha256() {
            return Err("projectile_static_reference_source_mismatch");
        }
        Ok(())
    }

    pub(crate) fn validate_dynamic(
        &self,
        catalog: &crate::dynamic_collision::Catalog,
    ) -> Result<(), &'static str> {
        if self.dynamic_catalog_sha256 != catalog.fingerprint() {
            return Err("projectile_dynamic_catalog_source_mismatch");
        }
        Ok(())
    }

    pub(crate) fn ground_phantom(&self, root: &str) -> bool {
        self.ground_phantom_roots.contains(root)
    }

    pub(crate) fn coverage(&self) -> Coverage {
        match self.static_coverage_policy {
            StaticCoveragePolicy::RepresentedReferencePose => Coverage::ReferencePose,
        }
    }
}

#[derive(Clone)]
pub(crate) enum SurfaceBinding {
    Sequence(SequenceBinding),
    Hospital(BodyBinding),
    Helo(BodyBinding),
    HospitalHulk { layer: u8 },
}

impl SurfaceBinding {
    pub(crate) fn layers(
        &self,
        bindings: &ContactBindings,
    ) -> Result<(u8, Option<Properties>), &'static str> {
        if bindings.scope != Scope::Domination && !matches!(self, Self::Sequence(_)) {
            return Err("projectile_overlay_scope_mismatch");
        }
        let properties = match self {
            Self::Sequence(key) => bindings
                .sequence
                .binary_search_by(|row| row.binding.cmp(key))
                .ok()
                .map(|index| bindings.sequence[index].contact),
            Self::Hospital(key) => find_body(&bindings.hospital, key),
            Self::Helo(key) => find_body(&bindings.helo, key),
            Self::HospitalHulk { layer } => return Ok((*layer, None)),
        }
        .ok_or("projectile_overlay_binding_unavailable")?;
        Ok((properties.root_filter_info, Some(properties)))
    }
}

fn find_body(rows: &[BodyRow], key: &BodyBinding) -> Option<Properties> {
    rows.binary_search_by(|row| row.binding.cmp(key))
        .ok()
        .map(|index| rows[index].contact)
}

#[cfg(test)]
pub(crate) mod acquisition_tests {
    use super::*;
    use serde_json::{Value, json};

    pub(crate) fn acquisition() -> Value {
        let hash = "a".repeat(64);
        json!({
            "schema":"mag_acquisition_projectile_contacts_v1",
            "mode":1,
            "map_name":"synthetic-acquisition",
            "retail_elf_sha256":hash,
            "proof_sha256":hash,
            "static_world_sha256":hash,
            "dynamic_catalog_sha256":hash,
            "ground_phantom_roots":["wheel"],
            "static_coverage_policy":"represented_reference_pose",
            "filter_row23":0x00600641,
            "sequence":[{
                "binding":{"source_sha256":hash,"neutral_sha256":hash,"collision_tag":9,
                           "part_ids":["synthetic-pack-part"]},
                "contact":{"root_filter_info":9,"child_filter_info":0,"convex_radius":0.0},
            }],
        })
    }

    fn parse(value: &Value) -> Result<ContactBindings, &'static str> {
        ContactBindings::from_acquisition_json(&serde_json::to_vec(value).unwrap())
    }

    #[test]
    fn acquisition_contacts_enforce_schema_row_and_surface_scope() {
        let raw = acquisition();
        let contacts = parse(&raw).unwrap();
        assert!(ContactBindings::from_json(&serde_json::to_vec(&raw).unwrap()).is_err());
        let mut wrong = raw.clone();
        wrong["mode"] = json!(2);
        assert!(parse(&wrong).is_err());
        let elf = "a".repeat(64);
        assert!(
            contacts
                .validate_acquisition_identity("synthetic-acquisition", &elf, 0x00600641)
                .is_ok()
        );
        assert!(
            contacts
                .validate_acquisition_identity("synthetic-acquisition", &elf, 0x006006c1)
                .is_err()
        );
        assert!(
            contacts
                .validate_acquisition_identity("another-map", &elf, 0x00600641)
                .is_err()
        );
        assert!(
            contacts
                .validate_acquisition_identity("synthetic-acquisition", &"b".repeat(64), 0x00600641)
                .is_err()
        );
        assert!(contacts.validate_scene_filter(0x006006c1).is_err());
        for field in ["hospital", "helo"] {
            let mut wrong = raw.clone();
            wrong[field] = json!([]);
            assert!(parse(&wrong).is_err());
        }
        assert!(
            SurfaceBinding::HospitalHulk { layer: 17 }
                .layers(&contacts)
                .is_err()
        );
        let body = BodyBinding {
            asset_sha256: "a".repeat(64),
            faction: 0,
            packfile_sha256: "a".repeat(64),
            part_ids: vec!["body".into()],
        };
        assert!(
            SurfaceBinding::Hospital(body.clone())
                .layers(&contacts)
                .is_err()
        );
        assert!(SurfaceBinding::Helo(body).layers(&contacts).is_err());
        let sequence = SurfaceBinding::Sequence(contacts.sequence[0].binding.clone());
        assert_eq!(sequence.layers(&contacts).unwrap().0, 9);
    }
}
