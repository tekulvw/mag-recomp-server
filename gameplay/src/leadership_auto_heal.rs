//! Scheduled self-healing for qualified living retail characters.
use super::*;
use std::collections::BTreeMap;

const INTERVAL: f32 = 1.5;
const EPSILON: f32 = 0.0005;

#[derive(Clone, Debug, PartialEq)]
struct Clock {
    world: u32,
    player: u16,
    generation: u32,
    match_id: String,
    last_update: Duration,
    elapsed: f32,
}
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct State {
    epoch: Option<Duration>,
    clocks: BTreeMap<String, Clock>,
}
impl State {
    fn same_accumulators(&self, other: &Self) -> bool {
        self.epoch == other.epoch
            && self.clocks.len() == other.clocks.len()
            && self.clocks.iter().all(|(id, a)| {
                other.clocks.get(id).is_some_and(|b| {
                    a.world == b.world
                        && a.player == b.player
                        && a.generation == b.generation
                        && a.match_id == b.match_id
                        && a.elapsed == b.elapsed
                })
            })
    }
}

impl<W: Write> Engine<W> {
    pub(crate) fn tick_leadership_auto_heal(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if !self.match_started || self.match_terminal.is_some() {
            return Ok(());
        }
        let leadership = self.plan_leadership(now)?;
        let mut after = if self.automatic_heal.epoch == self.match_epoch {
            self.automatic_heal.clone()
        } else {
            State {
                epoch: self.match_epoch,
                ..Default::default()
            }
        };
        let mut clocks = BTreeMap::new();
        let mut updates = Vec::new();
        let mut attempts = Vec::new();
        let mut actors: Vec<_> = self.active.iter().chain(self.peers.values()).collect();
        actors.sort_by_key(|a| (a.reservation.world, a.reservation.player));
        for actor in actors {
            let CharacterLife::Alive {
                generation,
                health_quarter_points: 1..,
            } = actor.life
            else {
                continue;
            };
            // Self-healing belongs to the Character, independently of the
            // associated vehicle's separate automatic repair timer.
            let supported_state = actor
                .owner_fields
                .state
                .is_some_and(|state| match state.state {
                    1 => !actor.is_mounted(),
                    14 => {
                        actor.mounted_seat.is_none()
                            && self.current_passive_ground_association(actor).is_some()
                    }
                    _ => false,
                });
            if actor.wire_profile.application_id() != 20231
                || !actor.entered_game
                || !actor.ready_for_updates
                || actor.initial_objects.is_none()
                || !supported_state
            {
                continue;
            }
            let id = &actor.reservation.id;
            let mut clock = after
                .clocks
                .get(id)
                .filter(|c| {
                    c.world == actor.reservation.world
                        && c.player == actor.reservation.player
                        && c.generation == generation
                        && c.match_id == actor.reward_match_id
                })
                .cloned()
                .unwrap_or_else(|| Clock {
                    world: actor.reservation.world,
                    player: actor.reservation.player,
                    generation,
                    match_id: actor.reward_match_id.clone(),
                    last_update: now,
                    elapsed: 0.,
                });
            let dt = now
                .checked_sub(clock.last_update)
                .ok_or("automatic_heal_time_regression")?
                .as_secs_f32();
            clock.last_update = now;
            let modifiers = leadership.modifiers(actor);
            let modifier = modifiers.value("kCharModAutoHeal");
            if !modifier.is_finite() {
                return Err("invalid_automatic_heal_modifier");
            }
            if modifier.abs() < EPSILON {
                clock.elapsed = 0.;
            } else {
                clock.elapsed += dt;
                if !clock.elapsed.is_finite() {
                    return Err("automatic_heal_time_overflow");
                }
                if clock.elapsed > INTERVAL {
                    // Heal uses float maximum/missing HP before quantizing every
                    // admitted application, including a positive sub-quarter one.
                    modifiers.max_health_quarter_points()?;
                    let max_hp = modifiers.max_health_hp();
                    let old = actor.last_damage;
                    let old_hp = f32::from(old.health_quarter_points) * 0.25;
                    let amount = (max_hp * ((modifier * INTERVAL) / 100.)).min(max_hp - old_hp);
                    let mut changed = false;
                    // Character spawn invulnerability allows type6. The model's
                    // independent special-context guard is outside this scope.
                    if amount > 0. && old.conditions & 2 == 0 {
                        let mut damage = old;
                        damage.health_quarter_points =
                            (((old_hp + amount).min(max_hp) * 4.).trunc() as u16) & 0xfff;
                        damage.last_location_and_type = 0x60 | (old.last_location_and_type & 0xf);
                        let packet = replication::CharacterDamageUpdate {
                            seated_exit: None,
                            sequence: 2,
                            timestamp_ms: now.as_millis() as u32,
                            object_index: actor.reservation.player,
                            damage,
                            forced_state: None,
                        }
                        .encode_with_profile(self.wire_profile)?;
                        updates.push((id.clone(), generation, damage, packet));
                        changed = true;
                    }
                    clock.elapsed -= INTERVAL;
                    attempts.push(
                        json!({"player":actor.reservation.player,"generation":generation,
                        "amount_hp":amount,"applied":changed}),
                    );
                }
            }
            clocks.insert(id.clone(), clock);
        }
        after.clocks = clocks;
        let changes = leadership.audit();
        if after.same_accumulators(&self.automatic_heal)
            && attempts.is_empty()
            && changes.is_empty()
        {
            // A quiet cadence cursor carries no accumulated gameplay result.
            self.automatic_heal = after;
            return Ok(());
        }
        self.record(json!({"op":"leadership_auto_heal","timestamp_ms":now.as_millis() as u64,
            "attempts":attempts,"leadership_changes":changes,
            "updates":updates.iter().map(|(id,_,_,p)| json!({"reservation_id":id,"packet":protocol::encode_hex(p)})).collect::<Vec<_>>()}),wall)?;
        self.automatic_heal = after;
        self.commit_leadership(leadership);
        for (id, generation, damage, packet) in updates {
            let actor = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|a| a.reservation.id == id)
                .unwrap();
            actor.last_damage = damage;
            actor.life = CharacterLife::Alive {
                generation,
                health_quarter_points: damage.health_quarter_points,
            };
            // Include initialized recipients still awaiting readiness: existing
            // human snapshots are not periodically resent like bot snapshots.
            for recipient in self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .filter(|a| a.initial_objects.is_some())
            {
                recipient.queue_shared(packet.clone());
                recipient.ordered_health_pending = true;
            }
        }
        Ok(())
    }
}
