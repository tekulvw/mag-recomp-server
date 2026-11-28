//! Equipped-tool and actor admission shared by native Structural objectives.
use crate::{
    CharacterLife, Engine, OutboundMessage,
    domination_objective_runtime::Actor,
    domination_repair_runtime::Authority,
    events::EventInput,
    leadership_runtime::LeadershipPlan,
    structure_catalog::{RepairParameters, StructureCatalog},
};
use serde_json::json;
use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};

/// Refusals an honest repair tool reaches through timing alone. The tool
/// sends a packet about twice a second while it is held, so the last packet
/// can still be in flight when the repairer dies or mounts, switches away
/// from the tool, the round closes, or the structure completes its repair or
/// otherwise leaves its repairable state. These become audited gameplay
/// denials. Every other refusal, such as an enemy actor, an unknown target or
/// an exhausted sequence, still ends the event as an error.
pub(crate) const STRUCTURAL_REPAIR_RACE_DENIALS: [&str; 5] = [
    "structural_repair_requires_live_actor",
    "structural_repair_requires_combat_actor",
    "repair_inventory_unavailable",
    "repair_tool_not_equipped",
    "structural_objective_not_repairable",
];

pub(crate) fn is_structural_repair_race_denial(code: &str) -> bool {
    STRUCTURAL_REPAIR_RACE_DENIALS.contains(&code)
}

/// The outcome of admitting one client structural repair packet.
pub(crate) enum StructuralRepairAdmission {
    Admitted(Authority, LeadershipPlan),
    Denied(&'static str),
}

pub(crate) fn load_retail_repair_tool(path: impl AsRef<Path>) -> io::Result<RepairParameters> {
    Ok(StructureCatalog::load_for_application(path, 20231)?.repair())
}

impl<W: Write> Engine<W> {
    pub(crate) fn plan_structural_repair_authority(
        &self,
        tool: Option<RepairParameters>,
        now: Duration,
    ) -> Result<(Authority, LeadershipPlan), &'static str> {
        let active = self.active.as_ref().ok_or("no_reservation")?;
        active.next_sequence()?;
        let CharacterLife::Alive { generation, .. } = active.life else {
            return Err("structural_repair_requires_live_actor");
        };
        if !active.entered_game
            || !active.ready_for_updates
            || active.is_mounted()
            || !self.round_combat_allowed(now)
        {
            return Err("structural_repair_requires_combat_actor");
        }
        let tool = tool.ok_or("structural_repair_tool_source_required")?;
        let handle = active
            .current_inventory_handle
            .ok_or("repair_inventory_unavailable")?;
        let equipped = active
            .equipped_loadout()
            .ok_or("repair_inventory_unavailable")?;
        let observed = if active.primary_equipped {
            active.observed_primary
        } else {
            active.observed_secondary
        }
        .ok_or("repair_tool_not_equipped")?;
        if !self.item_catalog.is_retail_repair_tool(tool.item_id)
            || observed.inventory_update_id != handle.update_id
            || equipped
                .inventories
                .get(usize::from(handle.inventory_index))
                .and_then(|inventory| inventory.slots.get(usize::from(observed.gear_slot)))
                .copied()
                .flatten()
                != Some(tool.item_id)
        {
            return Err("repair_tool_not_equipped");
        }
        let leadership = self.plan_leadership(now)?;
        let modifier = leadership.modifiers(active).value("kCharModRepairAmount");
        let authority = Authority {
            actor: Actor {
                player: active.reservation.player,
                generation: generation.into(),
                team: active.team(),
            },
            live: true,
            tool_active: true,
            repair_rate: tool
                .cooldown_seconds
                .mul_add(modifier, tool.cooldown_seconds),
        };
        Ok((authority, leadership))
    }

    /// Admit one client repair packet for a Structural objective. A race
    /// denial from the actor gate or from the objective's own repair state is
    /// returned as `Denied` before any candidate is built, so the caller can
    /// audit it without changing state. The objective check runs on a copy.
    pub(crate) fn admit_structural_repair(
        &self,
        target_object_id: u32,
        tool: Option<RepairParameters>,
        now: Duration,
    ) -> Result<StructuralRepairAdmission, &'static str> {
        let (authority, leadership) = match self.plan_structural_repair_authority(tool, now) {
            Ok(planned) => planned,
            Err(code) if is_structural_repair_race_denial(code) => {
                return Ok(StructuralRepairAdmission::Denied(code));
            }
            Err(code) => return Err(code),
        };
        let mut probe = self
            .objective_world
            .clone()
            .ok_or("objective_world_unavailable")?;
        match probe.stage_repair_packet(target_object_id, authority) {
            Err(code) if is_structural_repair_race_denial(code) => {
                Ok(StructuralRepairAdmission::Denied(code))
            }
            Err(code) => Err(code),
            Ok(_) => Ok(StructuralRepairAdmission::Admitted(authority, leadership)),
        }
    }

    /// Audit a refused structural repair packet and keep the session.
    pub(crate) fn record_structural_repair_denial(
        &mut self,
        op: &'static str,
        event: &EventInput,
        target_object_id: u32,
        code: &'static str,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.record(
            json!({
                "op": op, "timestamp_ms": now.as_millis() as u64,
                "reservation_id": event.reservation_id, "world_id": event.world_id,
                "player_id": event.player_id, "target": target_object_id,
                "transport": event.transport, "direction": event.direction,
                "rt_type": event.rt_type, "plugin_id": event.plugin_id,
                "payload_hex": event.payload_hex.to_ascii_lowercase(),
                "gameplay_denial": code,
            }),
            wall,
        )?;
        self.last_gameplay_denial = Some(code);
        Ok(Vec::new())
    }
}

#[cfg(test)]
#[path = "structural_repair_authority_tests.rs"]
pub(crate) mod tests;
