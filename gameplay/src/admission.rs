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
        if application_id == 20231 && matches!(policy.game_type, 0 | 3) && policy.auto_assign_teams
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
