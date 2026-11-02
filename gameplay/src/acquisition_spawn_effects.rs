//! Existing-vehicle respawn with concrete ordered collision/damage consumers.
//! Preparing restores all live authority; one record precedes every install.
use crate::{
    acquisition_character_effects::{self as character, NativeState},
    acquisition_ground_runtime::{self as ground, Effect},
    acquisition_world::{Filters, RootKey, Settings, World},
    dynamic_collision::Pose,
    *,
};
use std::collections::BTreeMap;

pub struct PreparedRespawn {
    before: ground::Runtime,
    after: ground::Runtime,
    characters: character::Candidate,
    audit: Value,
    packet: Option<Vec<u8>>,
}
pub struct RespawnRequest {
    pub source: ground::Key,
    pub point: ground::QualifiedRespawn,
    pub initial: bool,
    pub physics: Settings,
    pub filter_rows: [u32; 32],
    pub now: Duration,
}
impl PreparedRespawn {
    pub fn audit(&self) -> &Value {
        &self.audit
    }
}

impl<W: Write> Engine<W> {
    /// Explicit source-bound capabilities, installed before admission. Identity,
    /// initial pose and maximum HP remain those of the qualified ground catalog.
    pub fn with_acquisition_ground_capabilities(
        mut self,
        bindings: Vec<(u16, ground::Capabilities)>,
    ) -> Result<Self, &'static str> {
        if self.active.is_some()
            || !self.peers.is_empty()
            || bindings.is_empty()
            || bindings.len() > 32
        {
            return Err("ground_capabilities_require_idle_configuration");
        }
        let runtime = self
            .ground_runtime
            .as_mut()
            .ok_or("ground_vehicle_runtime_required")?;
        let mut ids = std::collections::BTreeSet::new();
        for (index, capabilities) in bindings {
            if !ids.insert(index) {
                return Err("duplicate_ground_capabilities");
            }
            let key = runtime.get(index).ok_or("unknown_ground_vehicle")?.key;
            runtime.configure(key, capabilities)?;
        }
        Ok(self)
    }
    pub fn acquisition_ground_state(&self) -> Option<&ground::Runtime> {
        self.ground_runtime.as_ref()
    }

    /// Source is the exact current qualified source point/final placed pose.
    /// The existing allocation is reused. This does not enable an objective,
    /// synthesize its state-entry hooks, allocate a vehicle or invent a timer.
    pub fn prepare_existing_acquisition_respawn(
        &mut self,
        request: RespawnRequest,
        native: &BTreeMap<u32, NativeState>,
    ) -> Result<PreparedRespawn, &'static str> {
        let RespawnRequest {
            source,
            point,
            initial,
            physics,
            filter_rows,
            now,
        } = request;
        if self.failed || self.match_profile().game_type != 1 {
            return Err("acquisition_respawn_unavailable");
        }
        let before = self
            .ground_runtime
            .as_ref()
            .ok_or("ground_vehicle_runtime_required")?
            .clone();
        let mut after = before.clone();
        let events = after.respawn_existing(source, point, initial)?;
        let clear = events.iter().any(|e| e.effect == Effect::KillSpawnBlockers);
        // Ground state is temporarily owned workspace just like the character
        // capsule. Every normal error restores it before returning.
        self.ground_runtime = Some(after);
        let mut changes = BTreeMap::<u16, u32>::new();
        for event in &events {
            let field = match event.effect {
                Effect::ForcePosition(_) => 1 << 25,
                Effect::ForceRotation(_) => 1 << 26,
                Effect::DamageReset => 1 << 18,
                Effect::StateChanged(_) => 1 << 5,
                _ => 0,
            };
            if field != 0 {
                *changes.entry(event.vehicle.index).or_default() |= field;
            }
        }
        let characters=self.prepare_acquisition_character_callback(native,now,|engine,native| {
            if !clear {return Ok((vec![],vec![]));}
            let mut audit=Vec::new();let mut messages=Vec::new();
            let current=engine.current_ground_vehicle(source.index).ok_or("ground_vehicle_runtime_required")?;
            let statics=engine.collision_world.clone().ok_or("world_cast_static_geometry_unavailable")?;
            if engine.spawn_catalog.as_ref().is_none_or(|s|s.map_name()!=statics.metadata().map_name) {return Err("world_cast_map_mismatch");}
            let snapshot=engine.dynamic_collision_snapshot(filter_rows[6])?;
            let filters=Filters::owned_entities(&snapshot,&statics,filter_rows)?;
            let mut world=World::new(&snapshot,&statics,filters,physics)?;
            let world_id=snapshot.context().0.world_id;
            let target=acquisition_volume_damage::Target{key:acquisition_volume_damage::TargetKey{id:u64::from(source.object_id()),generation:source.generation},kind:acquisition_volume_damage::TargetKind::GroundVehicle,team:current.team};
            let root=RootKey{entity:target.key.id,generation:source.generation,root:"chassis".into()};
            if !world.has_root(&root) {
                let resources=engine.dynamic_body_resources.as_ref().ok_or("dynamic_body_resources_unavailable")?;
                let transform=vehicle_exit::BunkerTransform{position:current.position,rotation_xyzw:current.rotation};
                let columns=[[1.,0.,0.],[0.,1.,0.],[0.,0.,1.]].map(|v|transform.rotate(v));
                let pose=Pose{translation:[current.position.x.into(),current.position.y.into(),current.position.z.into()],rotation:std::array::from_fn(|i|std::array::from_fn(|j|columns[j][i].into()))};
                world.add_allocated_spawn(&resources.catalog,resources.vehicle_template(&current)?,target,pose,"chassis")?;
            }
            let pairs=world.spawn_pairs(&root)?;
            let mut published=BTreeMap::new();
            for (key,target) in pairs {
                if target.key.id==u64::from(source.object_id()) {continue;}
                match target.kind {
                    acquisition_volume_damage::TargetKind::Character=>{
                        let resolved=acquisition_spawn_blockers::resolve_character(&mut world.character(key,root.clone())?)?;
                        let victim=character::Target{character_id:u32::try_from(target.key.id).map_err(|_|"invalid_acquisition_character_identity")?,generation:target.key.generation,world:world_id};
                        if let Some((_,_,p))=world.moved_positions().find(|(id,g,_)|*id==target.key.id && *g==target.key.generation)
                            && published.get(&target.key.id)!=Some(&p) {
                                let position=replication::CharacterPosition{x:p[0] as f32,y:p[1] as f32,z:p[2] as f32};
                                audit.push(engine.apply_acquisition_character_move(victim,position,now)?);published.insert(target.key.id,p);
                        }
                        if !resolved {
                            let vector=engine.acquisition_character_position(victim)?;
                            let state=native.get_mut(&victim.character_id).ok_or("acquisition_character_native_state_missing")?;
                            let plan=engine.plan_acquisition_character_effect(victim,character::Request::VolumeLethal{victim_vector:vector},*state,now)?;
                            audit.push(plan.audit().clone());messages.extend(engine.commit_acquisition_character_effect(plan,state)?);
                        }
                        let dead=engine.acquisition_character_is_dead(victim)?;
                        world.character_dead(target,dead)?;
                    },
                    acquisition_volume_damage::TargetKind::GroundVehicle=>{
                        let index=u16::try_from(target.key.id&0xffff).map_err(|_|"invalid_ground_vehicle_identity")?;
                        let runtime=engine.ground_runtime.as_mut().ok_or("ground_vehicle_runtime_required")?;
                        let current=runtime.get(index).filter(|v|v.key.generation==target.key.generation).ok_or("stale_ground_vehicle_key")?;
                        let vehicle_key=current.key;
                        if current.state==2 && world.contact_between(&key,&root)?.signed_distance < -0.1_f32 {
                            let effects=runtime.apply_lethal(vehicle_key,u32::MAX,true)?;
                            if !effects.is_empty() {*changes.entry(index).or_default()|=1<<18;}
                            audit.push(json!({"ground_lethal":index,"generation":vehicle_key.generation,"vaporize":true}));
                        }
                    },
                    _=>return Err("world_query_spawn_target_unsupported"),
                }
            }
            Ok((audit,messages))
        });
        let after = self
            .ground_runtime
            .replace(before.clone())
            .expect("owned ground candidate");
        let characters = characters?;
        let changes: Vec<_> = changes.into_iter().collect();
        let packet = if changes.is_empty() {
            None
        } else {
            Some(ground_vehicles::encode_runtime_delta(
                self.ground_vehicles
                    .as_ref()
                    .ok_or("ground_vehicle_catalog_required")?,
                &after,
                &changes,
                2,
                u32::try_from(now.as_millis()).map_err(|_| "acquisition_time_out_of_range")?,
            )?)
        };
        let audit = json!({"op":"acquisition_existing_vehicle_respawn","source_index":source.index,"source_generation":source.generation,"initial":initial,
            "source_events":events.iter().map(|e|json!({"index":e.vehicle.index,"generation":e.vehicle.generation,"effect":format!("{:?}",e.effect)})).collect::<Vec<_>>(),
            "character_effects":characters.audit(),"ground_update":packet.as_ref().map(|p|protocol::encode_hex(p))});
        Ok(PreparedRespawn {
            before,
            after,
            characters,
            audit,
            packet,
        })
    }

    pub fn record_existing_acquisition_respawn(
        &mut self,
        prepared: PreparedRespawn,
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
    fn acquisition_character_is_dead(
        &self,
        target: character::Target,
    ) -> Result<bool, &'static str> {
        if let Some(actor) = self.active.iter().chain(self.peers.values()).find(|a| {
            a.reservation.world == target.world
                && (0x03800000 | u32::from(a.reservation.player)) == target.character_id
        }) {
            let (generation, dead) = match actor.life {
                CharacterLife::Alive { generation, .. }
                | CharacterLife::Incapacitated { generation, .. } => (generation, false),
                CharacterLife::AwaitingRespawn { generation } => (generation, true),
                _ => return Err("acquisition_character_has_no_life"),
            };
            if generation != target.generation {
                return Err("stale_acquisition_character_generation");
            }
            return Ok(dead);
        }
        let bot = self
            .bot_registry
            .bots
            .get(&(target.character_id as u16))
            .ok_or("acquisition_character_missing")?;
        if bot
            .target
            .runtime
            .is_none_or(|r| r.generation != target.generation)
        {
            return Err("stale_acquisition_character_generation");
        }
        Ok(bot.health == 0)
    }
    fn acquisition_character_position(
        &self,
        target: character::Target,
    ) -> Result<[f32; 3], &'static str> {
        let p = if let Some(actor) = self.active.iter().chain(self.peers.values()).find(|a| {
            a.reservation.world == target.world
                && (0x03800000 | u32::from(a.reservation.player)) == target.character_id
        }) {
            if let Some(seat) = actor.ground_seat {
                self.current_ground_vehicle(seat.vehicle_index)
                    .ok_or("ground_vehicle_runtime_required")?
                    .position
            } else {
                actor
                    .tracking
                    .sample
                    .ok_or("dynamic_body_character_pose_unavailable")?
                    .position
            }
        } else {
            self.bot_registry
                .bots
                .get(&(target.character_id as u16))
                .and_then(|b| b.target.runtime)
                .ok_or("bot_runtime_missing")?
                .position
        };
        Ok([p.x, p.y, p.z])
    }
    /// Registration phase of ordinary ground Update. Death, inert timers and
    /// reinforcement need their complete consumers and are not advanced here.
    pub(super) fn tick_acquisition_ground_registration(
        &mut self,
        wall: u64,
    ) -> Result<(), &'static str> {
        let Some(runtime) = self.ground_runtime.as_ref() else {
            return Ok(());
        };
        let pending: Vec<_> = runtime
            .vehicles()
            .filter(|v| v.state == 2 && !v.is_dead() && v.body == ground::Body::Inactive)
            .map(|v| (v.key, v.next_spawn_timestamp_ms))
            .collect();
        if pending.is_empty() {
            return Ok(());
        }
        let mut candidate = runtime.clone();
        for &(key, stamp) in &pending {
            let events = candidate.update(key, 0., stamp)?;
            if events.iter().any(|e| e.effect != Effect::AddPhysics) {
                return Err("ground_registration_effect_unsupported");
            }
        }
        self.record(json!({"op":"acquisition_ground_bodies_registered","vehicles":pending.iter().map(|(k,_)|json!({"index":k.index,"generation":k.generation})).collect::<Vec<_>>()}),wall)?;
        self.ground_runtime = Some(candidate);
        Ok(())
    }
}
