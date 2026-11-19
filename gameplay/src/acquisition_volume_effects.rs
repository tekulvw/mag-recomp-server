//! Immediate DamageVolume effects against the Engine's current owned roster.
//! A complete callback can stage these alongside objective, FSP and round state.
use crate::{
    acquisition_character_effects::{self as character, NativeState},
    acquisition_ground_runtime as ground,
    acquisition_volume_damage::{self as damage, Effect, TargetKind},
    dynamic_collision::{QueryBox, QueryLimits},
    *,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug)]
pub struct VolumeRequest {
    pub query: QueryBox,
    pub arguments: damage::Arguments,
    pub row6: u32,
    pub limits: QueryLimits,
    pub now: Duration,
}

pub struct PreparedVolume {
    before: ground::Runtime,
    after: ground::Runtime,
    characters: character::Candidate,
    audit: Value,
    packet: Option<Vec<u8>>,
}
impl PreparedVolume {
    pub fn audit(&self) -> &Value {
        &self.audit
    }
}

impl<W: Write> Engine<W> {
    /// The admitted controller uses zero or negative actor damage. Positive
    /// damage requires its own ordinary metadata/modifier path and is refused.
    /// Mines remain unavailable in Engine, so none may be silently consumed.
    pub fn prepare_acquisition_volume(
        &mut self,
        request: VolumeRequest,
        native: &BTreeMap<u32, NativeState>,
    ) -> Result<PreparedVolume, &'static str> {
        if self.failed || self.match_profile().game_type != 1 {
            return Err("acquisition_volume_unavailable");
        }
        let before = self
            .ground_runtime
            .as_ref()
            .ok_or("ground_vehicle_runtime_required")?
            .clone();
        let mut changes = BTreeMap::new();
        let characters =
            self.prepare_acquisition_character_callback(native, request.now, |engine, native| {
                engine.stage_acquisition_volume(request, native, &mut changes)
            });
        let after = self.ground_runtime.replace(before.clone()).unwrap();
        let characters = characters?;
        let packet = self.encode_ground_changes(&after, &changes, request.now)?;
        let audit = json!({
            "op":"acquisition_volume_applied",
            "timestamp_ms":request.now.as_millis() as u64,
            "row6":request.row6,
            "query":{"position":request.query.pose.translation,"rotation":request.query.pose.rotation,
                "half_extents":request.query.half_extents,"radius":request.query.radius},
            "arguments":{"player":request.arguments.player_damage,"ground":request.arguments.ground_vehicle_damage,
                "mine":request.arguments.mine_damage,"team":request.arguments.apply_to_team},
            "pairs":characters.audit(),
        });
        Ok(PreparedVolume {
            before,
            after,
            characters,
            audit,
            packet,
        })
    }

    /// Source-internal consumer for an already isolated callback. The caller
    /// owns restoring ground and character state on any error and recording
    /// the complete callback before installation. Collector duplicates remain.
    pub(crate) fn stage_acquisition_volume(
        &mut self,
        request: VolumeRequest,
        native: &mut BTreeMap<u32, NativeState>,
        changes: &mut BTreeMap<u16, u32>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        let args = request.arguments;
        if [
            args.player_damage,
            args.ground_vehicle_damage,
            args.mine_damage,
        ]
        .iter()
        .any(|v| !v.is_finite())
        {
            return Err("unsupported_volume_damage_number");
        }
        if args.player_damage > 0. || args.ground_vehicle_damage > 0. {
            return Err("unsupported_acquisition_positive_volume_damage");
        }
        let snapshot = self.dynamic_collision_snapshot(request.row6)?;
        let world = snapshot.context().0.world_id;
        let pairs = snapshot.penetrations(request.query, request.limits)?;
        if pairs.len() > character::MAX_CHARACTER_EFFECTS {
            return Err("acquisition_character_effect_limit");
        }
        let mut audit = Vec::with_capacity(pairs.len());
        let mut messages = Vec::new();
        for pair in pairs {
            let mut row = json!({"id":pair.target.key.id,"generation":pair.target.key.generation,"root":pair.root});
            if let Some(effect) = damage::dispatch(args, pair.target)? {
                match (pair.target.kind, effect.effect) {
                    (TargetKind::Character, Effect::LethalActor) => {
                        let victim = character::Target {
                            character_id: u32::try_from(pair.target.key.id)
                                .map_err(|_| "invalid_acquisition_character_identity")?,
                            generation: pair.target.key.generation,
                            world,
                        };
                        let vector = self.acquisition_character_position(victim)?;
                        let state = native
                            .get_mut(&victim.character_id)
                            .ok_or("acquisition_character_native_state_missing")?;
                        let plan = self.plan_acquisition_character_effect(
                            victim,
                            character::Request::VolumeLethal {
                                victim_vector: vector,
                            },
                            *state,
                            request.now,
                        )?;
                        row["effect"] = plan.audit().clone();
                        messages.extend(self.commit_acquisition_character_effect(plan, state)?);
                    }
                    (TargetKind::GroundVehicle, Effect::LethalActor) => {
                        let id = u32::try_from(pair.target.key.id)
                            .map_err(|_| "invalid_ground_vehicle_identity")?;
                        if id & 0xffff_0000 != 0x0100_0000 {
                            return Err("invalid_ground_vehicle_identity");
                        }
                        let key = ground::Key {
                            index: id as u16,
                            generation: pair.target.key.generation,
                        };
                        let effects = self
                            .ground_runtime
                            .as_mut()
                            .ok_or("ground_vehicle_runtime_required")?
                            .apply_lethal(key, u32::MAX, true)?;
                        if !effects.is_empty() {
                            *changes.entry(key.index).or_default() |= 1 << 18;
                        }
                        row["effect"] = json!({"ground_lethal":key.index,"vaporize":true});
                    }
                    _ => return Err("unsupported_acquisition_volume_effect"),
                }
            }
            audit.push(row);
        }
        Ok((audit, messages))
    }

    pub(crate) fn encode_ground_changes(
        &self,
        runtime: &ground::Runtime,
        changes: &BTreeMap<u16, u32>,
        now: Duration,
    ) -> Result<Option<Vec<u8>>, &'static str> {
        if changes.is_empty() {
            return Ok(None);
        }
        Ok(Some(ground_vehicles::encode_runtime_delta(
            self.ground_vehicles
                .as_ref()
                .ok_or("ground_vehicle_catalog_required")?,
            runtime,
            &changes
                .iter()
                .map(|(&index, &fields)| (index, fields))
                .collect::<Vec<_>>(),
            2,
            u32::try_from(now.as_millis()).map_err(|_| "acquisition_time_out_of_range")?,
        )?))
    }

    pub fn record_acquisition_volume(
        &mut self,
        prepared: PreparedVolume,
        native: &mut BTreeMap<u32, NativeState>,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        if self.ground_runtime.as_ref() != Some(&prepared.before) {
            return Err("stale_acquisition_ground_candidate");
        }
        self.check_acquisition_character_effects(&prepared.characters, native)?;
        self.record(prepared.audit, wall)?;
        let messages = self.install_character_effects_after_record(prepared.characters, native);
        self.ground_runtime = Some(prepared.after);
        if let Some(packet) = prepared.packet {
            self.broadcast_bot_packet(packet);
        }
        Ok(messages)
    }
}
