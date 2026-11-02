//! Human lifecycle authority. Wire model conditions and spawn eligibility are distinct.
use super::*;

impl<W: Write> Engine<W> {
    /// Opt in using caller-loaded rules. No game tuning is embedded or loaded implicitly.
    pub fn with_incapacitation_rules(
        mut self,
        rules: incapacitation::Rules,
    ) -> Result<Self, &'static str> {
        rules.validate()?;
        self.incapacitation_rules = Some(rules);
        Ok(self)
    }

    pub(super) fn plan_human_damage(
        &self,
        victim: &Active,
        amount: f32,
        mut damage: replication::CharacterDamage,
        now: Duration,
    ) -> Result<(replication::CharacterDamage, CharacterLife), &'static str> {
        let modifiers = self.plan_leadership(now)?.modifiers(victim);
        match victim.life {
            CharacterLife::Alive { generation, .. } => {
                let mut life = if damage.health_quarter_points == 0 {
                    CharacterLife::AwaitingRespawn { generation }
                } else {
                    CharacterLife::Alive {
                        generation,
                        health_quarter_points: damage.health_quarter_points,
                    }
                };
                if damage.health_quarter_points == 0
                    && victim.mounted_seat.is_none()
                    && victim.wire_profile.application_id() == 20231
                    && let Some(rules) = &self.incapacitation_rules
                    && let Some(downed) = rules.admit(
                        amount,
                        victim.owner_fields.state.map_or(0, |s| u16::from(s.state)),
                        modifiers.value("kCharModIncapacitatedTime"),
                        now,
                    )?
                {
                    damage.conditions |= 2;
                    life = CharacterLife::Incapacitated { generation, downed };
                }
                Ok((damage, life))
            }
            CharacterLife::Incapacitated {
                generation,
                mut downed,
            } => {
                let rules = self
                    .incapacitation_rules
                    .as_ref()
                    .ok_or("incapacitation_rules_missing")?;
                // Later hits consume the downed damage budget, not the original
                // collapse attribution. Expiry wins if the deadline is already due.
                let incoming_type = damage.last_location_and_type & 0xf0;
                damage = victim.last_damage;
                let expired = downed.is_expired(now);
                damage.last_location_and_type = (if expired { 0 } else { incoming_type })
                    | (damage.last_location_and_type & 15);
                let terminal = expired || downed.apply_damage(rules, amount)?;
                damage.conditions = if terminal {
                    damage.conditions & !2
                } else {
                    damage.conditions | 2
                };
                Ok((
                    damage,
                    if terminal {
                        CharacterLife::AwaitingRespawn { generation }
                    } else {
                        CharacterLife::Incapacitated { generation, downed }
                    },
                ))
            }
            _ => Err("damage_target_not_living_or_incapacitated"),
        }
    }

    /// The world clock owns expiry; an idle/disconnected owner cannot postpone it.
    pub(super) fn tick_incapacitation(
        &mut self,
        now: Duration,
        wall: u64,
    ) -> Result<(), &'static str> {
        let mut due = Vec::new();
        for victim in self.active.iter().chain(self.peers.values()) {
            if let CharacterLife::Incapacitated { generation, downed } = victim.life
                && downed.is_expired(now)
            {
                let damage = replication::CharacterDamage {
                    conditions: victim.last_damage.conditions & !3,
                    last_location_and_type: victim.last_damage.last_location_and_type & 15,
                    ..victim.last_damage
                };
                let packet = replication::CharacterDamageUpdate {
                    sequence: 2,
                    timestamp_ms: now.as_millis() as u32,
                    object_index: victim.reservation.player,
                    damage,
                    forced_state: None,
                }
                .encode_with_profile(victim.wire_profile)?;
                due.push((victim.reservation.id.clone(), generation, damage, packet));
            }
        }
        if due.is_empty() {
            return Ok(());
        }
        self.record(json!({"op":"incapacitation_expired","timestamp_ms":now.as_millis() as u64,
            "patients":due.iter().map(|(id,g,_,_)|json!({"reservation_id":id,"generation":g})).collect::<Vec<_>>() }),wall)?;
        for (id, generation, damage, packet) in due {
            let victim = self
                .active
                .as_mut()
                .filter(|a| a.reservation.id == id)
                .or_else(|| self.peers.get_mut(&id))
                .ok_or("stale_incapacitation_expiry")?;
            victim.apply_peer_damage(generation, damage, now)?;
            // Queued shared packets are resequenced per recipient on delivery.
            self.broadcast_bot_packet(packet);
        }
        Ok(())
    }
}
impl Active {
    /// Replicate the downed owner's presentation, without giving its position,
    /// equipment or projectiles authority. After revival ignore in-flight death
    /// state reports until the owner publishes its first recovery state.
    pub(super) fn owner_presentation(
        &self,
        fields: &player_tracking::ClientCharacterFields,
    ) -> player_tracking::ClientCharacterFields {
        match self.life {
            CharacterLife::Incapacitated { .. } => player_tracking::ClientCharacterFields {
                state: fields.state.filter(|s| s.state == 7),
                pitch: fields.pitch,
                yaw: fields.yaw,
                ..Default::default()
            },
            CharacterLife::Alive { .. } => {
                let mut fields = fields.clone();
                if self.revive_state_pending {
                    fields.state = fields.state.filter(|s| s.state != 7);
                }
                fields
            }
            _ => Default::default(),
        }
    }
    pub(super) fn commit_revive(
        &mut self,
        generation: u32,
        damage: replication::CharacterDamage,
    ) -> Result<(), &'static str> {
        if !matches!(self.life,CharacterLife::Incapacitated{generation:g,..} if g==generation)
            || damage.health_quarter_points == 0
            || damage.conditions & 2 != 0
        {
            return Err("stale_revive_patient");
        }
        self.life = CharacterLife::Alive {
            generation,
            health_quarter_points: damage.health_quarter_points,
        };
        self.last_damage = damage;
        self.revive_state_pending = true;
        self.healing = None;
        self.pending_damage = None;
        self.pending_spawn_deadline = None;
        self.published_wave_deadline = None;
        self.last_exit = None;
        // No forced field4, spawn trigger, inventory reset or generation change.
        // The living owner publishes its recovery state through its normal stream.
        Ok(())
    }
}
