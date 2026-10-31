//! Transactional ordinary on-foot retail firearm death notifications.
use super::*;

// TODO: grenade/fall/kill-self, source type11, incapacitation and vehicle
// deaths need their own original metadata/lifecycle authority before admission.

#[derive(Clone, Debug, PartialEq, Eq)]
struct Participant {
    id: u16,
    generation: u32,
    reservation: Option<String>,
    world: u32,
    squad: u16,
    health: u16,
}

/// Owned and consumed exactly at the successful terminal damage commit.
/// No clone/default and no post-death constructor: a duplicate must pass the
/// existing living-generation damage guard again before it can publish.
pub(super) struct Plan {
    attacker: Participant,
    victim: Participant,
    wire: [u8; protocol::RetailDeathNotification::ENCODED_LEN],
    item: u16,
    location: u8,
}

fn firearm_source(global_equipment_index: u16) -> (u8, i32) {
    // Retail HandleMessageHit 0x7def44..0x7def64: Weapon(2), followed
    // by sign-extended SimInventoryItem::globalEqIdx (int16 at +0x90).
    // DEV DIE0x04e08492 places SimWeaponKit::m_inventory at +4;
    // DIE0x04e04d48 names its SimInventoryItem vector, and DIE0x04e047ca
    // names globalEqIdx. Retail uses that +4 base, 0xa0 stride and +0x90.
    // See evidence/retail-bullet-native-20261026.json for the retail hit block.
    // These are independent of damage kind, source object and client userdata.
    (2, i32::from(global_equipment_index as i16))
}
fn head_flag(
    catalog: &retail_combat::RetailCombatCatalog,
    location: u8,
) -> Result<protocol::HeadRegionHit, &'static str> {
    let body = catalog
        .body_for_damage(location)
        .map_err(|_| "unsupported_death_body")?;
    Ok(if body.armor_area == 0 {
        protocol::HeadRegionHit::Head
    } else {
        protocol::HeadRegionHit::NonHead
    })
}

impl Plan {
    pub(super) fn victim_matches(
        &self,
        id: u16,
        generation: u32,
        reservation: Option<&str>,
    ) -> bool {
        self.victim.id == id
            && self.victim.generation == generation
            && self.victim.reservation.as_deref() == reservation
    }
    pub(super) fn audit(&self) -> Value {
        let (source_type, source_index) = firearm_source(self.item);
        json!({"attacker":self.attacker.id,"attacker_generation":self.attacker.generation,
            "attacker_reservation":self.attacker.reservation,"defender":self.victim.id,
            "defender_generation":self.victim.generation,"defender_reservation":self.victim.reservation,
            "world_id":self.victim.world,"participant_squads":[self.attacker.squad,self.victim.squad],
            "damage_source_type":source_type,"damage_source_index":source_index,"weapon_item":self.item,
            "hit_location":self.location,"head_region":self.wire[11]&0x80!=0,"victim_was_mounted":self.wire[11]&0x40!=0,
            "payload_hex":protocol::encode_hex(&self.wire)})
    }
}
impl<W: Write> Engine<W> {
    fn death_participant(&self, id: u16, world: u32) -> Option<Participant> {
        if let Some(bot) = self.bot_registry.bots.get(&id) {
            let runtime = bot.target.runtime?;
            if bot.health == 0 || matches!(runtime.phase, development_target::BotPhase::Dead) {
                return None;
            }
            return Some(Participant {
                id,
                generation: runtime.generation,
                reservation: None,
                world,
                squad: bot.target.squad.squad_index(),
                health: bot.health,
            });
        }
        let human = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|a| a.reservation.player == id && a.reservation.world == world)?;
        if !human.ready_for_updates
            || !human.entered_game
            || human.initial_objects.is_none()
            || human.mounted_seat.is_some()
        {
            return None;
        }
        let CharacterLife::Alive {
            generation,
            health_quarter_points,
        } = human.life
        else {
            return None;
        };
        if health_quarter_points == 0 {
            return None;
        }
        Some(Participant {
            id,
            generation,
            reservation: Some(human.reservation.id.clone()),
            world,
            squad: human.squad_catalog.squad_index(),
            health: health_quarter_points,
        })
    }

    pub(super) fn plan_firearm_death(
        &self,
        world: u32,
        attacker: u16,
        victim: u16,
        item: u16,
        location: u8,
        damage: replication::CharacterDamage,
    ) -> Result<Option<Plan>, &'static str> {
        if self.wire_profile.application_id() != 20231
            || damage.health_quarter_points != 0
            || attacker == victim
        {
            return Ok(None);
        }
        let Some(attacker) = self.death_participant(attacker, world) else {
            return Ok(None);
        };
        let Some(victim) = self.death_participant(victim, world) else {
            return Ok(None);
        };
        let catalog = self.item_catalog.retail_combat()?;
        catalog
            .bullet_for_damage(item)
            .map_err(|_| "unsupported_death_firearm")?;
        let (damage_source_type, damage_source_index) = firearm_source(item);
        let wire = protocol::RetailDeathNotification {
            attacker: Some(attacker.id),
            defender: victim.id,
            damage_source_type,
            damage_source_index,
            head_region_hit: head_flag(catalog, location)?,
            // Both actual participants were admitted on foot before death clears
            // mount state. No vehicle/seat-only inference is admitted here.
            victim_was_mounted: protocol::VictimWasMounted::NoAssociatedVehicle,
        }
        .encode()?;
        Ok(Some(Plan {
            attacker,
            victim,
            wire,
            item,
            location,
        }))
    }

    /// Caller must record Plan::audit before invoking this method. Existing
    /// human/bot terminal guards are the authority for exactly-once publication.
    pub(super) fn commit_firearm_death(
        &mut self,
        plan: Plan,
        damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(), &'static str> {
        if damage.health_quarter_points != 0
            || self
                .death_participant(plan.attacker.id, plan.attacker.world)
                .as_ref()
                != Some(&plan.attacker)
            || self
                .death_participant(plan.victim.id, plan.victim.world)
                .as_ref()
                != Some(&plan.victim)
        {
            return Err("stale_firearm_death");
        }
        if let Some(reservation) = &plan.victim.reservation {
            let victim = self
                .active
                .as_mut()
                .filter(|a| a.reservation.id == *reservation)
                .or_else(|| self.peers.get_mut(reservation))
                .ok_or("stale_firearm_death")?;
            victim.apply_peer_damage(plan.victim.generation, damage, now)?;
        } else {
            self.commit_bot_damage(plan.victim.id, plan.victim.generation, damage, now)?;
        }
        // Current human members of the union of the two actual squads. Iterating
        // actual sessions once deduplicates identical squads and excludes bots.
        // Application publications never enter ObjectUpdate history/rebasing;
        // the existing drain records outgoing bytes again before delivery.
        for human in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|a| {
                a.ready_for_updates
                    && a.initial_objects.is_some()
                    && a.reservation.world == plan.victim.world
                    && [plan.attacker.squad, plan.victim.squad]
                        .contains(&a.squad_catalog.squad_index())
            })
        {
            if human.pending_notifications.len() >= 128 {
                human.replication_backlogged = true;
            } else {
                human
                    .pending_notifications
                    .push_back(match_flow::Publication::Application(plan.wire.to_vec()));
            }
        }
        Ok(())
    }
}
