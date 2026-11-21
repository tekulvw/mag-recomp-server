//! Ordinary on-foot Character effects from an authoritative hospital rocket.
use super::*;
use crate::{hospital_rocket_runtime::LaunchSource, hospital_rockets::Blast};

impl<W: Write> Engine<W> {
    /// The Character candidate carries health, statistics, rewards and reliable
    /// packets. The flight caller records it together with the terminal state
    /// and installs it only after the shared record succeeds.
    pub(crate) fn prepare_hospital_rocket_character_blast(
        &mut self,
        source: &LaunchSource,
        origin: [f64; 3],
        blast: Blast,
        collision: BlastCollision<'_>,
        now: Duration,
    ) -> Result<(character::Candidate, BTreeMap<u32, NativeState>), &'static str> {
        let attacker = self.hospital_rocket_blast_attacker(source)?;
        let effects = self
            .collect_hospital_rocket_character_blast(source, attacker, origin, blast, collision)?;
        let native = self.owned_acquisition_characters();
        let candidate = self.prepare_acquisition_character_effects(&effects, &native, now)?;
        Ok((candidate, native))
    }

    pub(super) fn hospital_rocket_blast_attacker(
        &self,
        source: &LaunchSource,
    ) -> Result<u32, &'static str> {
        if self.round_number != source.round
            || !self.active.iter().chain(self.peers.values()).any(|actor| {
                actor.reservation.id == source.reservation
                    && actor.reservation.world == source.world
                    && actor.reservation.player == source.player
                    && actor.reward_match_id == source.match_id
                    && actor.reservation.character.character_id == source.character_id
                    && actor.team() == source.team
            })
        {
            return Err("stale_hospital_rocket_character_source");
        }
        // Death or respawn of the same authenticated reservation does not
        // transfer credit. Release retires the flight at the owning boundary.
        Ok(0x0380_0000 | u32::from(source.player))
    }

    fn collect_hospital_rocket_character_blast(
        &self,
        source: &LaunchSource,
        attacker: u32,
        origin: [f64; 3],
        blast: Blast,
        collision: BlastCollision<'_>,
    ) -> Result<Vec<(character::Target, character::Request)>, &'static str> {
        if !blast.character_hp.is_finite() || blast.character_hp <= 0. {
            return Err("invalid_hospital_rocket_character_blast");
        }
        let shape = CharacterBlastShape {
            origin,
            half_radius_m: blast.half_radius_m,
            outer_radius_m: blast.outer_radius_m,
            material_penetration: blast.material_penetration,
        };
        let snapshot = self.dynamic_collision_snapshot(collision.filter_rows[6])?;
        Ok(self
            .collect_character_blast(
                source.world,
                shape,
                collision,
                &snapshot,
                OnFootBlastPose::default(),
            )?
            .into_iter()
            .map(|sample| {
                (
                    sample.target,
                    character::Request::HospitalRocketExplosion {
                        raw_hp: blast.character_hp * sample.scale,
                        from_direction: sample.direction,
                        attacker,
                        projectile_id: source.projectile_id,
                    },
                )
            })
            .collect())
    }

    pub(crate) fn collect_character_blast(
        &self,
        world: u32,
        shape: CharacterBlastShape,
        collision: BlastCollision<'_>,
        snapshot: &dynamic_collision::Snapshot,
        accepted: OnFootBlastPose,
    ) -> Result<Vec<CharacterBlastObservation>, &'static str> {
        let origin = shape.origin;
        let origin_f32 = origin.map(|component| component as f32);
        if !origin_f32.into_iter().all(f32::is_finite)
            || !shape.half_radius_m.is_finite()
            || !shape.outer_radius_m.is_finite()
            || shape.half_radius_m <= 0.
            || shape.outer_radius_m <= shape.half_radius_m
            || !shape.material_penetration.is_finite()
            || shape.material_penetration < 0.
        {
            return Err("invalid_character_blast_shape");
        }
        if snapshot.context().0.world_id != world {
            return Err("stale_character_blast_world");
        }
        let scene = self
            .collision_world
            .as_ref()
            .filter(|scene| {
                scene.stats().triangles != 0 && scene.metadata().map_name == self.map_name()
            })
            .ok_or("character_blast_static_collision_required")?;
        let sphere = Geometry::Sphere {
            radius: f64::from(shape.outer_radius_m),
        };
        let mut overlap_budget = collision.limits.max_support_points;
        let mut static_budget = 1_000_000;
        let mut effects = Vec::new();
        let mut admitted = BTreeSet::new();
        for body in snapshot.bodies().iter().filter(|body| {
            body.target.kind == TargetKind::Character
                && collision.filter_rows[6] & (1 << body.layer) != 0
        }) {
            let key = (body.target.key.id, body.target.key.generation);
            if admitted.contains(&key)
                || !geometry::penetrates(
                    &sphere,
                    Pose::at(origin),
                    &body.shape,
                    body.pose,
                    collision.limits,
                    &mut overlap_budget,
                )?
            {
                continue;
            }
            let Some((target, anchor)) =
                self.domination_character_blast_anchor(body.target, world, accepted)?
            else {
                continue;
            };
            let (displacement, distance_squared) = separation(origin_f32, anchor)?;
            let distance = distance_squared.sqrt();
            let scale =
                armor::native_radial_scale(distance, shape.half_radius_m, shape.outer_radius_m)?;
            if scale <= 0.
                || !collision.clear_path(
                    scene,
                    snapshot,
                    BlastRay {
                        start: origin,
                        end: anchor.map(f64::from),
                        excludes: [Some(body.target.key.id), None],
                        penetration: shape.material_penetration,
                        zero_penetration_strike: false,
                        filter_static: true,
                    },
                    &mut static_budget,
                )?
            {
                continue;
            }
            admitted.insert(key);
            effects.push(CharacterBlastObservation {
                target,
                distance,
                scale,
                direction: direction(displacement, distance_squared),
            });
        }
        Ok(effects)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rocket_character_dose_keeps_attribution_after_death_but_rejects_stale_source() {
        let mut engine = crate::human_kill_reward_tests::fixture();
        let actor = engine.active.as_mut().unwrap();
        let source = LaunchSource {
            reservation: actor.reservation.id.clone(),
            world: actor.reservation.world,
            round: engine.round_number,
            match_id: actor.reward_match_id.clone(),
            character_id: actor.reservation.character.character_id,
            player: actor.reservation.player,
            life_generation: 4,
            team: actor.team(),
            item: 3,
            projectile_id: 0x1380_0001,
        };
        actor.life = CharacterLife::AwaitingRespawn { generation: 4 };
        assert_eq!(
            engine.hospital_rocket_blast_attacker(&source),
            Ok(0x0380_0001)
        );
        for wrong in [
            LaunchSource {
                round: source.round + 1,
                ..source.clone()
            },
            LaunchSource {
                team: source.team ^ 1,
                ..source.clone()
            },
            LaunchSource {
                reservation: "replaced-player".into(),
                ..source.clone()
            },
            LaunchSource {
                match_id: "previous-match".into(),
                ..source.clone()
            },
        ] {
            assert_eq!(
                engine.hospital_rocket_blast_attacker(&wrong),
                Err("stale_hospital_rocket_character_source")
            );
        }
        engine.active = None;
        assert_eq!(
            engine.hospital_rocket_blast_attacker(&source),
            Err("stale_hospital_rocket_character_source")
        );
    }

    #[test]
    fn rocket_character_dose_cover_respects_both_layer_filter_and_material_cost() {
        let scene = collision::CollisionWorld::from_bytes(
            &serde_json::to_vec(&json!({
                "format":"mag_collision_world", "schema_version":1, "coordinate_space":"world",
                "metadata":{"map_name":"synthetic","build":"synthetic","coverage":"provisional",
                    "provenance":["synthetic"],"unknowns":[]},
                "instances":[{"id":"covered_face", "material_index":4,
                    "collision":{"filter_info":9,"convex_radius":0.,"child_filter_policy":"uniform","provenance":"synthetic"},
                    "vertices_xyz":[[0.,0.,0.],[3.,0.,0.],[0.,0.,3.]],"triangles":[[0,1,2]]}]
            }))
            .unwrap(),
            Default::default(),
        )
        .unwrap();
        let snapshot = dynamic_collision::Snapshot::capture(
            crate::acquisition_atomic::WorldKey {
                world_id: 1,
                incarnation: 1,
            },
            0,
            &[],
            vec![],
            0,
        )
        .unwrap();
        let mut rows = [0; 32];
        rows[24] = 1 << 9;
        let costs = BTreeMap::from([(4, 1.0)]);
        let collision = BlastCollision {
            filter_rows: &rows,
            limits: Default::default(),
            sequences: &SequenceCollisionState::default(),
            material_costs: &costs,
            terrain_materials: None,
        };
        let ray = BlastRay {
            start: [1., 1., 1.],
            end: [1., -1., 1.],
            excludes: [None; 2],
            penetration: 1.,
            zero_penetration_strike: false,
            filter_static: true,
        };
        assert_eq!(
            collision.clear_path(&scene, &snapshot, ray, &mut 100),
            Ok(false)
        );
        let with_penetration = BlastRay {
            penetration: 1.5,
            ..ray
        };
        assert_eq!(
            collision.clear_path(&scene, &snapshot, with_penetration, &mut 100),
            Ok(true)
        );
        let mut filtered_out = rows;
        filtered_out[24] = 1 << 10;
        let collision = BlastCollision {
            filter_rows: &filtered_out,
            ..collision
        };
        assert_eq!(
            collision.clear_path(&scene, &snapshot, ray, &mut 100),
            Ok(true)
        );
    }
}
