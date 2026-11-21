//! Source-bound rules and explicit activation for the owned retail Domination round.
use crate::{
    Engine,
    acquisition_round::{Round, Rules},
    objective_catalog::ObjectiveCatalog,
    spawn_catalog::SpawnCatalog,
};
use serde::Deserialize;
use std::{
    io::{self, Read, Write},
    path::Path,
};

const MAX_SETUP_BYTES: usize = 4096;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    format: String,
    schema_version: u32,
    spawn_catalog_sha256: String,
    authored_objective_catalog_sha256: String,
    game_rules_sha256: String,
    game_rules_json_pointer: String,
    starting_points: [i16; 2],
    major_victory_percent: u8,
    end_game_delay_ms: u32,
}

#[derive(Deserialize)]
struct SpawnProvenance<'a> {
    #[serde(borrow)]
    game_rules_source: RulesProvenance<'a>,
}

#[derive(Deserialize)]
struct RulesProvenance<'a> {
    sha256: &'a str,
    rules_json_pointer: &'a str,
}

impl Setup {
    pub fn load(path: impl AsRef<Path>) -> io::Result<Self> {
        let mut bytes = String::new();
        std::fs::File::open(path)?
            .take((MAX_SETUP_BYTES + 1) as u64)
            .read_to_string(&mut bytes)?;
        Self::from_json(&bytes)
    }

    pub fn from_json(text: &str) -> io::Result<Self> {
        if text.len() > MAX_SETUP_BYTES {
            return Err(io::Error::other("domination_round_setup_too_large"));
        }
        let value: Self = serde_json::from_str(text).map_err(io::Error::other)?;
        value.validate().map_err(io::Error::other)?;
        Ok(value)
    }

    fn validate(&self) -> Result<(), &'static str> {
        if self.format != "mag_domination_round_v1"
            || self.schema_version != 1
            || ![
                &self.spawn_catalog_sha256,
                &self.authored_objective_catalog_sha256,
                &self.game_rules_sha256,
            ]
            .into_iter()
            .all(|hash| hash.len() == 64 && hash.bytes().all(|digit| digit.is_ascii_hexdigit()))
            || !self.game_rules_json_pointer.starts_with('/')
            || self.game_rules_json_pointer.len() > 256
            || self.starting_points.iter().any(|points| *points <= 0)
            || self.major_victory_percent > 100
            || self.end_game_delay_ms > 60_000
        {
            return Err("invalid_domination_round_setup");
        }
        Ok(())
    }

    pub(crate) fn round_for(
        &self,
        spawns: &SpawnCatalog,
        objectives: Option<&ObjectiveCatalog>,
    ) -> Result<Round, &'static str> {
        self.validate()?;
        let profile = spawns.match_profile();
        if profile.game_type != 2 || !profile.auto_assign_teams || !profile.has_objectives {
            return Err("domination_round_requires_explicit_mode2_profile");
        }
        let objectives = objectives.ok_or("domination_round_requires_objective_source")?;
        objectives.validate_match(spawns.map_name(), profile)?;
        if spawns.source_sha256() != Some(self.spawn_catalog_sha256.as_str())
            || objectives.source().authored_catalog_sha256 != self.authored_objective_catalog_sha256
            || profile.maximum_points != self.starting_points
        {
            return Err("domination_round_profile_source_mismatch");
        }
        let source: SpawnProvenance<'_> = serde_json::from_str(spawns.authored_source())
            .map_err(|_| "domination_round_missing_game_rules_source")?;
        if source.game_rules_source.sha256 != self.game_rules_sha256
            || source.game_rules_source.rules_json_pointer != self.game_rules_json_pointer
        {
            return Err("domination_round_game_rules_source_mismatch");
        }
        Ok(Round::new(Rules {
            starting_points: self.starting_points,
            major_percent: self.major_victory_percent.into(),
            end_delay_ms: self.end_game_delay_ms,
        }))
    }
}

impl<W: Write> Engine<W> {
    /// Enable the shared score/result lifecycle for a fixed retail Domination world.
    /// Selected worlds configure an equivalent per-profile setup in their registry.
    pub fn with_domination_round(mut self, setup: Setup) -> Result<Self, &'static str> {
        if self.runtime_application_id != Some(20231)
            || self.active.is_some()
            || !self.peers.is_empty()
            || self.match_epoch.is_some()
            || self.match_profiles.is_some()
            || self.acquisition_round.is_some()
            || self.domination_round.is_some()
        {
            return Err("domination_round_requires_idle_explicit_retail_engine");
        }
        let spawns = self
            .spawn_catalog
            .as_ref()
            .ok_or("domination_round_requires_spawn_catalog")?;
        self.domination_round = Some(setup.round_for(spawns, self.objective_catalog.as_ref())?);
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::Setup;
    use crate::acquisition_round::Phase;
    use crate::acquisition_score_rate::Request;
    use crate::spawn_progression::tests::{engine, ready, reserve};
    use crate::*;

    const SOURCE_SHA: &str = "bad0d7a174a0245ccf53cf4f01b36786ebb8de987139e7ccb4aebcfcccae0204";
    const RULES_POINTER: &str = "/synthetic/rules/1";
    const START: [i16; 2] = [31, 43];
    const END_DELAY: u32 = 211;

    fn inputs() -> (Value, Value) {
        let squads: Vec<_> = (0..32).map(|index| json!({
            "index":index,"team":index/16,"platoon":index%16/4,"squad":index%4,
            "faction":if index >= 16 { Some(0) } else { None },"available":[0],"default_spawn":0,
        })).collect();
        let objective_squads: Vec<_> = (0..32)
            .map(|index| {
                json!({
                    "team":index/16,"squad_index":index,"initial_objective_index":65535,
                    "authored_objectives":[{"object_index":0,"order":2}],
                })
            })
            .collect();
        let source =
            json!({"game_rules_source":{"sha256":SOURCE_SHA,"rules_json_pointer":RULES_POINTER}});
        let spawns = json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":source.to_string(),
            "active":{"map_name":"synthetic_domination","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":2,"auto_assign_teams":true,"attacking_faction":1,"squad_capacity":1,
                "team_capacity":16,"has_bunkers":false,"has_objectives":true,"maximum_points":START},
            "maps":[{"name":"synthetic_domination","owner_faction":0,"spawn_count":1,"squads":squads,
                "locations":[{"index":0,"token":123,"kind":"insertion","position":{"x":1.,"y":2.,"z":3.}}]}],
        });
        let objectives = json!({
            "format":"mag_retail_objective_catalog","schema_version":1,"application_id":20231,
            "protocol":protocol::ProtocolInfo::RETAIL_PROTOCOL,"game_type":2,"map_name":"synthetic_domination",
            "entity_family":12,"network_type":12,
            "source":{"authored_catalog_sha256":SOURCE_SHA,"binding_capture_sha256":"1".repeat(64),
                "startup_proof_sha256":"2".repeat(64),"source_image_sha256":"3".repeat(64),
                "capture_qualification":"native_initial_load",
                "initial_load_scope":"pre_mission_start_before_dynamic_creation"},
            "objects":[{"token":"0x0000007b","object_index":0,"initial_state":0}],"squads":objective_squads,
        });
        (spawns, objectives)
    }

    fn setup_json(engine: &Engine<Vec<u8>>) -> Value {
        json!({
            "format":"mag_domination_round_v1","schema_version":1,
            "spawn_catalog_sha256":engine.spawn_catalog.as_ref().unwrap().source_sha256().unwrap(),
            "authored_objective_catalog_sha256":SOURCE_SHA,"game_rules_sha256":SOURCE_SHA,
            "game_rules_json_pointer":RULES_POINTER,"starting_points":START,
            "major_victory_percent":37,"end_game_delay_ms":END_DELAY,
        })
    }

    fn mode2() -> Engine<Vec<u8>> {
        let mut engine = engine();
        let (spawns, objectives) = inputs();
        engine.spawn_catalog =
            Some(spawn_catalog::SpawnCatalog::from_json(&spawns.to_string()).unwrap());
        engine
            .with_objective_catalog(
                objective_catalog::ObjectiveCatalog::from_json(&objectives.to_string()).unwrap(),
            )
            .unwrap()
    }

    fn setup(engine: &Engine<Vec<u8>>) -> Setup {
        Setup::from_json(&setup_json(engine).to_string()).unwrap()
    }

    fn started(engine: &mut Engine<Vec<u8>>, player: u16, epoch: Duration) {
        reserve(engine, player);
        assert!(
            engine
                .active
                .as_ref()
                .is_some_and(|actor| actor.reservation.player == player)
        );
        for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
            ready(actor);
        }
        engine.match_epoch = Some(epoch);
    }

    #[test]
    fn interactive_domination_requires_connected_backend_before_admission() {
        let mut engine = mode2();
        let (_, mut objectives) = inputs();
        objectives["source"]["capture_qualification"] = json!("native_source_gameplay");
        objectives["source"]
            .as_object_mut()
            .unwrap()
            .remove("initial_load_scope");
        engine.objective_catalog =
            Some(objective_catalog::ObjectiveCatalog::from_json(&objectives.to_string()).unwrap());
        assert_eq!(
            engine.validate_objective_configuration(engine.wire_profile),
            Err("domination_gameplay_configuration_required"),
        );
        let setup = setup(&engine);
        let engine = engine.with_domination_round(setup).unwrap();
        // A score/result clock by itself cannot supply objective entry effects.
        assert_eq!(
            engine.validate_objective_configuration(engine.wire_profile),
            Err("domination_gameplay_configuration_required"),
        );
    }

    #[test]
    fn explicit_setup_requires_exact_loaded_source_and_score() {
        for (field, value) in [
            ("spawn_catalog_sha256", json!("b".repeat(64))),
            ("authored_objective_catalog_sha256", json!("b".repeat(64))),
            ("game_rules_sha256", json!("b".repeat(64))),
            ("game_rules_json_pointer", json!("/synthetic/rules/other")),
            ("starting_points", json!([5, 6])),
        ] {
            let engine = mode2();
            let mut payload = setup_json(&engine);
            payload[field] = value;
            let setup = Setup::from_json(&payload.to_string()).unwrap();
            assert!(engine.with_domination_round(setup).is_err(), "{field}");
        }
        let mut engine = mode2();
        engine.runtime_application_id = None;
        let setup = setup(&engine);
        assert!(engine.with_domination_round(setup).is_err());
    }

    #[test]
    fn domination_wires_initial_timed_depletion_postgame_and_requeue_once() {
        use protocol::ScoreChangeReason::{Initial, TimedBleed};
        let engine = mode2();
        let setup = setup(&engine);
        let mut engine = engine.with_domination_round(setup).unwrap();
        assert!(
            engine
                .tick_match_clock(Duration::from_millis(1), 1)
                .unwrap()
                .is_empty()
        );
        assert!(engine.round.scores.is_none());
        started(&mut engine, 1, Duration::from_millis(100));
        engine
            .tick_match_clock(Duration::from_millis(100), 1)
            .unwrap();
        let initial = engine.drain_notifications(Vec::new(), 1).unwrap();
        let payload = |team, points, reason, profile| {
            protocol::encode_hex(
                &protocol::ScoreChange {
                    team,
                    points,
                    reason,
                }
                .encode_with_profile(profile)
                .unwrap(),
            )
        };
        for team in 0..2 {
            let expected = payload(team, START[team as usize], Initial, engine.wire_profile);
            assert!(initial.iter().any(|notice| notice.payload_hex == expected));
        }
        engine
            .domination_round
            .as_mut()
            .unwrap()
            .rates_mut()
            .configure(
                Request {
                    team: 1,
                    amount: START[1] as u32,
                    frequency_ms: 1000,
                },
                100,
            )
            .unwrap();
        engine
            .tick_match_clock(Duration::from_millis(1101), 2)
            .unwrap();
        let expected = payload(1, 0, TimedBleed, engine.wire_profile);
        let expected_end = protocol::encode_hex(
            &protocol::EndOfGame {
                winning_team: 0,
                major_victory: true,
                end_time_ms: 1101 + END_DELAY,
            }
            .encode_with_profile(engine.wire_profile)
            .unwrap(),
        );
        let end = engine.drain_notifications(Vec::new(), 2).unwrap();
        assert!(end.iter().any(|notice| notice.payload_hex == expected));
        assert_eq!(
            end.iter()
                .filter(|notice| notice.payload_hex == expected_end)
                .count(),
            1
        );
        let first = engine.match_terminal.as_ref().unwrap().match_id.clone();
        let committed = engine.domination_round.clone();
        engine
            .tick_match_clock(Duration::from_millis(1101), 2)
            .unwrap();
        assert_eq!(engine.domination_round, committed);
        let repeat = engine.drain_notifications(Vec::new(), 2).unwrap();
        assert!(
            !repeat
                .iter()
                .any(|notice| notice.payload_hex == expected || notice.payload_hex == expected_end)
        );
        let deadline = Duration::from_millis(u64::from(1101 + END_DELAY));
        assert!(!engine.round_combat_allowed(deadline));
        engine.tick_match_clock(deadline, 3).unwrap();
        assert!(engine.round.outcome.as_ref().unwrap().finalized);
        assert_eq!(
            engine.domination_round.as_ref().unwrap().phase(),
            Phase::Finished
        );
        let release = engine.handle(
            crate::Request::Release {
                v: BRIDGE_VERSION,
                request_id: "release-domination-round".into(),
                reservation_id: "synthetic-reservation-0001".into(),
            },
            deadline,
            3,
        );
        assert!(release.ok, "{:?}", release.error);
        let next_epoch = deadline + Duration::from_millis(1);
        started(&mut engine, 2, next_epoch);
        assert_eq!(
            engine.domination_round.as_ref().unwrap().phase(),
            Phase::Playing
        );
        assert!(engine.match_terminal.is_none());
        assert!(engine.round.scores.is_none());
        let next_update = next_epoch + Duration::from_secs(1);
        engine.tick_match_clock(next_update, 4).unwrap();
        assert_eq!(engine.round.scores, Some(START));
        engine
            .domination_round
            .as_mut()
            .unwrap()
            .apply_score(acquisition_round::ScoreIntent {
                team: 0,
                delta: -START[0],
            })
            .unwrap();
        engine
            .tick_match_clock(next_update + Duration::from_millis(1), 4)
            .unwrap();
        let terminal = engine.match_terminal.as_ref().unwrap();
        assert_ne!(terminal.match_id, first);
        assert_eq!(terminal.reason, "defender_victory");
        assert_eq!(terminal.reservation_ids, ["synthetic-reservation-0002"]);
    }

    #[test]
    fn registry_replay_preserves_the_round_and_fresh_selection_replaces_it() {
        struct Files(std::path::PathBuf);
        impl Files {
            fn write(&self, name: &str, data: &Value) -> std::path::PathBuf {
                let path = self.0.join(name);
                std::fs::write(&path, data.to_string()).unwrap();
                path
            }
        }
        impl Drop for Files {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).unwrap();
            }
        }

        let directory = std::env::temp_dir().join(format!(
            "mag-synthetic-domination-round-{}",
            std::process::id()
        ));
        std::fs::create_dir(&directory).unwrap();
        let files = Files(directory);
        let (spawns, objectives) = inputs();
        let spawn_path = files.write("spawns.json", &spawns);
        let objective_path = files.write("objectives.json", &objectives);
        let setup_payload = setup_json(&mode2());
        let setup_path = files.write("round.json", &setup_payload);
        let manifest = files.write("profiles.json", &json!({
            "format":"mag_match_profiles_v1","profiles":[
                {"profile_id":"scored","spawn_catalog":spawn_path,"objective_catalog":objective_path,
                 "domination_round":setup_path},
                {"profile_id":"entry-only","spawn_catalog":spawn_path,"objective_catalog":objective_path}
            ],
        }));
        let registry = match_profiles::Registry::load(manifest).unwrap();
        let mut engine = engine().with_match_profiles(registry).unwrap();
        engine
            .select_match_profile("first".into(), "scored".into(), 1)
            .unwrap();
        engine
            .domination_round
            .as_mut()
            .unwrap()
            .apply_score(acquisition_round::ScoreIntent { team: 0, delta: -3 })
            .unwrap();
        engine
            .select_match_profile("first".into(), "scored".into(), 2)
            .unwrap();
        assert_eq!(
            engine.domination_round.as_ref().unwrap().points(),
            [START[0] - 3, START[1]]
        );
        engine
            .select_match_profile("second".into(), "entry-only".into(), 3)
            .unwrap();
        assert!(engine.domination_round.is_none());
        engine
            .select_match_profile("third".into(), "scored".into(), 4)
            .unwrap();
        assert_eq!(engine.domination_round.as_ref().unwrap().points(), START);
    }

    #[test]
    fn failed_record_preserves_accepted_round_and_emits_no_partial_outcome() {
        let engine = mode2();
        let setup = setup(&engine);
        let mut engine = engine.with_domination_round(setup).unwrap();
        started(&mut engine, 1, Duration::from_millis(100));
        engine
            .tick_match_clock(Duration::from_millis(100), 1)
            .unwrap();
        engine
            .domination_round
            .as_mut()
            .unwrap()
            .rates_mut()
            .configure(
                Request {
                    team: 1,
                    amount: START[1] as u32,
                    frequency_ms: 1000,
                },
                100,
            )
            .unwrap();
        let prior = engine.domination_round.clone();
        engine.max_log = engine.written;
        assert!(
            engine
                .tick_match_clock(Duration::from_millis(1101), 2)
                .is_err()
        );
        assert_eq!(engine.domination_round, prior);
        assert_eq!(engine.round.scores, Some(START));
        assert!(engine.round.outcome.is_none() && engine.match_terminal.is_none());
    }
}
