//! Invented catalog inputs shared by the CLI and loopback worker tests.
use sha2::{Digest, Sha256};
use std::{
    path::PathBuf,
    sync::atomic::{AtomicUsize, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

pub(super) struct SyntheticCatalogs(pub(super) PathBuf);

impl SyntheticCatalogs {
    pub(super) fn new() -> Self {
        static NEXT_DIRECTORY: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "mag-synthetic-runtime-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT_DIRECTORY.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).unwrap();
        let files = Self(path);
        let items = serde_json::json!({"schema_version":2,"application_id":20231,
            "items":{"FactionItemList0:2":{"authored_ref":"FactionItemList0:2",
                "faction_list":"FactionItemList0","local_ordinal":2,"wire_item_id":null,
                "name":"synthetic_weapon","item_type":"kWeapon",
                "definition":{"group":"weapons","name":"synthetic_weapon"}}},
            "definitions":{"weapons":{"synthetic_weapon":{"parameters":{}}}},
            "resources":{}});
        files.write("items.json", &items);
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(files.0.join("items.json")).unwrap())
        );
        files.write("spawns.json", &serde_json::json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC CLI schema test",
            "active":{"map_name":"synthetic_arena","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_arena","spawn_count":1,"locations":[
                {"index":0,"token":1,"kind":"on_foot","position":{"x":10.,"y":2.,"z":12.}}
            ]}]
        }));
        let mut wire: serde_json::Value =
            serde_json::from_str(include_str!("../../data/synthetic-wire-profiles.json")).unwrap();
        wire["profiles"] = serde_json::json!([wire["profiles"][1].clone()]);
        files.write("wire.json", &wire);
        files.write("blast.json", &serde_json::json!({"schema_version":1,"applications":[20230,20231],
            "character_damage":{"armor_rating_reduction":0.25,"armor_prevention_reduction":0.75,"hit_location":3,"damage_type":10}}));
        let mut provenance = serde_json::json!({"verified_binding":{"validated":true,
            "positions_compared":667,"blank_positions_compared":25,"blank_zero_empirically_verified":true}});
        for name in [
            "original_retail_elf_sha256",
            "snapshot_sha256",
            "catalog_sha256",
            "rules_sha256",
            "owner_attestation_sha256",
            "gear_slots_sha256",
            "comparator_sha256",
        ] {
            provenance[name] = serde_json::json!(if name == "catalog_sha256" {
                digest.clone()
            } else {
                "a".repeat(64)
            });
        }
        let mut kit = vec![None; 22];
        kit[..3].fill(Some(2_u16));
        let loadouts: Vec<_> = (0..5)
            .map(|index| {
                serde_json::json!({
                    "update_id":1,"items":if index < 3 {kit.clone()} else {vec![None;22]},
                    "primary_ammo":if index < 3 {7} else {0}
                })
            })
            .collect();
        files.write("bootstrap.json", &serde_json::json!({"schema_version":1,"application_id":20231,
            "provenance":provenance,
            "factions":(0..3).map(|faction|serde_json::json!({"faction":faction,"class_ref":"CharacterClassInfo0","loadouts":loadouts})).collect::<Vec<_>>()
        }));
        files.write("combat.json", &serde_json::json!({"schema_version":2,"application_id":20231,
            "status":"SYNTHETIC schema test","authority":{"bullet_damage":true,"grenade_damage":false,
                "grenade_physics":false,"armor_reduction":true,"body_damage":true,"healing_schedule":false,"ammo_tokens":false},
            "provenance":{"catalog_sha256":digest},"bullets":{},"grenades":{},"armor":{},"healing":{},"unsupported":{},
            "body":(0..16).map(|location|serde_json::json!({"location":location,"armor_area":0,"multiplier":1.0,"source":"SYNTHETIC"})).collect::<Vec<_>>(),
            "limitations":["Synthetic schema data proves no native compatibility"]}));
        files.write("armory.json", &serde_json::json!({"schema_version":1,"application_id":20231,
            "source_sha256":{"gameplay/data/retail-items.json":digest},
            "items":[{"wire_id":2,"name":"synthetic_weapon","authored_ref":"FactionItemList0:2","available_bit":2,"factions":[0,1,2],"primary_ammo":7}],
            "factions":(0..3).map(|faction|serde_json::json!({"faction":faction,"available_item_bits":[2],"skill_ids":[],
                "loadouts":loadouts.iter().map(|row|serde_json::json!({"update_id":row["update_id"],"items":row["items"]})).collect::<Vec<_>>()
            })).collect::<Vec<_>>()
        }));
        files.write("skills.json", &serde_json::json!({"schema_version":2,"application_id":20231,
            "source_sha256":{"gameplay/data/retail-items.json":digest},
            "table":{"Skills":{"MagCharacterSkill1":{"MagCharacterSkill":{"Green":{"Available":true}}}}}}));
        files.write("fall.json", &serde_json::json!({"schema_version":1,"source_build":"retail","application_id":20231,
            "min_height":3.0,"damage_factor":4.0,"minimum_damage_exclusive":2.0,"jump_fall_state":1,"damage_type":5,"damage_source_type":1}));
        files
    }

    pub(super) fn write(&self, name: &str, value: &serde_json::Value) {
        std::fs::write(self.0.join(name), serde_json::to_vec(value).unwrap()).unwrap();
    }

    #[allow(dead_code)]
    pub(super) fn read(&self, name: &str) -> serde_json::Value {
        serde_json::from_slice(&std::fs::read(self.0.join(name)).unwrap()).unwrap()
    }

    pub(super) fn runtime_args(&self) -> Vec<String> {
        let mut args = vec!["--application-id".into(), "20231".into()];
        for (flag, name) in [
            ("--item-catalog", "items.json"),
            ("--spawn-catalog", "spawns.json"),
            ("--wire-profiles", "wire.json"),
            ("--character-blast-rules", "blast.json"),
            ("--bootstrap-loadouts", "bootstrap.json"),
            ("--retail-combat-metadata", "combat.json"),
            ("--retail-armory-catalog", "armory.json"),
            ("--skill-catalog", "skills.json"),
            ("--fall-damage-rules", "fall.json"),
        ] {
            args.push(flag.into());
            args.push(self.0.join(name).to_str().unwrap().into());
        }
        args
    }

    #[allow(dead_code)]
    pub(super) fn args(&self) -> Vec<String> {
        let mut args = self.runtime_args();
        args.push("--validate-catalogs".into());
        args
    }
}

impl Drop for SyntheticCatalogs {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
