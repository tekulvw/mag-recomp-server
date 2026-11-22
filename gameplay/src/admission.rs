//! Team/squad admission from the loaded authored match configuration.
use crate::*;

impl Active {
    pub(super) fn team(&self) -> u8 {
        // SquadCatalog validates the native 0..31 squad address on load/selection.
        (self.squad_catalog.squad_index() / 16) as u8
    }
}
impl<W: Write> Engine<W> {
    pub(super) fn admission_capacity(&self) -> usize {
        self.admission_capacity_for(self.wire_profile.application_id())
    }
    pub(super) fn admission_capacity_for(&self, application_id: u32) -> usize {
        let policy = self.match_profile();
        if application_id == 20231 && matches!(policy.game_type, 0..=3) && policy.auto_assign_teams
        {
            policy.team_capacity * 2
        } else {
            8
        }
    }
    pub(super) fn actor_matches_team(&self, actor: &Active) -> bool {
        let policy = self.match_profile();
        let faction = if actor.team() == 0 {
            policy.attacking_faction
        } else {
            policy.defending_faction
        };
        actor.reservation.character.faction == faction
    }
    pub(super) fn squad_reservation_count(&self, squad: u16) -> usize {
        self.peers
            .values()
            .chain(self.active.iter())
            .filter(|a| a.squad_catalog.squad_index() == squad)
            .count()
    }
    pub(super) fn allocate_squad(
        &self,
        faction: u8,
    ) -> Result<spawn_catalog::SpawnCatalog, &'static str> {
        let catalog = self
            .spawn_catalog
            .as_ref()
            .ok_or("spawn_catalog_unavailable")?;
        let policy = catalog.match_profile();
        if !policy.auto_assign_teams {
            if faction != policy.defending_faction {
                return Err("unsupported_match_character");
            }
            return Ok(catalog.clone());
        }
        let preferred_team = catalog.squad_index() / 16;
        let mut teams = Vec::new();
        let mut faction_allowed = false;
        for team in 0..2_u8 {
            let expected = if team == 0 {
                policy.attacking_faction
            } else {
                policy.defending_faction
            };
            if faction != expected {
                continue;
            }
            faction_allowed = true;
            let count = self
                .peers
                .values()
                .chain(self.active.iter())
                .filter(|a| a.team() == team)
                .count();
            if count < policy.team_capacity {
                teams.push((count, u16::from(team) != preferred_team, team));
            }
        }
        teams.sort_unstable();
        for (_, _, team) in teams {
            if let Some(binding) = catalog
                .map_pool()
                .squads
                .iter()
                .filter(|b| b.team == team)
                .map(|b| (self.squad_reservation_count(b.index), b.index))
                .filter(|(count, _)| *count < policy.squad_capacity)
                .min()
            {
                return catalog.for_squad(binding.1);
            }
        }
        Err(if faction_allowed {
            "match_full"
        } else {
            "unsupported_match_character"
        })
    }
}

pub(crate) struct PreparedAdmission {
    pub(crate) candidate: Reservation,
    pub(crate) modifiers: skill_catalog::OwnedModifiers,
    pub(crate) squad_catalog: spawn_catalog::SpawnCatalog,
    pub(crate) shared_spawn: u16,
    pub(crate) round_number: u64,
}
impl<W: Write> Engine<W> {
    pub(crate) fn validate_admission_character(
        &self,
        character: &Character,
    ) -> Result<skill_catalog::OwnedModifiers, &'static str> {
        let modifiers = match character.skills {
            Some(owned) => self
                .skill_catalog
                .as_ref()
                .ok_or("skill_catalog_unavailable")?
                .resolve(character.faction, &owned)?,
            None => skill_catalog::OwnedModifiers::default(),
        };
        if let Some(saved) = &character.loadouts {
            if character.character_id.is_none_or(|id| id == 0) {
                return Err("invalid_saved_character_id");
            }
            if self.wire_profile.application_id() == 20231
                && let Some(armory) = self.item_catalog.retail_armory()
            {
                self.item_catalog.validate_saved_item_bindings(saved)?;
                armory.validate_saved(character.faction, saved)?;
            } else {
                saved.validate(&self.item_catalog)?;
            }
        }
        Ok(modifiers)
    }
    pub(crate) fn allocate_fresh_squad(
        &self,
        faction: u8,
    ) -> Result<spawn_catalog::SpawnCatalog, &'static str> {
        let mut catalog = self
            .spawn_catalog
            .clone()
            .ok_or("spawn_catalog_unavailable")?;
        catalog.reset_runtime();
        let policy = catalog.match_profile();
        if !policy.auto_assign_teams {
            if faction != policy.defending_faction {
                return Err("unsupported_match_character");
            }
            return Ok(catalog);
        }
        let preferred = catalog.squad_index() / 16;
        let mut teams = (0..2_u8)
            .filter(|t| {
                faction
                    == if *t == 0 {
                        policy.attacking_faction
                    } else {
                        policy.defending_faction
                    }
            })
            .collect::<Vec<_>>();
        if teams.is_empty() {
            return Err("unsupported_match_character");
        }
        teams.sort_unstable_by_key(|t| (u16::from(*t) != preferred, *t));
        for team in teams {
            if let Some(index) = catalog
                .map_pool()
                .squads
                .iter()
                .filter(|b| b.team == team)
                .map(|b| b.index)
                .min()
            {
                return catalog.for_squad(index);
            }
        }
        Err("match_full")
    }
    pub(crate) fn install_reserved(&mut self, prepared: PreparedAdmission, now: Duration) {
        let PreparedAdmission {
            candidate,
            modifiers,
            squad_catalog,
            shared_spawn,
            round_number,
        } = prepared;
        self.round_number = round_number;
        let reward_match_id = self
            .reward_match_id
            .get_or_insert_with(|| progression::round_identity(&candidate, self.round_number))
            .clone();
        let participation = progression::participant_key(&candidate)
            .and_then(|key| self.round_participants.get(&key))
            .cloned()
            .unwrap_or_default();
        let nominations = self.initial_nominations(&candidate.character);
        let leadership_level = self
            .leadership_record(&candidate.character)
            .or(candidate.character.leadership)
            .map_or(1, |record| record.leadership_level);
        self.active = Some(Active {
            allocation_sequence: self.sequence,
            wire_profile: self.wire_profile,
            reservation: candidate,
            modifiers,
            leadership: None,
            leadership_level,
            nominations,
            last_event: now,
            claimed: None,
            ready_for_updates: false,
            initial_objects: None,
            spawn_delta: None,
            life: CharacterLife::AwaitingFirstSpawn,
            revive_state_pending: false,
            tracking: player_tracking::PlayerTracking::default(),
            fall_tracker: fall_damage::Tracker::default(),
            pending_damage: None,
            last_damage: replication::CharacterDamage {
                last_location_and_type: 0,
                health_quarter_points: 400,
                conditions: 0,
                last_attacker: 0,
                last_source_object: 0,
                last_user_data: 0,
            },
            healing: None,
            healing_sequence: 0,
            reward_match_id,
            match_experience: participation.match_experience,
            reward_statistics: participation.statistics,
            repair_credit: participation.repair_credit,
            next_heal_allowed: Duration::ZERO,
            projectiles: projectile::ProjectileFlights::default(),
            hospital_rocket_flights: Default::default(),
            infantry_rocket_state: Default::default(),
            projectile_replicas: projectile_replication::ProjectileReplicas::with_profile(
                self.wire_profile,
            ),
            projectile_cleanup: Default::default(),
            projectile_publications: Vec::new(),
            grenade_remaining: 0,
            static_resupply: Vec::new(),
            observed_primary: None,
            observed_secondary: None,
            primary_equipped: true,
            dummy_created: false,
            development_target: None,
            target_commands: std::collections::BTreeMap::new(),
            dummy_health: None,
            latest_owner_sequence: 0,
            grenade_damage: std::collections::VecDeque::new(),
            damage_commands: std::collections::BTreeMap::new(),
            replication_history: Vec::new(),
            outbound_sequence: 0,
            pending_shared: std::collections::VecDeque::new(),
            shared_frames_drained: 0,
            statistics_cursor: 0,
            prefer_statistics: false,
            pending_firearm_assist_rows: Default::default(),
            hospital_bootstrap: 0..0,
            ordered_health_pending: false,
            replication_backlogged: false,
            objective_delta: None,
            objective_assigned: false,
            objective_needs_clear: false,
            emplacement_creation: None,
            ground_vehicle_batch: 0,
            mounted_seat: None,
            mounted_bunker: None,
            ground_seat: None,
            helo_seat: None,
            helo_creation_sent: false,
            seat_transitions: Vec::new(),
            last_exit: None,
            match_clock_started: None,
            last_clock_points: self.match_profile().maximum_points[0],
            round_scores: None,
            pending_notifications: Default::default(),
            end_announced: false,
            results_announced: false,
            postgame_announced: false,
            current_inventory_handle: None,
            spawned_loadout: None,
            selected_loadout: None,
            loadout_selection_revision: 0,
            last_loadout_refresh: None,
            pending_loadout_selection: None,
            selected_spawn: None,
            squad_spawn: shared_spawn,
            squad_catalog: std::sync::Arc::new(squad_catalog),
            pending_spawn_deadline: None,
            published_wave_deadline: None,
            entered_game: false,
            owner_fields: player_tracking::ClientCharacterFields::default(),
        });
    }
}
