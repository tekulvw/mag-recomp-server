//! Owned vehicle delivery, staged with the objective callback and occupant changes.
use super::*;
use crate::{
    acquisition_gameplay_engine::Runtime,
    acquisition_ground_runtime::{self as ground, Body, Key},
    dynamic_collision::{Geometry, Pose, geometry},
    ground_vehicles::Creation,
    replication::{CharacterState, ObjectiveState},
    vehicle_exit::BunkerExit,
};
use std::collections::BTreeMap;

struct OccupantExit {
    player: u16,
    generation: u32,
    state: CharacterState,
    exit: BunkerExit,
    packet: Vec<u8>,
}

struct Delivery {
    driver: u16,
    exits: Vec<OccupantExit>,
}

/// Frozen mounted pose with the exact pending request that may consume it.
pub(crate) struct RewardWitness {
    pub recipient: Option<progression::RewardRecipient>,
    pub reservation_id: String,
    pub world: u32,
    pub player: u16,
    pub match_id: String,
    pub position: Option<[f32; 3]>,
    pub target_version: u64,
    pub timestamp_ms: u32,
}

impl<W: Write> Engine<W> {
    /// Called inside the isolated character callback. All occupants must have
    /// qualified exits before any occupant, vehicle, or objective is changed.
    pub(crate) fn stage_acquisition_deliveries(
        &mut self,
        runtime: &mut Runtime,
        ground: &mut ground::Runtime,
        now: Duration,
        changes: &mut BTreeMap<u16, u32>,
        entry_actors: &mut BTreeMap<u32, u16>,
        rewards: &mut BTreeMap<u32, RewardWitness>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        let setup = runtime.setup.clone();
        let mut audit = Vec::new();
        for chain in setup.controller.chains() {
            let arrival = runtime
                .state
                .objective(chain.arrival_token)
                .ok_or("acquisition_delivery_arrival_missing")?;
            if arrival.state.current != ObjectiveState::Deactivated as i32
                || arrival.state.target != arrival.state.current
            {
                continue;
            }
            let capture = runtime
                .state
                .objective(chain.capture_token)
                .ok_or("acquisition_delivery_capture_missing")?;
            if capture.state.current != ObjectiveState::Completed as i32
                || capture.state.target != capture.state.current
            {
                continue;
            }
            let fsp = capture.fsp.ok_or("acquisition_delivery_fsp_missing")?;
            let Some(vehicle) = ground.get(fsp.vehicle.index).filter(|vehicle| {
                vehicle.key == fsp.vehicle
                    && vehicle.source_spawn_type == Some(9)
                    && vehicle.source_spawn_point_index == fsp.source.spawn_point_index
                    && vehicle.source_spawn_point_token == fsp.source.spawn_point_token
                    && vehicle.source_spawn_point_token == chain.vehicle_spawn_token
                    && vehicle.associated_objective == Some(capture.object_id())
                    && vehicle.state == 2
                    && vehicle.body == Body::Operational
                    && !vehicle.is_dead()
            }) else {
                continue;
            };
            let key = vehicle.key;
            let previous = self.ground_runtime.replace(ground.clone());
            let planned = self.plan_acquisition_delivery(&setup, chain.arrival_token, key, now);
            self.ground_runtime = previous;
            let Some(delivery) = (match planned {
                Err("ground_exit_no_qualified_current_candidate") => {
                    audit.push(
                        json!({"op":"acquisition_delivery_deferred","objective":chain.arrival_token,
                        "vehicle":key.object_id(),"reason":"occupant_exit_clearance"}),
                    );
                    continue;
                }
                result => result?,
            }) else {
                continue;
            };

            runtime
                .state
                .request_state(chain.arrival_token, ObjectiveState::Completed)?;
            let witness = self.plan_acquisition_delivery_reward(
                runtime,
                ground,
                chain.arrival_token,
                delivery.driver,
                now,
            )?;
            self.install_acquisition_occupant_exits(delivery.exits, now)?;
            ground.deliver(key)?;
            if let Some(witness) = witness {
                rewards.insert(chain.arrival_token, witness);
            }
            entry_actors.insert(chain.arrival_token, delivery.driver);
            // Arrival publishes the inactive vehicle and both ownership revocations together.
            let fields = changes.entry(key.index).or_default();
            *fields |= (1 << 1) | (1 << 5);
            if ground
                .get(key.index)
                .is_some_and(|vehicle| vehicle.turret.is_some())
            {
                *fields |= 1 << 6;
            }
            audit.push(
                json!({"op":"acquisition_delivery","objective":chain.arrival_token,
                "vehicle":key.object_id(),"driver":delivery.driver}),
            );
        }
        Ok((audit, Vec::new()))
    }

    fn plan_acquisition_delivery(
        &self,
        setup: &acquisition_gameplay_engine::Setup,
        arrival_token: u32,
        key: Key,
        now: Duration,
    ) -> Result<Option<Delivery>, &'static str> {
        let Some(driver) = self.ground_driver_owner(key.index) else {
            return Ok(None);
        };
        let vehicle = self
            .current_ground_vehicle(key.index)
            .ok_or("acquisition_delivery_vehicle_missing")?;
        let trigger = setup
            .arrival_triggers
            .get(&arrival_token)
            .ok_or("acquisition_arrival_trigger_missing")?;
        let snapshot = self.dynamic_collision_snapshot(setup.filter_rows[6])?;
        let trigger_shape = Geometry::Box {
            half_extents: trigger.half_extents.map(f64::from),
            radius: 0.,
        };
        let trigger_pose = Pose {
            translation: trigger.centre.map(f64::from),
            rotation: std::array::from_fn(|i| {
                std::array::from_fn(|j| f64::from(trigger.axes[j][i]))
            }),
        };
        let source = u64::from(key.object_id());
        let limits = setup.physics.query_limits;
        let mut budget = limits.max_support_points;
        let chassis = snapshot
            .bodies()
            .iter()
            .find(|body| {
                body.target.key.id == source
                    && body.target.key.generation == key.generation
                    && body.root == "chassis"
            })
            .ok_or("acquisition_delivery_chassis_missing")?;
        if !geometry::penetrates(
            &trigger_shape,
            trigger_pose,
            &chassis.shape,
            chassis.pose,
            limits,
            &mut budget,
        )? {
            return Ok(None);
        }
        let position = vehicle.position;
        let mut ray_budget = limits.max_support_points;
        let static_hit = setup
            .arrival_static
            .as_ref()
            .map(|scene| scene.positive_hit(position, setup.filter_rows, &mut ray_budget))
            .transpose()?
            .unwrap_or(false);
        let start = [position.x, position.y, position.z].map(f64::from);
        let end = [start[0], f64::from(position.y + 5.), start[2]];
        if !static_hit
            && !snapshot.positive_ray_hit(start, end, source, setup.filter_rows, limits)?
        {
            return Ok(None);
        }
        let exits = self.plan_acquisition_occupant_exits(&vehicle, now)?;
        Ok(Some(Delivery { driver, exits }))
    }

    pub(crate) fn stage_acquisition_ground_ejection(
        &mut self,
        vehicle: &Creation,
        now: Duration,
    ) -> Result<(), &'static str> {
        let exits = self.plan_acquisition_occupant_exits(vehicle, now)?;
        self.install_acquisition_occupant_exits(exits, now)
    }

    fn plan_acquisition_occupant_exits(
        &self,
        vehicle: &Creation,
        now: Duration,
    ) -> Result<Vec<OccupantExit>, &'static str> {
        let mut occupants: Vec<_> = self
            .active
            .iter()
            .chain(self.peers.values())
            .filter_map(|actor| {
                actor
                    .ground_seat
                    .filter(|seat| seat.vehicle_index == vehicle.index)
                    .map(|seat| (seat.seat, actor))
            })
            .collect();
        occupants.sort_by_key(|(seat, _)| *seat);
        let mut exits = Vec::with_capacity(occupants.len());
        let mut reserved = Vec::with_capacity(occupants.len());
        for (_, actor) in occupants {
            let (generation, state) = match actor.life {
                CharacterLife::Alive { generation, .. } => (generation, 1),
                CharacterLife::Incapacitated { generation, .. } => (generation, 7),
                _ => return Err("invalid_acquisition_delivery_occupant"),
            };
            let player = actor.reservation.player;
            let exit = self.ground_exit_with_reserved(vehicle, player, &reserved)?;
            let state = CharacterState {
                state,
                stance: 0,
                user_data: [0; 3],
            };
            let packet = ground_seats::transition(
                2,
                now.as_millis() as u32,
                player,
                state,
                Some(exit),
                None,
                self.wire_profile,
            )?;
            reserved.push(exit.position);
            exits.push(OccupantExit {
                player,
                generation,
                state,
                exit,
                packet,
            });
        }
        Ok(exits)
    }

    fn install_acquisition_occupant_exits(
        &mut self,
        exits: Vec<OccupantExit>,
        now: Duration,
    ) -> Result<(), &'static str> {
        for planned in exits {
            let actor = self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .find(|actor| actor.reservation.player == planned.player)
                .ok_or("acquisition_delivery_occupant_missing")?;
            let exit = planned.exit;
            actor.fall_tracker.reset(
                planned.generation,
                actor.latest_owner_sequence,
                exit.position.y,
            )?;
            actor.ground_seat = None;
            actor.flash_yaw = None;
            actor.helo_seat = None;
            actor.mounted_seat = None;
            actor.mounted_bunker = None;
            actor.last_exit = None;
            actor.owner_fields.state = Some(planned.state);
            actor.owner_fields.position = Some(exit.position);
            actor.owner_fields.yaw = Some(exit.packed_yaw);
            actor.tracking.server_position(
                exit.position,
                player_tracking::PositionOrigin::ServerExit,
                now.as_millis() as u64,
            );
            for recipient in self
                .active
                .iter_mut()
                .chain(self.peers.values_mut())
                .filter(|actor| actor.initial_objects.is_some())
            {
                recipient.queue_shared(planned.packet.clone());
            }
        }
        Ok(())
    }
}
