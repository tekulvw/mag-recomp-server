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
    rule: &'static str,
}

/// A client Arrival contact admitted from the linked Escort's current driver.
/// It lasts until delivery, a different Escort incarnation, or the Arrival
/// leaving its open Deactivated state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ArrivalRequest {
    pub vehicle: Key,
    pub player: u16,
    pub generation: u32,
    pub timestamp_ms: u32,
}

/// The operational Escort of `chain` when its capture is complete and its
/// Arrival is open; otherwise `None`.
fn linked_escort(
    runtime: &Runtime,
    ground: &ground::Runtime,
    chain: &acquisition_control::Chain,
) -> Result<Option<Key>, &'static str> {
    let arrival = runtime
        .state
        .objective(chain.arrival_token)
        .ok_or("acquisition_delivery_arrival_missing")?;
    if arrival.state.current != ObjectiveState::Deactivated as i32
        || arrival.state.target != arrival.state.current
    {
        return Ok(None);
    }
    let capture = runtime
        .state
        .objective(chain.capture_token)
        .ok_or("acquisition_delivery_capture_missing")?;
    if capture.state.current != ObjectiveState::Completed as i32
        || capture.state.target != capture.state.current
    {
        return Ok(None);
    }
    let fsp = capture.fsp.ok_or("acquisition_delivery_fsp_missing")?;
    Ok(ground
        .get(fsp.vehicle.index)
        .filter(|vehicle| {
            vehicle.key == fsp.vehicle
                && vehicle.source_spawn_type == Some(9)
                && vehicle.source_spawn_point_index == fsp.source.spawn_point_index
                && vehicle.source_spawn_point_token == fsp.source.spawn_point_token
                && vehicle.source_spawn_point_token == chain.vehicle_spawn_token
                && vehicle.associated_objective == Some(capture.object_id())
                && vehicle.state == 2
                && vehicle.body == Body::Operational
                && !vehicle.is_dead()
        })
        .map(|vehicle| vehicle.key))
}

/// Whether the server holds collision above the Arrival trigger for the
/// retail ray to hit. The probe starts at the trigger's highest point, so the
/// ground under the trigger never counts.
fn arrival_container_modelled(
    setup: &acquisition_gameplay_engine::Setup,
    trigger: &crate::objective_range::TriggerBox,
) -> Result<bool, &'static str> {
    let Some(scene) = &setup.arrival_static else {
        return Ok(false);
    };
    let rise: f32 = (0..3)
        .map(|j| trigger.axes[j][1].abs() * trigger.half_extents[j])
        .sum();
    let top = crate::replication::CharacterPosition {
        x: trigger.centre[0],
        y: trigger.centre[1] + rise,
        z: trigger.centre[2],
    };
    let mut budget = setup.physics.query_limits.max_support_points;
    scene.positive_hit(top, setup.filter_rows, &mut budget)
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
            let linked = linked_escort(runtime, ground, chain)?;
            // A contact belongs to one Escort incarnation and one open Arrival.
            if let Some(request) = runtime.arrival_requests.get(&chain.arrival_token).copied()
                && linked != Some(request.vehicle)
            {
                runtime.arrival_requests.remove(&chain.arrival_token);
                audit.push(json!({"op":"acquisition_arrival_request_cleared",
                    "objective":chain.arrival_token,"vehicle":request.vehicle.object_id(),
                    "player":request.player}));
            }
            let Some(key) = linked else {
                continue;
            };
            let requested = runtime.arrival_requests.contains_key(&chain.arrival_token);
            let previous = self.ground_runtime.replace(ground.clone());
            let planned =
                self.plan_acquisition_delivery(&setup, chain.arrival_token, key, now, requested);
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
            runtime.arrival_requests.remove(&chain.arrival_token);
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
                "vehicle":key.object_id(),"driver":delivery.driver,
                "requested":requested,"rule":delivery.rule}),
            );
        }
        Ok((audit, Vec::new()))
    }

    /// The qualification rule depends on who reported the contact:
    ///
    /// - A client Arrival contact from the Escort's driver uses the retail
    ///   upward ray when the server holds collision above the trigger (the
    ///   extraction container). The container is often created only by the
    ///   Arrival's client sequence; without that collision the ray has nothing
    ///   to hit, so the chassis must overlap the authored trigger instead.
    /// - Without a client contact the server needs both the overlap and the ray.
    fn plan_acquisition_delivery(
        &self,
        setup: &acquisition_gameplay_engine::Setup,
        arrival_token: u32,
        key: Key,
        now: Duration,
        requested: bool,
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
        let source = u64::from(key.object_id());
        let limits = setup.physics.query_limits;
        let overlap = || -> Result<bool, &'static str> {
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
            geometry::penetrates(
                &trigger_shape,
                trigger_pose,
                &chassis.shape,
                chassis.pose,
                limits,
                &mut budget,
            )
        };
        let ray = || -> Result<bool, &'static str> {
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
            Ok(static_hit
                || snapshot.positive_ray_hit(start, end, source, setup.filter_rows, limits)?)
        };
        let (rule, qualified) = if !requested {
            ("server_contact", overlap()? && ray()?)
        } else if arrival_container_modelled(setup, trigger)? {
            ("retail_ray", ray()?)
        } else {
            ("trigger_overlap", overlap()?)
        };
        if !qualified {
            return Ok(None);
        }
        let exits = self.plan_acquisition_occupant_exits(&vehicle, now)?;
        Ok(Some(Delivery {
            driver,
            exits,
            rule,
        }))
    }

    /// The Arrival objective that `target_object_id` names, when it is one.
    pub(crate) fn acquisition_arrival_chain(
        &self,
        target_object_id: u32,
    ) -> Option<acquisition_control::Chain> {
        if target_object_id & 0xffff_0000 != 0x0c80_0000 {
            return None;
        }
        let runtime = self.acquisition_gameplay.as_ref()?;
        runtime
            .setup
            .controller
            .chains()
            .iter()
            .find(|chain| {
                runtime
                    .state
                    .objective(chain.arrival_token)
                    .is_some_and(|o| o.replication.object_index == target_object_id as u16)
            })
            .copied()
    }

    /// The retail client sends Activate for an Arrival when the Escort it is
    /// driving touches the trigger. Admit it only from the current driver of
    /// the Escort linked to that Arrival; delivery then follows in the
    /// objective tick through the same staged transaction as any delivery.
    pub(crate) fn event_acquisition_arrival_request(
        &mut self,
        event: &events::EventInput,
        request: protocol::ObjectiveInteraction,
        chain: acquisition_control::Chain,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        let mut runtime = self
            .acquisition_gameplay
            .clone()
            .ok_or("acquisition_gameplay_unavailable")?;
        let outcome = self.admit_arrival_request(&runtime, &chain, now)?;
        let active = self.active.as_ref().ok_or("no_reservation")?;
        let accepted = outcome.is_ok();
        let reply = protocol::ObjectiveInteraction {
            target_object_id: request.target_object_id,
            kind: if accepted {
                protocol::ObjectiveInteractionKind::Ack
            } else {
                protocol::ObjectiveInteractionKind::Nak
            },
            actor_index: active.reservation.player,
            trigger_index: if accepted {
                request.trigger_index
            } else {
                u8::MAX
            },
        };
        let message = OutboundMessage {
            reservation_id: event.reservation_id.clone(),
            world_id: event.world_id,
            player_id: event.player_id,
            plugin_id: 1,
            payload_hex: protocol::encode_hex(&reply.encode()),
        };
        let audit = json!({
            "op":"acquisition_arrival_request", "timestamp_ms":now.as_millis() as u64,
            "world_id":event.world_id, "player_id":event.player_id,
            "reservation_id":event.reservation_id, "transport":event.transport,
            "direction":event.direction, "payload_hex":event.payload_hex.to_ascii_lowercase(),
            "objective":chain.arrival_token, "object_index":request.target_object_id as u16,
            "accepted":accepted, "reason":outcome.err(),
            "vehicle":outcome.ok().map(|r| r.vehicle.object_id()),
            "messages":[&message],
        });
        self.record(audit, wall)?;
        if let Ok(admitted) = outcome {
            runtime
                .arrival_requests
                .insert(chain.arrival_token, admitted);
            self.acquisition_gameplay = Some(runtime);
        }
        Ok(vec![message])
    }

    /// Outer errors are configuration faults; inner errors refuse the request.
    fn admit_arrival_request(
        &self,
        runtime: &Runtime,
        chain: &acquisition_control::Chain,
        now: Duration,
    ) -> Result<Result<ArrivalRequest, &'static str>, &'static str> {
        let active = self.active.as_ref().ok_or("no_reservation")?;
        let ground = self
            .ground_runtime
            .as_ref()
            .ok_or("ground_vehicle_runtime_required")?;
        if !runtime.started() {
            return Ok(Err("acquisition_gameplay_not_started"));
        }
        let CharacterLife::Alive { generation, .. } = active.life else {
            return Ok(Err("acquisition_arrival_actor_not_alive"));
        };
        let Some(index) = self
            .ground_driver_seat_index(active)
            .filter(|&index| self.ground_driver_authority(active, index))
        else {
            return Ok(Err("acquisition_arrival_actor_not_driver"));
        };
        let Some(vehicle) = linked_escort(runtime, ground, chain)? else {
            return Ok(Err("acquisition_arrival_unavailable"));
        };
        if vehicle.index != index {
            return Ok(Err("acquisition_arrival_vehicle_not_linked"));
        }
        Ok(Ok(ArrivalRequest {
            vehicle,
            player: active.reservation.player,
            generation,
            timestamp_ms: now.as_millis() as u32,
        }))
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
