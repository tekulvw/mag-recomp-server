//! Apply due reinforcement waves inside the owned Acquisition transaction.
use crate::{
    acquisition_character_effects::NativeState,
    acquisition_gameplay_engine::Runtime,
    acquisition_ground_runtime::{self as ground, Effect},
    acquisition_spawn_effects::Clearance,
    acquisition_vehicle_waves::VehicleWaves,
    *,
};
use std::collections::BTreeMap;

pub(crate) struct WaveFrame<'a> {
    pub waves: &'a mut VehicleWaves,
    pub physics: crate::acquisition_world::Settings,
    pub filter_rows: [u32; 32],
    pub staged_spawns: Option<&'a crate::spawn_catalog::SpawnCatalog>,
    pub now: Duration,
}

impl<W: Write> Engine<W> {
    pub(crate) fn stage_acquisition_vehicle_waves(
        &mut self,
        runtime: &mut Runtime,
        ground: &mut ground::Runtime,
        now: Duration,
        native: &mut BTreeMap<u32, NativeState>,
        changes: &mut BTreeMap<u16, u32>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        let mut audit = Vec::new();
        if runtime.vehicle_waves.is_none() {
            let epoch = self.match_epoch.unwrap_or(now).as_millis() as u32;
            runtime.vehicle_waves = Some(VehicleWaves::start(
                epoch,
                runtime.setup.vehicle_reinforcement_intervals_ms,
            )?);
            audit.push(json!({"op":"acquisition_vehicle_waves_started","epoch_ms":epoch}));
        }
        let (rows, messages) = self.stage_ground_vehicle_waves(
            WaveFrame {
                waves: runtime.vehicle_waves.as_mut().unwrap(),
                physics: runtime.setup.physics,
                filter_rows: runtime.setup.filter_rows,
                staged_spawns: None,
                now,
            },
            ground,
            native,
            changes,
        )?;
        audit.extend(rows);
        Ok((audit, messages))
    }

    pub(crate) fn stage_ground_vehicle_waves(
        &mut self,
        frame: WaveFrame<'_>,
        ground: &mut ground::Runtime,
        native: &mut BTreeMap<u32, NativeState>,
        changes: &mut BTreeMap<u16, u32>,
    ) -> Result<(Vec<Value>, Vec<OutboundMessage>), &'static str> {
        let WaveFrame {
            waves,
            physics,
            filter_rows,
            staged_spawns,
            now,
        } = frame;
        let mut audit = Vec::new();
        let mut messages = Vec::new();
        for (team, due) in waves.tick(now.as_millis() as u32).into_iter().enumerate() {
            let Some(queued) = due else { continue };
            let team = team as u8;
            audit.push(json!({
                "op":"acquisition_vehicle_wave", "team":team,
                "next_spawn_timestamp_ms":waves.next_timestamp(team)?,
                "queued":queued.iter().map(|key| key.object_id()).collect::<Vec<_>>(),
            }));
            for key in queued {
                let catalog = self
                    .ground_vehicles
                    .as_ref()
                    .ok_or("ground_vehicle_catalog_required")?;
                let creation = catalog.vehicle(key.index).ok_or("unknown_ground_vehicle")?;
                let current = ground.get(key.index).ok_or("unknown_ground_vehicle")?;
                let source_type = catalog.source_spawn_type(creation);
                if creation.team != team
                    || current.key != key
                    || current.state != 6
                    || !matches!(source_type, Some(3 | 7))
                    || current.source_spawn_type != source_type
                {
                    return Err("acquisition_vehicle_wave_stale_entry");
                }
                let spawns = staged_spawns
                    .or(self.spawn_catalog.as_ref())
                    .ok_or("acquisition_vehicle_wave_source_missing")?;
                let point = spawns
                    .point(creation.spawn_point_index)
                    .ok_or("acquisition_vehicle_wave_source_missing")?;
                if point.index != current.source_spawn_point_index
                    || point.token != current.source_spawn_point_token
                    || point.token != creation.spawn_point_token
                {
                    return Err("acquisition_vehicle_wave_source_mismatch");
                }
                let active =
                    staged_spawns.map_or(point.active, |spawns| spawns.point_active(point.index));
                if source_type == Some(7) && !active {
                    waves.queue(team, key)?;
                    audit.push(json!({"op":"acquisition_vehicle_wave_source_inactive","vehicle":key.object_id()}));
                    continue;
                }
                let source = ground::QualifiedRespawn {
                    spawn_point_index: point.index,
                    spawn_point_token: point.token,
                    active,
                    pose: ground::Pose {
                        position: creation.position,
                        rotation: creation.rotation,
                    },
                };
                for event in ground.respawn_existing(key, source, false)? {
                    audit.push(json!({"op":"acquisition_vehicle_wave_effect",
                        "vehicle":key.object_id(),"effect":format!("{:?}",event.effect)}));
                    let mask = match event.effect {
                        Effect::ForcePosition(_) => 1 << 25,
                        Effect::ForceRotation(_) => 1 << 26,
                        Effect::DamageReset => 1 << 18,
                        Effect::StateChanged(_) => 1 << 5,
                        Effect::KillSpawnBlockers => {
                            let previous = self
                                .ground_runtime
                                .take()
                                .ok_or("ground_vehicle_runtime_required")?;
                            self.ground_runtime = Some(ground.clone());
                            let result = self.stage_respawn_clearance(
                                Clearance {
                                    source: key,
                                    physics,
                                    filter_rows,
                                    now,
                                },
                                native,
                                changes,
                            );
                            *ground = self
                                .ground_runtime
                                .replace(previous)
                                .ok_or("ground_vehicle_runtime_required")?;
                            let (rows, packets) = result?;
                            audit.extend(rows);
                            messages.extend(packets);
                            0
                        }
                        Effect::DeactivateHulk | Effect::SpawnCounterIncremented(_) => 0,
                        _ => return Err("unexpected_acquisition_vehicle_wave_effect"),
                    };
                    if mask != 0 {
                        *changes.entry(key.index).or_default() |= mask;
                    }
                }
            }
        }
        Ok((audit, messages))
    }
}
