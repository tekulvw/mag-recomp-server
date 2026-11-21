//! Apply durable character state settled outside this gameplay cohort.

use crate::{match_flow::Publication, progression::ProgressionSnapshot, *};

impl<W: Write> Engine<W> {
    pub(super) fn refresh_progression(
        &mut self,
        request: Request,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let Request::RefreshProgression {
            reservation_id,
            world_id,
            player_id,
            account_id,
            character_id,
            progression: snapshot,
            leadership,
            ..
        } = request
        else {
            return Err("wrong_progression_refresh_operation");
        };
        if self.failed {
            return Err("recorder_unavailable");
        }
        let current_actors = self.active.iter().chain(self.peers.values());
        let actor = current_actors
            .clone()
            .find(|actor| actor.reservation.id == reservation_id)
            .filter(|_| !self.released_reservations.contains(&reservation_id))
            .ok_or("refresh_progression_owner_unavailable")?;
        let character = &actor.reservation.character;
        if actor.reservation.world != world_id
            || actor.reservation.player != player_id
            || character.account_id.as_deref() != Some(&account_id)
            || character.character_id != Some(character_id)
        {
            return Err("refresh_progression_identity_mismatch");
        }
        if now.saturating_sub(actor.last_event) >= IDLE_TIMEOUT {
            return Err("refresh_progression_reservation_expired");
        }
        let current = character
            .progression
            .ok_or("refresh_progression_owner_unavailable")?;
        let progression = if snapshot.experience > current.experience {
            ProgressionSnapshot {
                level: snapshot.level.max(current.level),
                ..snapshot
            }
        } else {
            current
        };
        let leadership = self
            .committed_leadership(
                actor,
                leadership,
                snapshot.experience,
                self.match_profile().game_type,
            )
            .filter(|update| {
                Some(update.record) != character.leadership
                    || update.record.leadership_level != actor.leadership_level
                    || update.nominations != actor.nominations
            });
        if progression == current && leadership.is_none() {
            return Ok(Vec::new());
        }

        let leadership_changed = leadership.is_some_and(|update| {
            update.record.leadership_level != actor.leadership_level
                || update.nominations != actor.nominations
        });
        let level = if actor.initial_objects.is_some()
            && (progression.level != current.level || leadership_changed)
        {
            let mut mag_info = actor.mag_info(progression.level);
            if let Some(update) = leadership {
                mag_info.leadership_level = update.record.leadership_level;
                mag_info.nominations = update.nominations;
            }
            Some(
                replication::CharacterLevelUpdate {
                    sequence: 1,
                    timestamp_ms: now.as_millis() as u32,
                    object_index: player_id,
                    mag_info,
                }
                .encode_with_profile(actor.wire_profile)?,
            )
        } else {
            None
        };
        let stat = if progression.experience != current.experience {
            Some(
                protocol::StatUpdate {
                    character_index: player_id,
                    stats: vec![protocol::StatValue {
                        stat: 1,
                        value: progression.experience as i32,
                    }],
                }
                .encode_with_profile(actor.wire_profile)?,
            )
        } else {
            None
        };

        // Existing reliable FIFOs assign recipient sequences and deliver
        // application updates after older shared replication.
        let cohort = actor.reward_match_id.clone();
        let sees_level = |recipient: &Active| {
            recipient.reservation.world == world_id
                && recipient.reward_match_id == cohort
                && recipient.initial_objects.is_some()
        };
        let stat_overflow = stat.is_some()
            && (actor.replication_backlogged
                || actor.pending_notifications.len() >= match_flow::MAX_PENDING_NOTIFICATIONS);
        let level_overflow = level.is_some()
            && current_actors
                .filter(|actor| sees_level(actor))
                .any(|actor| {
                    actor.replication_backlogged
                        || actor.ordinary_shared_len() >= multiplayer::MAX_SHARED_QUEUE
                });
        if stat_overflow || level_overflow {
            return Err("refresh_progression_publications_full");
        }

        self.record(
            json!({"op":"progression_refreshed","reservation_id":reservation_id,
                "world_id":world_id,"player_id":player_id,"account_id":account_id,"character_id":character_id,
                "progression":progression,"leadership":leadership.map(|update| update.record)}),
            wall,
        )?;
        let mut stat = stat;
        for actor in self.active.iter_mut().chain(self.peers.values_mut()) {
            if sees_level(actor)
                && let Some(packet) = &level
            {
                actor.queue_shared(packet.clone());
            }
            if actor.reservation.id == reservation_id {
                actor.reservation.character.progression = Some(progression);
                if let Some(update) = leadership {
                    actor.install_committed_leadership(update);
                }
                if let Some(packet) = stat.take() {
                    actor.queue_round(Publication::Application(packet));
                }
            }
        }
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        extended_objective_reward_tests::{callback, fixture},
        leadership_runtime::LeadershipRecord,
        match_flow::Publication,
        progression::{ProgressionSnapshot, RewardKind, RewardRecipient},
        replication::ObjectiveState,
        spawn_progression::{engine, ready},
        *,
    };

    const OWNER: &str = "synthetic-refresh-owner";
    const PEER: &str = "synthetic-refresh-peer";
    const WORLD: u32 = 2;
    const ACCOUNT: &str = "synthetic-durable-account";
    const CHARACTER: u32 = 55;
    const NOW: Duration = Duration::from_secs(1);

    fn actor<'a>(engine: &'a Engine<Vec<u8>>, id: &str) -> &'a Active {
        engine
            .peers
            .get(id)
            .or_else(|| engine.active.as_ref().filter(|a| a.reservation.id == id))
            .unwrap()
    }

    fn actor_mut<'a>(engine: &'a mut Engine<Vec<u8>>, id: &str) -> &'a mut Active {
        engine
            .peers
            .get_mut(id)
            .or_else(|| engine.active.as_mut().filter(|a| a.reservation.id == id))
            .unwrap()
    }

    fn bind_character(actor: &mut Active) {
        let character = &mut actor.reservation.character;
        character.account_id = Some(ACCOUNT.into());
        character.character_id = Some(CHARACTER);
        character.progression = Some(ProgressionSnapshot::default());
    }

    fn current() -> Engine<Vec<u8>> {
        let mut engine = engine();
        for (id, slot) in [(OWNER, 0), (PEER, 1)] {
            let mut character =
                json!({"name":"Synthetic Player","faction":1,"head":0,"voice":0,"universe_id":26});
            if slot == 0 {
                character["account_id"] = json!(ACCOUNT);
                character["character_id"] = json!(CHARACTER);
                character["progression"] = json!(ProgressionSnapshot::default());
            }
            let reserved = dispatch(
                &mut engine,
                json!({
                    "op":"reserve","v":BRIDGE_VERSION,"request_id":"synthetic-new-reservation",
                    "reservation_id":id,"world_id":WORLD,"player_id":slot,"account_id":slot+1,
                    "session_key":"aaaaaaaaaaaaaaaa","access_key":"bbbbbbbbbbbbbbbb",
                    "character":character
                }),
            );
            assert!(reserved.ok, "{:?}", reserved.error);
        }
        let owner = actor_mut(&mut engine, OWNER);
        ready(owner);
        owner.match_experience = 11;
        owner.reward_statistics.insert(42, 4);
        ready(actor_mut(&mut engine, PEER));
        engine.enable_world_ticks();
        engine
    }

    fn refresh(experience: u32, level: u8) -> Value {
        json!({
            "op":"refresh_progression","v":BRIDGE_VERSION,"request_id":"synthetic-refresh",
            "reservation_id":OWNER,"world_id":WORLD,"player_id":0,
            "account_id":ACCOUNT,"character_id":CHARACTER,
            "progression":{"experience":experience,"level":level,"skill_points":1}
        })
    }

    fn dispatch(engine: &mut Engine<Vec<u8>>, request: Value) -> Response {
        engine.handle(
            parse_request(request.to_string().as_bytes()).unwrap(),
            NOW,
            2,
        )
    }

    #[test]
    fn old_worker_settlement_refreshes_only_durable_state_of_the_new_cohort() {
        let mut earlier = fixture(3, 4);
        bind_character(earlier.active.as_mut().unwrap());
        let mut latest = current();
        callback(
            &mut earlier,
            ObjectiveState::Activated,
            ObjectiveState::Completed,
        )
        .unwrap();
        let reward = earlier.pending_reward_batch()[0].clone();
        let original_match = reward.match_id.clone();
        assert_ne!(
            latest.reward_match_id.as_deref(),
            Some(original_match.as_str())
        );
        let mut settlement = json!({
            "op":"progression_committed","v":BRIDGE_VERSION,"request_id":"synthetic-old-settlement",
            "reservation_id":reward.reservation_id,"world_id":reward.world_id,"player_id":reward.player_id,
            "reward_id":reward.reward_id,"progression":{"experience":90,"level":3,"skill_points":1},
            "match_experience":90
        });
        if reward.frago == Some(true) {
            settlement["match_frago_points"] = json!(20);
        }
        let settled = dispatch(&mut earlier, settlement);
        assert!(settled.ok, "{:?}", settled.error);
        assert!(earlier.committed_rewards.contains_key(&reward.reward_id));

        let resident_reward = RewardRecipient::from_active(actor(&latest, OWNER))
            .unwrap()
            .event(
                "synthetic-current-earned-reward".into(),
                RewardKind::EnemyKilled,
                1,
            );
        latest.commit_rewards(vec![resident_reward.clone()]);
        let recorded = latest.sequence;
        let mut refresh = refresh(0, 1);
        refresh["progression"] = json!(earlier.committed_rewards[&reward.reward_id].progression);
        let response = dispatch(&mut latest, refresh);
        assert!(response.ok, "{:?}", response.error);
        assert!(response.messages.is_empty() && response.rewards.is_empty());
        assert_eq!(latest.sequence, recorded + 1);
        assert_eq!(latest.pending_reward_batch(), [resident_reward]);
        assert!(latest.committed_rewards.is_empty());
        assert!(latest.round_participants.is_empty());

        let owner = actor(&latest, OWNER);
        assert_eq!(
            owner.reservation.character.progression.unwrap().experience,
            90
        );
        assert_eq!(owner.reservation.character.progression.unwrap().level, 3);
        assert_eq!(owner.match_experience, 11);
        assert_eq!(owner.reward_statistics, [(42, 4)].into_iter().collect());
        assert_eq!(actor(&latest, PEER).reservation.character.progression, None);
        let messages = latest.drain_shared(Vec::new(), 2).unwrap();
        let stat1 = messages
            .iter()
            .filter(|m| m.payload_hex.starts_with("0204"))
            .collect::<Vec<_>>();
        assert_eq!(stat1.len(), 1);
        assert_eq!(stat1[0].reservation_id, OWNER);
        assert_eq!(stat1[0].payload_hex, "0204000001015a");
        let level_recipients: std::collections::BTreeSet<_> = messages
            .iter()
            .filter(|m| !m.payload_hex.starts_with("0204"))
            .map(|m| m.reservation_id.as_str())
            .collect();
        assert_eq!(level_recipients, [OWNER, PEER].into_iter().collect());
    }

    #[test]
    fn exact_owner_refresh_is_monotonic_and_retries_are_idempotent() {
        let mut engine = current();
        let mut snapshot = refresh(90, 3);
        snapshot["leadership"] = json!({"leadership_level":3,"nominations":0,
            "matches_since_last_command":2,"leadership_points":29});
        assert!(dispatch(&mut engine, snapshot.clone()).ok);
        let sequence = engine.sequence;
        assert!(dispatch(&mut engine, snapshot).ok);
        let mut stale = refresh(30, 2);
        stale["leadership"] = json!({"leadership_level":2,"nominations":0,
            "matches_since_last_command":6,"leadership_points":28});
        assert!(dispatch(&mut engine, stale).ok);
        assert_eq!(engine.sequence, sequence);
        let owner = actor(&engine, OWNER);
        assert_eq!(
            owner.reservation.character.progression.unwrap().experience,
            90
        );
        assert_eq!(owner.pending_shared.len(), 1);
        assert_eq!(owner.pending_notifications.len(), 1);
        assert_eq!(
            owner.reservation.character.leadership,
            Some(LeadershipRecord {
                leadership_level: 3,
                nominations: 0,
                matches_since_last_command: 2,
                leadership_points: Some(29),
            })
        );
    }

    #[test]
    fn refresh_requires_exact_current_identity_and_bounded_schema() {
        let mut engine = current();
        let sequence = engine.sequence;
        for (field, wrong) in [
            ("reservation_id", json!("synthetic-retired-owner")),
            ("world_id", json!(3)),
            ("player_id", json!(1)),
            ("account_id", json!("synthetic-unrelated-account")),
            ("character_id", json!(56)),
        ] {
            let mut request = refresh(90, 3);
            request[field] = wrong;
            assert!(!dispatch(&mut engine, request).ok, "{field}");
        }
        assert_eq!(engine.sequence, sequence);
        assert_eq!(
            actor(&engine, OWNER).reservation.character.progression,
            Some(Default::default())
        );
        for (field, value) in [
            ("match_experience", json!(31)),
            ("reward_id", json!("synthetic-reward")),
            ("account_id", json!("x".repeat(33))),
            ("character_id", json!(0)),
            (
                "progression",
                json!({"experience":0,"level":0,"skill_points":0}),
            ),
        ] {
            let mut request = refresh(90, 3);
            request[field] = value;
            assert!(
                parse_request(request.to_string().as_bytes()).is_err(),
                "{field}"
            );
        }
    }

    #[test]
    fn expired_and_released_owners_cannot_be_refreshed() {
        let mut engine = current();
        let recorded = engine.sequence;
        let expired = engine.handle(
            parse_request(refresh(90, 3).to_string().as_bytes()).unwrap(),
            NOW + IDLE_TIMEOUT,
            3,
        );
        assert_eq!(
            expired.error.as_deref(),
            Some("refresh_progression_reservation_expired")
        );
        assert_eq!(engine.sequence, recorded);

        let released = dispatch(
            &mut engine,
            json!({
                "op":"release","v":BRIDGE_VERSION,"request_id":"synthetic-refresh-release",
                "reservation_id":OWNER,
            }),
        );
        assert!(released.ok, "{:?}", released.error);
        assert!(engine.released_reservations.contains(OWNER));
        let recorded = engine.sequence;
        let unavailable = dispatch(&mut engine, refresh(90, 3));
        assert_eq!(
            unavailable.error.as_deref(),
            Some("refresh_progression_owner_unavailable")
        );
        assert_eq!(engine.sequence, recorded);
    }

    #[test]
    fn queue_or_recorder_failure_leaves_progression_and_publications_unchanged() {
        let mut engine = current();
        let recorded = engine.sequence;
        engine.last_wall_secs = 1;
        let wall = engine.last_wall_secs;
        let peer = actor_mut(&mut engine, PEER);
        peer.pending_shared
            .resize(multiplayer::MAX_SHARED_QUEUE, Vec::new());
        assert!(!dispatch(&mut engine, refresh(90, 3)).ok);
        actor_mut(&mut engine, PEER).pending_shared.clear();
        let owner = actor_mut(&mut engine, OWNER);
        for _ in 0..128 {
            owner.queue_round(Publication::Application(Vec::new()));
        }
        assert!(!dispatch(&mut engine, refresh(90, 3)).ok);
        let owner = actor_mut(&mut engine, OWNER);
        assert_eq!(owner.pending_notifications.len(), 128);
        assert!(!owner.replication_backlogged);
        owner.pending_notifications.clear();
        engine.max_log = engine.written;
        let response = dispatch(&mut engine, refresh(90, 3));
        assert_eq!(response.error.as_deref(), Some("log_limit_reached"));

        let owner = actor(&engine, OWNER);
        assert_eq!(engine.sequence, recorded);
        assert_eq!(engine.last_wall_secs, wall);
        assert_eq!(
            owner.reservation.character.progression,
            Some(Default::default())
        );
        assert!(owner.pending_shared.is_empty());
        assert!(owner.pending_notifications.is_empty());
        assert!(actor(&engine, PEER).pending_shared.is_empty());
    }
}
