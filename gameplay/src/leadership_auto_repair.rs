//! Scheduler-owned automatic repair for the qualified Acquisition Ground path.
use super::*;
use crate::acquisition_ground_runtime::Key;
use std::collections::BTreeMap;

const INTERVAL_SECONDS: f32 = 1.5;
const MODIFIER_EPSILON: f32 = 0.0005;

#[derive(Clone, Debug, PartialEq)]
struct Clock {
    world: u32,
    player: u16,
    generation: u32,
    match_id: String,
    last_update: Duration,
    elapsed_seconds: f32,
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
            && self.clocks.iter().all(|(id, clock)| {
                other.clocks.get(id).is_some_and(|prior| {
                    clock.world == prior.world
                        && clock.player == prior.player
                        && clock.generation == prior.generation
                        && clock.match_id == prior.match_id
                        && clock.elapsed_seconds == prior.elapsed_seconds
                })
            })
    }
}

impl<W: Write> Engine<W> {
    /// The association is independent of Heal's life/critical/full-HP gates:
    /// a due attempt on a valid association consumes its interval even if Heal
    /// declines. Only source-type9 is qualified for this first consumer.
    fn automatic_repair_association(&self, actor: &Active) -> Option<Key> {
        if actor.wire_profile.application_id() != 20231
            || !actor.entered_game
            || !actor.ready_for_updates
            || actor.initial_objects.is_none()
            || !matches!(
                actor.life,
                CharacterLife::Alive {
                    health_quarter_points: 1..,
                    ..
                }
            )
            || !self.ground_vehicle_creation_complete(actor.ground_vehicle_batch)
        {
            return None;
        }
        let seat = actor.ground_seat?;
        let catalog = self.ground_vehicles.as_ref()?;
        let creation = catalog.vehicle(seat.vehicle_index)?;
        if creation.team != actor.team()
            || (!creation.passenger_seats.contains(&seat.seat)
                && !catalog
                    .driver_seat(seat.vehicle_index)
                    .is_some_and(|driver| driver.seat == seat.seat))
            || self
                .active
                .iter()
                .chain(self.peers.values())
                .filter(|other| {
                    other.reservation.world == actor.reservation.world
                        && other.ground_seat == Some(seat)
                })
                .count()
                != 1
        {
            return None;
        }
        let vehicle = self.ground_runtime.as_ref()?.get(seat.vehicle_index)?;
        (vehicle.source_spawn_type == Some(9)).then_some(vehicle.key)
    }

    pub(crate) fn tick_leadership_auto_repair(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        if self.acquisition_gameplay.is_none()
            || !self.match_started
            || self.match_terminal.is_some()
        {
            return Ok(());
        }
        let leadership = self.plan_leadership(now)?;
        let mut after = self.automatic_repair.clone();
        if after.epoch != self.match_epoch {
            after = State {
                epoch: self.match_epoch,
                ..Default::default()
            };
        }
        let mut ground = self
            .ground_runtime
            .clone()
            .ok_or("ground_vehicle_runtime_required")?;
        let mut changes = BTreeMap::new();
        let mut attempts = Vec::new();
        let mut clocks = BTreeMap::new();
        // Family slot order remains stable regardless of which peer sent input.
        let mut actors: Vec<_> = self.active.iter().chain(self.peers.values()).collect();
        actors.sort_by_key(|actor| (actor.reservation.world, actor.reservation.player));
        for actor in actors {
            let CharacterLife::Alive {
                generation,
                health_quarter_points: 1..,
            } = actor.life
            else {
                continue;
            };
            if actor.wire_profile.application_id() != 20231
                || !actor.entered_game
                || !actor.ready_for_updates
                || actor.initial_objects.is_none()
            {
                continue;
            }
            let id = &actor.reservation.id;
            let mut clock = after
                .clocks
                .get(id)
                .filter(|clock| {
                    clock.world == actor.reservation.world
                        && clock.player == actor.reservation.player
                        && clock.generation == generation
                        && clock.match_id == actor.reward_match_id
                })
                .cloned()
                .unwrap_or_else(|| Clock {
                    world: actor.reservation.world,
                    player: actor.reservation.player,
                    generation,
                    match_id: actor.reward_match_id.clone(),
                    last_update: now,
                    elapsed_seconds: 0.,
                });
            let dt = now
                .checked_sub(clock.last_update)
                .ok_or("automatic_repair_time_regression")?
                .as_secs_f32();
            clock.last_update = now;
            let modifier = leadership.modifiers(actor).value("kCharModAutoRepair");
            if !modifier.is_finite() {
                return Err("invalid_automatic_repair_modifier");
            }
            let association = self.automatic_repair_association(actor);
            let Some(key) = association.filter(|_| modifier.abs() >= MODIFIER_EPSILON) else {
                // Reset elapsed time, retaining the current life clock so a
                // restored association counts its first ordinary update delta.
                clock.elapsed_seconds = 0.;
                clocks.insert(id.clone(), clock);
                continue;
            };
            clock.elapsed_seconds += dt;
            if !clock.elapsed_seconds.is_finite() {
                return Err("automatic_repair_time_overflow");
            }
            if clock.elapsed_seconds > INTERVAL_SECONDS {
                let percent = modifier * INTERVAL_SECONDS;
                let events = ground.heal_percent(key, percent)?;
                if !events.is_empty() {
                    changes.insert(key.index, 1 << 18);
                }
                clock.elapsed_seconds -= INTERVAL_SECONDS;
                attempts.push(
                    json!({"player":actor.reservation.player,"vehicle":key.object_id(),
                    "percent":percent,"changed":!events.is_empty()}),
                );
            }
            clocks.insert(id.clone(), clock);
        }
        after.clocks = clocks;
        let leadership_changes = leadership.audit();
        if after.same_accumulators(&self.automatic_repair)
            && changes.is_empty()
            && leadership_changes.is_empty()
        {
            // Like the ordinary Ground quiet-frame cursor, last_update only
            // observes cadence. It carries no accumulated repair or gameplay
            // result and must not create an empty durable record every tick.
            self.automatic_repair = after;
            return Ok(());
        }
        let packet = self.encode_ground_changes(&ground, &changes, now)?;
        self.record(
            json!({"op":"leadership_auto_repair","timestamp_ms":now.as_millis() as u64,
            "attempts":attempts,"leadership_changes":leadership_changes,
            "ground_update":packet.as_ref().map(|packet|protocol::encode_hex(packet))}),
            wall,
        )?;
        self.ground_runtime = Some(ground);
        self.automatic_repair = after;
        self.commit_leadership(leadership);
        if let Some(packet) = packet {
            self.broadcast_bot_packet(packet);
        }
        Ok(())
    }
}
