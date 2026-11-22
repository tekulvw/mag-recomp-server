//! Immediate character effects for Acquisition world callbacks.
//!
//! Planning is read-only. The caller records the complete outer callback before
//! committing a plan on its owned candidate. No owner-update damage queue, audit
//! write, service, or network delivery is performed here. Publication queues are
//! the normal reliable Engine queues and must be installed with the candidate.
use super::*;
use acquisition_damage_metadata as metadata;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Target {
    pub character_id: u32,
    pub generation: u32,
    pub world: u32,
}
/// Values must come from the owned, current character state. Absence is an
/// explicit unsupported input, never an implicit unprotected/on-foot default.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NativeState {
    pub invulnerability: Option<u8>,
    pub associated_vehicle: Option<protocol::VictimWasMounted>,
}
#[derive(Clone, Copy, Debug)]
pub enum Request {
    VolumeLethal {
        victim_vector: [f32; 3],
    },
    VehicleDeathOccupant {
        last_attacker: u32,
        vehicle_vector: [f32; 3],
    },
    /// A qualified radial target after geometry, seat visibility and falloff.
    VehicleExplosion {
        raw_hp: f32,
        from_direction: [f32; 3],
        attacker: u32,
    },
    /// A Structural blast target after authored radius and material tracing.
    StructuralExplosion {
        raw_hp: f32,
        from_direction: [f32; 3],
        attacker: u32,
        objective_id: u32,
    },
    /// A qualified on-foot Character dose from an authoritative rocket.
    HospitalRocketExplosion {
        raw_hp: f32,
        from_direction: [f32; 3],
        attacker: u32,
        projectile_id: u32,
    },
    /// A leadership strike target after radial falloff and occlusion.
    AirStrikeExplosion {
        raw_hp: f32,
        from_direction: [f32; 3],
        attacker: u32,
    },
    /// A qualified chemical damage pulse from a gas volume.
    AirStrikeGas {
        raw_hp: f32,
        from_direction: [f32; 3],
        attacker: u32,
    },
    /// A qualified target in the Structural objective's periodic damage scan.
    StructuralArea {
        raw_hp: f32,
        from_direction: [f32; 3],
    },
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    DeadIgnored,
    Protected,
    NoDamage,
    Applied,
}

#[derive(PartialEq)]
enum LifeIdentity {
    Alive,
    Incapacitated(incapacitation::Downed),
    Dead,
    Bot(u8),
}
#[derive(PartialEq)]
struct Snapshot {
    reservation: Option<String>,
    health: u16,
    life: LifeIdentity,
    last_damage: (u8, u16, u8, u32, u32, u32),
    mounted: bool,
}
fn damage_identity(d: replication::CharacterDamage) -> (u8, u16, u8, u32, u32, u32) {
    (
        d.last_location_and_type,
        d.health_quarter_points,
        d.conditions,
        d.last_attacker,
        d.last_source_object,
        d.last_user_data,
    )
}
/// A single collector-pair effect. Consume in collector order, re-planning the
/// next pair against the already-mutated candidate, including duplicate pairs.
/// This is not an atomic clone of Engine or an account-storage transaction.
pub struct Plan {
    target: Target,
    expected: Snapshot,
    native_before: NativeState,
    native_after: NativeState,
    now: Duration,
    outcome: Outcome,
    damage: Option<replication::CharacterDamage>,
    life: Option<CharacterLife>,
    packet: Option<Vec<u8>>,
    consequences: Option<events::CharacterEffectConsequences>,
    notice: Option<death_notifications::CharacterEffectNotice>,
    audit: Value,
    clear_healing: bool,
}
impl Plan {
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }
    pub fn audit(&self) -> &Value {
        &self.audit
    }
    pub fn native_after(&self) -> NativeState {
        self.native_after
    }
}
/// The callback passes normalized displacement to the direction encoder with
/// exponent 7. Each axis uses 9 magnitude bits and one sign bit.
pub(crate) fn explosion_direction_user_data(vector: [f32; 3]) -> Result<u32, &'static str> {
    let length = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
    if !length.is_finite() {
        return Err("invalid_acquisition_explosion_direction");
    }
    if length == 0. {
        return Ok(0);
    }
    let [x, y, z] = vector.map(|axis| {
        let magnitude = ((128. * (axis / length)).abs() as u32).min(511);
        magnitude | (u32::from(axis < 0.) << 9)
    });
    Ok((x << 20) | (y << 10) | z)
}

impl<W: Write> Engine<W> {
    fn acquisition_character_snapshot(&self, target: Target) -> Result<Snapshot, &'static str> {
        if target.character_id & 0xffff_0000 != 0x0380_0000 || target.generation == 0 {
            return Err("invalid_acquisition_character_identity");
        }
        if let Some(actor) = self.active.iter().chain(self.peers.values()).find(|a| {
            a.reservation.world == target.world
                && (0x0380_0000 | u32::from(a.reservation.player)) == target.character_id
        }) {
            if !actor.entered_game || !actor.ready_for_updates || actor.initial_objects.is_none() {
                return Err("acquisition_character_not_in_world");
            }
            let generation = match actor.life {
                CharacterLife::Alive { generation, .. }
                | CharacterLife::Incapacitated { generation, .. }
                | CharacterLife::AwaitingRespawn { generation } => generation,
                _ => return Err("acquisition_character_has_no_life"),
            };
            if generation != target.generation {
                return Err("stale_acquisition_character_generation");
            }
            return Ok(Snapshot {
                reservation: Some(actor.reservation.id.clone()),
                health: actor.life.damage_health().map_or(0, |(_, h)| h),
                life: match actor.life {
                    CharacterLife::Alive { .. } => LifeIdentity::Alive,
                    CharacterLife::Incapacitated { downed, .. } => {
                        LifeIdentity::Incapacitated(downed)
                    }
                    _ => LifeIdentity::Dead,
                },
                last_damage: damage_identity(actor.last_damage),
                mounted: actor.is_mounted(),
            });
        }
        let bot = self
            .bot_registry
            .bots
            .get(&(target.character_id as u16))
            .ok_or("acquisition_character_missing")?;
        if !self.accepts_subscription_world(target.world) {
            return Err("acquisition_character_world_mismatch");
        }
        let runtime = bot.target.runtime.ok_or("bot_runtime_missing")?;
        if runtime.generation != target.generation {
            return Err("stale_acquisition_character_generation");
        }
        Ok(Snapshot {
            reservation: None,
            health: bot.health,
            life: LifeIdentity::Bot(match runtime.phase {
                development_target::BotPhase::Moving => 0,
                development_target::BotPhase::Dead => 1,
                development_target::BotPhase::Complete => 2,
                development_target::BotPhase::Relocating => 3,
            }),
            last_damage: damage_identity(bot.damage),
            mounted: false,
        })
    }

    pub(crate) fn plan_acquisition_character_effect(
        &self,
        target: Target,
        request: Request,
        native: NativeState,
        now: Duration,
    ) -> Result<Plan, &'static str> {
        self.plan_character_effect_with_bot_origin(target, request, native, now, None)
    }

    pub(crate) fn plan_character_effect_with_bot_origin(
        &self,
        target: Target,
        request: Request,
        native: NativeState,
        now: Duration,
        bot_plant: Option<crate::objective_control::BotPlantSource>,
    ) -> Result<Plan, &'static str> {
        if bot_plant
            .is_some_and(|origin| origin.world != target.world || origin.round != self.round_number)
        {
            return Err("stale_bot_plant_character_source");
        }
        if self.wire_profile.application_id() != 20231 {
            return Err("unsupported_acquisition_character_profile");
        }
        let expected = self.acquisition_character_snapshot(target)?;
        let initial_invulnerability = native
            .invulnerability
            .ok_or("acquisition_character_invulnerability_missing")?;
        let rocket = matches!(request, Request::HospitalRocketExplosion { .. });
        let source_object = match request {
            Request::StructuralExplosion { objective_id, .. }
                if objective_id & 0xffff_0000 == 0x0c80_0000 =>
            {
                objective_id
            }
            Request::StructuralExplosion { .. } => {
                return Err("invalid_structural_blast_source_object");
            }
            Request::HospitalRocketExplosion {
                attacker,
                projectile_id,
                ..
            } => {
                let expected_index = 0x0080_0000 | (attacker & 0xffff);
                if attacker & 0xffff_0000 != 0x0380_0000
                    || !(16..=19).contains(&(projectile_id >> 24))
                    || projectile_id & 0x00ff_ffff != expected_index
                {
                    return Err("invalid_hospital_rocket_character_source");
                }
                if expected.mounted
                    || native.associated_vehicle
                        != Some(protocol::VictimWasMounted::NoAssociatedVehicle)
                {
                    return Err("hospital_rocket_character_mounted");
                }
                projectile_id
            }
            _ => u32::MAX,
        };
        let (
            raw,
            kind,
            wire_damage_type,
            attacker,
            source_type,
            source_index,
            from_direction,
            user_data,
            bypass,
        ) = match request {
            Request::VolumeLethal { victim_vector } => {
                let damage = metadata::Metadata::from_symbols(
                    "DamageType.ExternalKinetic",
                    "DamageSourceType.eDamageSourceTypeCrushed",
                )?
                .character_lethal(
                    target.character_id,
                    victim_vector,
                    initial_invulnerability,
                )?;
                (
                    armor::RawCharacterDamage {
                        amount_hp: damage.amount_hp,
                        armor_rating_redux: damage.armor_reduction[0],
                        armor_prevention_redux: damage.armor_reduction[1],
                        hit_location: damage.location,
                    },
                    damage::DamageKind::ExternalKinetic,
                    damage.damage_type,
                    damage.attacker,
                    damage.source_type,
                    damage.source_index,
                    damage.from_direction,
                    damage.user_data,
                    damage.clear_invulnerability,
                )
            }
            Request::VehicleDeathOccupant {
                last_attacker,
                vehicle_vector,
            } => {
                let rules = self
                    .character_damage_rules
                    .as_ref()
                    .ok_or("acquisition_occupant_damage_rules_missing")?;
                let selected = if last_attacker >> 24 == 3 {
                    last_attacker
                } else {
                    target.character_id
                };
                let same_team = bot_plant
                    .map(|origin| origin.team)
                    .or_else(|| self.character_team_in_world(selected, target.world))
                    .zip(self.character_team_in_world(target.character_id, target.world))
                    .map(|(a, v)| a == v);
                let mut damage = metadata::vehicle_death_occupant(
                    target.character_id,
                    last_attacker,
                    vehicle_vector,
                    same_team,
                    rules.friendly_fire_multiplier(),
                )?;
                if bot_plant.is_some() {
                    damage.attacker = u32::MAX;
                }
                (
                    armor::RawCharacterDamage {
                        amount_hp: damage.amount_hp,
                        armor_rating_redux: damage.armor_rating_reduction,
                        armor_prevention_redux: damage.armor_prevention_reduction,
                        hit_location: damage.location,
                    },
                    damage::DamageKind::Explosive,
                    damage.damage_type,
                    damage.attacker,
                    damage.source_type,
                    damage.source_index,
                    damage.from_direction,
                    damage.user_data,
                    false,
                )
            }
            Request::VehicleExplosion {
                raw_hp,
                from_direction,
                attacker,
            }
            | Request::StructuralExplosion {
                raw_hp,
                from_direction,
                attacker,
                ..
            }
            | Request::AirStrikeExplosion {
                raw_hp,
                from_direction,
                attacker,
            }
            | Request::AirStrikeGas {
                raw_hp,
                from_direction,
                attacker,
            } => {
                if !raw_hp.is_finite() || raw_hp <= 0. {
                    return Err("invalid_acquisition_explosion_damage");
                }
                let (kind, wire_damage_type) = if matches!(request, Request::AirStrikeGas { .. }) {
                    (damage::DamageKind::Poison, 4)
                } else {
                    (damage::DamageKind::Explosive, 10)
                };
                (
                    armor::RawCharacterDamage {
                        amount_hp: raw_hp,
                        armor_rating_redux: 0.,
                        armor_prevention_redux: 0.,
                        hit_location: 1,
                    },
                    kind,
                    wire_damage_type,
                    attacker,
                    match request {
                        Request::AirStrikeExplosion { .. } => 4,
                        Request::AirStrikeGas { .. } => 7,
                        Request::StructuralExplosion { .. } => 6,
                        _ => 5,
                    },
                    -1,
                    [0., -1., 0.],
                    explosion_direction_user_data(from_direction)?,
                    false,
                )
            }
            Request::HospitalRocketExplosion {
                raw_hp,
                from_direction,
                attacker,
                ..
            } => {
                if !raw_hp.is_finite() || raw_hp <= 0. {
                    return Err("invalid_hospital_rocket_character_damage");
                }
                let rules = self.item_catalog.character_blast_rules();
                (
                    armor::native_character_blast_damage(raw_hp, rules),
                    damage::DamageKind::Explosive,
                    rules.damage_type,
                    attacker,
                    2, // Native Weapon category. The mounted global index is unqualified.
                    -1,
                    from_direction,
                    explosion_direction_user_data(from_direction)?,
                    false,
                )
            }
            Request::StructuralArea {
                raw_hp,
                from_direction,
            } => {
                if !raw_hp.is_finite()
                    || raw_hp <= 0.
                    || !from_direction.iter().all(|x| x.is_finite())
                {
                    return Err("invalid_structural_area_damage");
                }
                (
                    armor::RawCharacterDamage {
                        amount_hp: raw_hp,
                        armor_rating_redux: 0.,
                        armor_prevention_redux: 0.,
                        hit_location: 0,
                    },
                    damage::DamageKind::Bullet,
                    9,
                    target.character_id,
                    9,
                    -1,
                    from_direction,
                    0,
                    false,
                )
            }
        };
        if bot_plant.is_some() && attacker != u32::MAX {
            return Err("bot_plant_attacker_must_be_uncredited");
        }
        // Mirror the native bool temporary scope on the owned state value. The
        // restored byte is canonical1 even when the original nonzero byte was2.
        let mut native_during = native;
        if bypass {
            native_during.invulnerability = Some(0);
        }
        let mut native_after = native_during;
        if bypass {
            native_after.invulnerability = Some(1);
        }
        let mut plan = Plan {
            target,
            expected,
            native_before: native,
            native_after,
            now,
            outcome: Outcome::NoDamage,
            damage: None,
            life: None,
            packet: None,
            consequences: None,
            notice: None,
            audit: Value::Null,
            clear_healing: false,
        };
        // OnCharacterHit precedes the native dead/protection guards. Preserve
        // every pair in the caller's outer audit, including ignored duplicates.
        let mut audit = json!({"on_character_hit":true,"character_id":target.character_id,
            "generation":target.generation,"world":target.world,"raw_amount_hp":raw.amount_hp,
            "damage_type":wire_damage_type,
            "damage_source_type":source_type,"damage_source_index":(!rocket).then_some(source_index),
            "attacker":attacker,"source_object":source_object,"location":raw.hit_location,"user_data":user_data,
            "from_direction":from_direction,"invulnerability_before":initial_invulnerability,
            "invulnerability_during":native_during.invulnerability,"invulnerability_after":native_after.invulnerability});
        let human = plan.expected.reservation.as_ref().and_then(|id| {
            self.active
                .iter()
                .chain(self.peers.values())
                .find(|a| a.reservation.id == *id)
        });
        let dead = human.map_or(plan.expected.health == 0, |a| {
            a.life.damage_health().is_none()
        });
        if dead {
            plan.outcome = Outcome::DeadIgnored;
        } else if native_during.invulnerability != Some(0) {
            plan.outcome = Outcome::Protected;
        } else {
            let catalog = self.item_catalog.retail_combat()?;
            let mut armor = if let Some(victim) = human {
                let kit = victim
                    .equipped_loadout()
                    .ok_or("acquisition_character_live_loadout_missing")?;
                let modifiers = self.plan_leadership(now)?.modifiers(victim);
                let armor = armor::ArmorProfile::from_retail_loadout(&kit, catalog)?
                    .with_modifiers(&modifiers);
                if matches!(victim.life, CharacterLife::Incapacitated { .. }) {
                    armor.for_incapacitated(catalog)?
                } else {
                    armor
                }
            } else {
                let bot = self
                    .bot_registry
                    .bots
                    .get(&(target.character_id as u16))
                    .ok_or("acquisition_character_missing")?;
                armor::ArmorProfile::from_retail_loadout(&bot.target.loadout, catalog)?
            };
            // No default FF tuning for a live world effect. Self source is
            // base1, and distinct resolved same-team attackers use bound rules.
            if attacker != target.character_id {
                self.character_damage_rules
                    .as_ref()
                    .ok_or("acquisition_character_damage_rules_missing")?;
            }
            let incoming_base = if let Some(origin) = bot_plant {
                let exact_self = target.character_id == 0x0380_0000 | u32::from(origin.player)
                    && target.generation == origin.life
                    && self
                        .bot_registry
                        .bots
                        .get(&origin.player)
                        .is_some_and(|bot| bot.allocation_generation == origin.allocation)
                    && !self
                        .active
                        .iter()
                        .chain(self.peers.values())
                        .any(|actor| actor.reservation.player == origin.player);
                if !exact_self
                    && self.character_team_in_world(target.character_id, target.world)
                        == Some(origin.team)
                {
                    self.character_damage_rules
                        .as_ref()
                        .ok_or("acquisition_character_damage_rules_missing")?
                        .friendly_fire_multiplier()
                } else {
                    1.
                }
            } else {
                self.character_incoming_base(attacker, target.character_id, target.world)
            };
            armor = armor.with_incoming_base(incoming_base)?;
            let amount = armor.adjust_for_kind(raw, kind)?;
            audit["calculation"] = armor.calculation(raw, kind)?;
            plan.clear_healing = raw.amount_hp * armor.incoming_scale(kind) > 0.0;
            if amount > 0.0 {
                // The native internal request admits positive compensated raw
                // amounts beyond the external command's1000 limit. Reuse the
                // shared remaining-HP quantizer; never force terminal health.
                let mut damage = replication::CharacterDamage {
                    last_location_and_type: (wire_damage_type << 4) | raw.hit_location,
                    health_quarter_points: damage::remaining_health_quarter_points(
                        plan.expected.health,
                        amount,
                    )?,
                    conditions: 0,
                    last_attacker: attacker,
                    last_source_object: source_object,
                    last_user_data: user_data,
                };
                if let Some(victim) = human {
                    let (model, life) = self.plan_human_damage(victim, amount, damage, now)?;
                    damage = model;
                    plan.life = Some(life);
                    // Preflight the only fallible cleanup in apply_peer_damage.
                    if damage.health_quarter_points == 0 {
                        let removals = victim
                            .projectile_replicas
                            .clone()
                            .take_all_removals()
                            .into_iter()
                            .map(|removal| {
                                removal
                                    .encode_with_profile(
                                        1,
                                        now.as_millis() as u32,
                                        victim.wire_profile,
                                    )
                                    .map(|wire| protocol::encode_hex(&wire))
                            })
                            .collect::<Result<Vec<_>, _>>()?;
                        audit["projectile_removal_publications_hex"] = json!(removals);
                    }
                }
                plan.packet = Some(
                    replication::CharacterDamageUpdate {
                        sequence: 2,
                        timestamp_ms: now.as_millis() as u32,
                        object_index: target.character_id as u16,
                        damage,
                        forced_state: (damage.health_quarter_points == 0).then_some(
                            replication::CharacterState {
                                state: 7,
                                stance: 0,
                                user_data: [0; 3],
                            },
                        ),
                    }
                    .encode_with_profile(self.wire_profile)?,
                );
                audit["damage_publication_hex"] =
                    json!(protocol::encode_hex(plan.packet.as_ref().unwrap()));
                if !rocket && plan.expected.health > 0 && damage.health_quarter_points == 0 {
                    plan.notice = Some(
                        self.plan_character_effect_notice(
                            death_notifications::CharacterEffectNoticeRequest {
                                world: target.world,
                                victim: target.character_id as u16,
                                attacker,
                                source_type,
                                source_index,
                                location: raw.hit_location,
                                mounted: native
                                    .associated_vehicle
                                    .ok_or("acquisition_character_vehicle_association_missing")?,
                            },
                        )?,
                    );
                }
                plan.consequences = Some(events::plan_character_effect_consequences(
                    self,
                    events::CharacterEffectInput {
                        world: target.world,
                        victim_id: target.character_id as u16,
                        generation: target.generation,
                        attacker,
                        damage,
                        life: plan.life,
                        now,
                    },
                )?);
                audit["health_after_quarter_points"] = json!(damage.health_quarter_points);
                audit["life_after"] = json!(format!("{:?}", plan.life));
                audit["consequences"] = plan.consequences.as_ref().unwrap().audit();
                audit["death_notice"] = plan.notice.as_ref().map_or(Value::Null, |n| n.audit());
                plan.damage = Some(damage);
                plan.outcome = Outcome::Applied;
            }
        }
        audit["outcome"] = json!(format!("{:?}", plan.outcome));
        plan.audit = audit;
        Ok(plan)
    }

    /// No audit or direct delivery: call only on the isolated candidate after
    /// outer planning succeeds. Returned application messages join that outer
    /// transaction. Account persistence is the normal progression outbox.
    pub(crate) fn commit_acquisition_character_effect(
        &mut self,
        plan: Plan,
        native: &mut NativeState,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if *native != plan.native_before
            || self.acquisition_character_snapshot(plan.target)? != plan.expected
        {
            return Err("stale_acquisition_character_effect");
        }
        if let Some(consequences) = &plan.consequences {
            consequences.validate(self)?;
        }
        if let Some(damage) = plan.damage {
            if let Some(id) = &plan.expected.reservation {
                let victim = self
                    .active
                    .as_mut()
                    .filter(|a| a.reservation.id == *id)
                    .or_else(|| self.peers.get_mut(id))
                    .ok_or("acquisition_character_missing")?;
                victim.apply_peer_damage(plan.target.generation, damage, plan.now)?;
                if let Some(life) = plan.life {
                    victim.life = life;
                }
                if !victim.life.is_alive() {
                    victim
                        .grenade_damage
                        .retain(|hit| hit.target_character_id != plan.target.character_id);
                }
            } else {
                self.commit_bot_damage(
                    plan.target.character_id as u16,
                    plan.target.generation,
                    damage,
                    plan.now,
                )?;
            }
        }
        if plan.clear_healing {
            if let Some(id) = &plan.expected.reservation {
                if let Some(victim) = self
                    .active
                    .as_mut()
                    .filter(|a| a.reservation.id == *id)
                    .or_else(|| self.peers.get_mut(id))
                {
                    victim.healing = None;
                }
            } else if let Some(bot) = self
                .bot_registry
                .bots
                .get_mut(&(plan.target.character_id as u16))
            {
                bot.healing = Default::default();
            }
        }
        if let Some(packet) = plan.packet {
            self.broadcast_bot_packet(packet);
        }
        if let Some(notice) = plan.notice {
            notice.commit(self);
        }
        *native = plan.native_after;
        Ok(plan.consequences.map_or_else(Vec::new, |c| c.commit(self)))
    }
}

macro_rules! actor_effect_state {
    ($($field:ident : $ty:ty),* $(,)?) => {
        struct ActorEffects {
            $($field: $ty,)*
            notifications: std::collections::VecDeque<(bool, Vec<u8>)>,
        }
        impl ActorEffects {
            fn capture(actor: &Active) -> Self {
                Self { $($field: actor.$field.clone(),)* notifications: actor.pending_notifications.iter().map(|p| match p {
                    match_flow::Publication::Application(wire) => (false, wire.clone()),
                    match_flow::Publication::Replication(wire) => (true, wire.clone()),
                }).collect() }
            }
            fn install(self, actor: &mut Active) {
                $(actor.$field = self.$field;)*
                actor.pending_notifications = self.notifications.into_iter().map(|(replication, wire)|
                    if replication { match_flow::Publication::Replication(wire) } else { match_flow::Publication::Application(wire) }).collect();
            }
        }
    };
}
actor_effect_state! {
    life: CharacterLife,
    tracking: player_tracking::PlayerTracking,
    owner_fields: player_tracking::ClientCharacterFields,
    fall_tracker: fall_damage::Tracker,
    revive_state_pending: bool,
    pending_damage: Option<PendingDamage>,
    static_resupply: Vec<crate::human_static_resupply::Session>,
    last_damage: replication::CharacterDamage,
    healing: Option<progression::HealingSession>,
    reward_statistics: std::collections::BTreeMap<u8, i32>,
    pending_firearm_assist_rows: std::collections::BTreeMap<u16, firearm_assist_publication::Pending>,
    repair_credit: crate::repair_rewards::State,
    projectiles: projectile::ProjectileFlights,
    projectile_replicas: projectile_replication::ProjectileReplicas,
    projectile_publications: Vec<ProjectilePublication>,
    grenade_damage: std::collections::VecDeque<PendingDamage>,
    pending_shared: std::collections::VecDeque<Vec<u8>>,
    ordered_health_pending: bool,
    replication_backlogged: bool,
    mounted_seat: Option<u8>,
    mounted_bunker: Option<u16>,
    ground_seat: Option<ground_seats::PassengerSeat>,
    helo_seat: Option<domination_helo::PassengerSeat>,
    last_exit: Option<protocol::DismountSeat>,
    pending_spawn_deadline: Option<Duration>,
    published_wave_deadline: Option<Duration>,
}
struct EffectState {
    assists: firearm_assists::State,
    team_waves: [Option<TeamWaveClock>; 2],
    objective_world: Option<objective_control::ObjectiveWorld>,
    active_abilities: Option<active_ability_engine::Runtime>,
    actors: std::collections::BTreeMap<String, ActorEffects>,
    bots: bot_registry::CharacterDamageCandidate,
    rewards: std::collections::BTreeMap<String, progression::RewardEvent>,
    reward_order: std::collections::VecDeque<String>,
}
impl EffectState {
    fn capture<W: Write>(engine: &Engine<W>) -> Self {
        Self {
            assists: engine.firearm_assists.clone(),
            objective_world: engine.objective_world.clone(),
            active_abilities: engine.active_abilities.clone(),
            team_waves: engine.team_waves,
            actors: engine
                .active
                .iter()
                .chain(engine.peers.values())
                .map(|a| (a.reservation.id.clone(), ActorEffects::capture(a)))
                .collect(),
            bots: engine.bot_registry.character_damage_candidate(),
            rewards: engine.pending_rewards.clone(),
            reward_order: engine.reward_order.clone(),
        }
    }
    fn same_actors<W: Write>(&self, engine: &Engine<W>) -> bool {
        let ids: std::collections::BTreeSet<_> = engine
            .active
            .iter()
            .chain(engine.peers.values())
            .map(|a| a.reservation.id.as_str())
            .collect();
        ids == self.actors.keys().map(String::as_str).collect()
    }
    fn install<W: Write>(mut self, engine: &mut Engine<W>) {
        engine.firearm_assists = self.assists;
        engine.active_abilities = self.active_abilities;
        engine.team_waves = self.team_waves;
        engine.objective_world = self.objective_world;
        for actor in engine.active.iter_mut().chain(engine.peers.values_mut()) {
            self.actors
                .remove(&actor.reservation.id)
                .expect("validated character-effect candidate topology")
                .install(actor);
        }
        engine
            .bot_registry
            .install_character_damage_candidate(self.bots);
        engine.pending_rewards = self.rewards;
        engine.reward_order = self.reward_order;
    }
}
/// Maximum number of collector effects/corrections staged by one callback.
/// Duplicates count separately because they retain native collector order.
pub const MAX_CHARACTER_EFFECTS: usize = 2048;
type NativeMap = std::collections::BTreeMap<u32, NativeState>;
#[derive(PartialEq, Eq)]
struct Topology {
    humans: std::collections::BTreeMap<String, (u32, u16, u64, String)>,
    bots: std::collections::BTreeMap<u16, (u32, u16, u32)>,
}
impl Topology {
    fn capture<W: Write>(engine: &Engine<W>) -> Self {
        Self {
            humans: engine
                .active
                .iter()
                .chain(engine.peers.values())
                .map(|actor| {
                    (
                        actor.reservation.id.clone(),
                        (
                            actor.reservation.world,
                            actor.reservation.player,
                            actor.allocation_sequence,
                            format!("{:?}", actor.life),
                        ),
                    )
                })
                .collect(),
            bots: engine
                .bot_registry
                .bots
                .iter()
                .map(|(&id, bot)| {
                    (
                        id,
                        (
                            bot.target.runtime.map_or(0, |r| r.generation),
                            bot.health,
                            bot.allocation_generation,
                        ),
                    )
                })
                .collect(),
        }
    }
}
/// Fully prepared owned lifecycle/reward/reliable-publication fields. Dropping
/// it leaves authoritative Engine unchanged. Origin/revision/round/topology and
/// native input fences prevent a different Engine or later event adopting it.
pub struct Candidate {
    state: EffectState,
    native: NativeMap,
    native_before: NativeMap,
    origin: std::sync::Arc<()>,
    revision: u64,
    assist_revision: u64,
    round: u64,
    topology: Topology,
    now: Duration,
    audit: Vec<Value>,
    messages: Vec<OutboundMessage>,
}
impl Candidate {
    pub fn audit(&self) -> &[Value] {
        &self.audit
    }
    pub(crate) fn messages(&self) -> &[OutboundMessage] {
        &self.messages
    }
    pub fn native_states(&self) -> &NativeMap {
        &self.native
    }

    /// Check a terminal packet against the post-damage reliable queue. The
    /// Character callback only appends to this queue, so it shares the live
    /// recipient's separately reserved bootstrap range.
    pub(crate) fn check_extra_shared<W: Write>(
        &self,
        engine: &Engine<W>,
        world: u32,
        count: usize,
    ) -> Result<(), &'static str> {
        self.check_extra_shared_for(engine, world, |actor| {
            actor.initial_objects.is_some().then_some(count)
        })
    }

    pub(crate) fn check_extra_shared_for<W: Write>(
        &self,
        engine: &Engine<W>,
        world: u32,
        packets_for: impl Fn(&Active) -> Option<usize>,
    ) -> Result<(), &'static str> {
        for actor in engine
            .active
            .iter()
            .chain(engine.peers.values())
            .filter(|actor| actor.reservation.world == world)
        {
            let Some(count) = packets_for(actor) else {
                continue;
            };
            let staged = self
                .state
                .actors
                .get(&actor.reservation.id)
                .ok_or("stale_acquisition_character_candidate")?;
            let fits = staged
                .pending_shared
                .len()
                .checked_sub(actor.hospital_bootstrap.len())
                .and_then(|ordinary| ordinary.checked_sub(actor.smoke_bootstrap.len()))
                .and_then(|ordinary| ordinary.checked_add(count))
                .is_some_and(|total| total <= multiplayer::MAX_SHARED_QUEUE);
            if staged.replication_backlogged || !fits {
                return Err("hospital_rocket_publication_budget_exceeded");
            }
        }
        Ok(())
    }
}
impl<W: Write> Engine<W> {
    /// Run a bounded callback against cloned character-effect authority and
    /// restore the original fields before returning. Queries, corrections and
    /// damage may be interleaved in native pair order inside `prepare`.
    /// The closure may mutate only character-effect fields covered by EffectState;
    /// other world/objective candidates remain owned by the outer callback.
    pub(crate) fn prepare_acquisition_character_callback<F>(
        &mut self,
        native: &NativeMap,
        now: Duration,
        prepare: F,
    ) -> Result<Candidate, &'static str>
    where
        F: FnOnce(
            &mut Self,
            &mut NativeMap,
        ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str>,
    {
        if native.len() > MAX_CHARACTER_EFFECTS {
            return Err("acquisition_character_effect_limit");
        }
        let before = EffectState::capture(self);
        let origin = std::sync::Arc::clone(&self.acquisition_effect_origin);
        let revision = self.sequence;
        let assist_revision = self.firearm_assists.revision();
        let round = self.round_number;
        let topology = Topology::capture(self);
        let mut owned_native = native.clone();
        let result = prepare(self, &mut owned_native).and_then(|(audit, messages)| {
            if audit.len() > MAX_CHARACTER_EFFECTS
                || owned_native.len() > MAX_CHARACTER_EFFECTS
                || messages.len()
                    > MAX_CHARACTER_EFFECTS.saturating_mul(self.maximum_players.max(1))
            {
                return Err("acquisition_character_effect_limit");
            }
            Ok((EffectState::capture(self), audit, messages))
        });
        before.install(self);
        let (state, audit, messages) = result?;
        Ok(Candidate {
            state,
            native: owned_native,
            native_before: native.clone(),
            origin,
            revision,
            assist_revision,
            round,
            topology,
            now,
            audit,
            messages,
        })
    }
    pub fn prepare_acquisition_character_effects(
        &mut self,
        effects: &[(Target, Request)],
        native: &NativeMap,
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        self.prepare_acquisition_character_effects_with_moves(&[], effects, native, now)
    }
    pub(crate) fn prepare_rocket_character_effects(
        &mut self,
        source: &crate::hospital_rocket_runtime::LaunchSource,
        effects: &[(Target, Request)],
        native: &NativeMap,
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        if effects.len() > MAX_CHARACTER_EFFECTS {
            return Err("acquisition_character_effect_limit");
        }
        self.prepare_acquisition_character_callback(native, now, |engine, native| {
            let mut audit = Vec::with_capacity(effects.len());
            let mut messages = Vec::new();
            for &(target, request) in effects {
                if target.world != source.world
                    || !matches!(request, Request::HospitalRocketExplosion {
                        attacker: id, projectile_id, ..
                    } if id == attacker && projectile_id == source.projectile_id)
                {
                    return Err("invalid_hospital_rocket_character_source");
                }
                let state = native
                    .get_mut(&target.character_id)
                    .ok_or("acquisition_character_native_state_missing")?;
                let plan =
                    engine.plan_acquisition_character_effect(target, request, *state, now)?;
                let assist = plan
                    .damage
                    .filter(|damage| damage.health_quarter_points < plan.expected.health)
                    .and_then(|damage| {
                        firearm_assists::Plan::prepare_rocket(
                            engine,
                            source,
                            now,
                            firearm_assists::Fighter {
                                index: target.character_id as u16,
                                reservation: plan.expected.reservation.as_deref(),
                                generation: target.generation,
                            },
                            damage.health_quarter_points == 0,
                        )
                    });
                let statistic = assist
                    .as_ref()
                    .map(|assist| {
                        assist.check_before_record(engine)?;
                        events::plan_bot_firearm_assists(engine, assist)
                    })
                    .transpose()?;
                let mut effect_audit = plan.audit().clone();
                messages.extend(engine.commit_acquisition_character_effect(plan, state)?);
                if let (Some(assist), Some(statistic)) = (assist, statistic) {
                    effect_audit["assists"] = json!(assist.audit());
                    messages.extend(statistic.commit(engine));
                    assist.install(engine)?;
                }
                audit.push(effect_audit);
            }
            Ok((audit, messages))
        })
    }
    /// Convenience API for a pre-resolved batch. World-query callbacks that must
    /// update later cast layers after each death use the interleaved closure API.
    pub fn prepare_acquisition_character_effects_with_moves(
        &mut self,
        moves: &[(Target, replication::CharacterPosition)],
        effects: &[(Target, Request)],
        native: &NativeMap,
        now: Duration,
    ) -> Result<Candidate, &'static str> {
        if moves
            .len()
            .checked_add(effects.len())
            .is_none_or(|n| n > MAX_CHARACTER_EFFECTS)
        {
            return Err("acquisition_character_effect_limit");
        }
        self.prepare_acquisition_character_callback(native, now, |engine, native| {
            let mut audit = Vec::with_capacity(moves.len() + effects.len());
            let mut messages = Vec::new();
            for &(target, position) in moves {
                audit.push(engine.apply_acquisition_character_move(target, position, now)?);
            }
            for &(target, request) in effects {
                let state = native
                    .get_mut(&target.character_id)
                    .ok_or("acquisition_character_native_state_missing")?;
                let plan =
                    engine.plan_acquisition_character_effect(target, request, *state, now)?;
                audit.push(plan.audit().clone());
                messages.extend(engine.commit_acquisition_character_effect(plan, state)?);
            }
            Ok((audit, messages))
        })
    }
    /// Validate before the durable outer callback record. Accepted intervening
    /// events, another Engine, round changes or native-state changes are stale.
    pub fn check_acquisition_character_effects(
        &self,
        candidate: &Candidate,
        native: &NativeMap,
    ) -> Result<(), &'static str> {
        if !std::sync::Arc::ptr_eq(&self.acquisition_effect_origin, &candidate.origin)
            || self.sequence != candidate.revision
            || self.firearm_assists.revision() != candidate.assist_revision
            || self.round_number != candidate.round
            || *native != candidate.native_before
            || Topology::capture(self) != candidate.topology
            || !candidate.state.same_actors(self)
        {
            return Err("stale_acquisition_character_candidate");
        }
        if self.failed {
            return Err("recorder_unavailable");
        }
        Ok(())
    }
    /// Source-internal installation for an already validated, recorded outer
    /// callback. There are deliberately no fallible checks after its record.
    pub(crate) fn install_character_effects_after_record(
        &mut self,
        candidate: Candidate,
        native: &mut NativeMap,
    ) -> Vec<OutboundMessage> {
        candidate.state.install(self);
        self.prune_firearm_assist_victims();
        *native = candidate.native;
        candidate.messages
    }
    /// Apply only to an isolated owned candidate. Authoritative callers use the
    /// recording method below or check + their outer record + recorded install.
    #[cfg(test)]
    pub(crate) fn install_acquisition_character_effects(
        &mut self,
        candidate: Candidate,
        native: &mut NativeMap,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.check_acquisition_character_effects(&candidate, native)?;
        Ok(self.install_character_effects_after_record(candidate, native))
    }
    /// Concrete authoritative transaction: validate all fences, record complete
    /// effect/publication evidence, then install with no later fallible check.
    /// Account persistence remains the normal progression outbox protocol.
    pub fn record_acquisition_character_effects(
        &mut self,
        candidate: Candidate,
        native: &mut NativeMap,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.check_acquisition_character_effects(&candidate, native)?;
        self.record(
            json!({"op":"acquisition_character_effects", "effects":candidate.audit,
            "messages":candidate.messages,"timestamp_ms":candidate.now.as_millis() as u64}),
            wall,
        )?;
        Ok(self.install_character_effects_after_record(candidate, native))
    }
    /// Apply a qualified force-position result to the owned candidate. The
    /// ordinary Character field5 packet is reliable and includes its owner.
    pub(crate) fn apply_acquisition_character_move(
        &mut self,
        target: Target,
        position: replication::CharacterPosition,
        now: Duration,
    ) -> Result<Value, &'static str> {
        position.validate()?;
        let snapshot = self.acquisition_character_snapshot(target)?;
        if snapshot.life == LifeIdentity::Dead
            || snapshot.health == 0 && snapshot.reservation.is_none()
        {
            return Err("acquisition_move_target_not_living");
        }
        let fields = player_tracking::ClientCharacterFields {
            position: Some(position),
            ..Default::default()
        };
        let packet = replication::multiplayer::CharacterOwnerDelta {
            sequence: 2,
            timestamp_ms: now.as_millis() as u32,
            player: target.character_id as u16,
            fields: &fields,
        }
        .encode_with_profile(self.wire_profile)?;
        if let Some(id) = &snapshot.reservation {
            let actor = self
                .active
                .as_mut()
                .filter(|a| a.reservation.id == *id)
                .or_else(|| self.peers.get_mut(id))
                .ok_or("acquisition_character_missing")?;
            let mut fall = actor.fall_tracker.clone();
            fall.reset(
                target.generation,
                actor.tracking.accepted_position_sequence,
                position.y,
            )?;
            actor.tracking.server_position(
                position,
                player_tracking::PositionOrigin::ServerCorrection,
                now.as_millis() as u64,
            );
            actor.owner_fields.position = Some(position);
            actor.fall_tracker = fall;
        } else {
            self.bot_registry.correct_character_position(
                target.character_id as u16,
                target.generation,
                position,
                now,
            )?;
        }
        for recipient in self
            .active
            .iter_mut()
            .chain(self.peers.values_mut())
            .filter(|a| a.reservation.world == target.world && a.initial_objects.is_some())
        {
            recipient.queue_shared(packet.clone());
        }
        Ok(
            json!({"op":"acquisition_character_position", "character_id":target.character_id,
            "generation":target.generation,"world":target.world,"position":position,
            "publication_hex":protocol::encode_hex(&packet),"include_owner":true}),
        )
    }
}

#[cfg(test)]
#[path = "acquisition_character_effects_tests.rs"]
mod tests;

#[cfg(test)]
mod rocket_character_damage_tests {
    use super::*;

    #[test]
    fn rocket_character_dose_uses_native_explosive_metadata_and_existing_kill_credit() {
        let mut engine = crate::human_kill_reward_tests::enable_incoming_base(
            crate::human_kill_reward_tests::fixture(),
            0.25,
        );
        let id = "synthetic-reservation-0000";
        let victim = &engine.peers[id];
        let target = Target {
            character_id: 0x0380_0000,
            generation: 4,
            world: victim.reservation.world,
        };
        let rules = engine.item_catalog.character_blast_rules();
        let native = engine.owned_acquisition_characters();
        let request = Request::HospitalRocketExplosion {
            raw_hp: 120.,
            from_direction: [1., 0., 0.],
            attacker: 0x0380_0001,
            projectile_id: 0x1380_0001,
        };
        let candidate = engine
            .prepare_acquisition_character_effects(
                &[(target, request)],
                &native,
                Duration::from_secs(1),
            )
            .unwrap();
        assert!(engine.peers[id].life.is_alive());
        assert!(engine.pending_reward_batch().is_empty());
        let audit = &candidate.audit()[0];
        assert_eq!(audit["damage_source_type"], 2);
        assert!(audit["damage_source_index"].is_null());
        assert_eq!(audit["damage_type"], 10);
        assert_eq!(audit["location"], rules.hit_location);
        assert_eq!(audit["source_object"], 0x1380_0001);
        assert!(audit["death_notice"].is_null());

        let mut native = native;
        engine
            .install_acquisition_character_effects(candidate, &mut native)
            .unwrap();
        let victim = &engine.peers[id];
        assert!(!victim.life.is_alive());
        assert_eq!(
            victim.last_damage.last_location_and_type,
            0xa0 | rules.hit_location
        );
        assert_eq!(victim.last_damage.last_source_object, 0x1380_0001);
        assert_eq!(victim.last_damage.last_attacker, 0x0380_0001);
        assert_eq!(victim.reward_statistics.get(&12), Some(&1));
        assert_eq!(
            engine.active.as_ref().unwrap().reward_statistics.get(&10),
            Some(&1)
        );
        assert_eq!(
            engine
                .pending_reward_batch()
                .iter()
                .filter(|event| event.kind == progression::RewardKind::EnemyKilled)
                .count(),
            1
        );
    }

    #[test]
    fn rocket_character_dose_refuses_mounted_and_stale_victims() {
        let mut engine = crate::human_kill_reward_tests::fixture();
        let victim = engine.peers.get_mut("synthetic-reservation-0000").unwrap();
        victim.mounted_seat = Some(1);
        let mut target = Target {
            character_id: 0x0380_0000,
            generation: 4,
            world: victim.reservation.world,
        };
        let native = engine.owned_acquisition_characters();
        let request = Request::HospitalRocketExplosion {
            raw_hp: 120.,
            from_direction: [0., 0., 0.],
            attacker: 0x0380_0001,
            projectile_id: 0x1380_0001,
        };
        assert_eq!(
            engine
                .prepare_acquisition_character_effects(
                    &[(target, request)],
                    &native,
                    Duration::from_secs(1),
                )
                .err(),
            Some("hospital_rocket_character_mounted")
        );
        target.generation += 1;
        assert_eq!(
            engine
                .prepare_acquisition_character_effects(
                    &[(target, request)],
                    &native,
                    Duration::from_secs(1),
                )
                .err(),
            Some("stale_acquisition_character_generation")
        );
    }

    #[test]
    fn rocket_character_dose_terminal_preflights_staged_health_and_reserved_bootstrap() {
        let mut engine = crate::human_kill_reward_tests::fixture();
        let actor = engine.active.as_mut().unwrap();
        actor.initial_objects = Some(vec![]);
        actor
            .pending_shared
            .resize(multiplayer::MAX_SHARED_QUEUE - 1, vec![0]);
        actor.queue_hospital_bootstrap(vec![vec![1]]);
        actor.queue_smoke_bootstrap(vec![vec![3]]);
        let id = actor.reservation.id.clone();
        let world = actor.reservation.world;
        let native = engine.owned_acquisition_characters();
        let mut candidate = engine
            .prepare_acquisition_character_effects(&[], &native, Duration::from_secs(1))
            .unwrap();
        assert_eq!(candidate.check_extra_shared(&engine, world, 1), Ok(()));
        candidate
            .state
            .actors
            .get_mut(&id)
            .unwrap()
            .pending_shared
            .push_back(vec![2]);
        assert_eq!(
            candidate.check_extra_shared(&engine, world, 1),
            Err("hospital_rocket_publication_budget_exceeded")
        );
        assert_eq!(
            engine.active.as_ref().unwrap().ordinary_shared_len(),
            multiplayer::MAX_SHARED_QUEUE - 1
        );
    }
}
