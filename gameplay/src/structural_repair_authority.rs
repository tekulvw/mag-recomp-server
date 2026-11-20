//! Equipped-tool and actor admission shared by native Structural objectives.
use crate::{
    CharacterLife, Engine,
    domination_objective_runtime::Actor,
    domination_repair_runtime::Authority,
    leadership_runtime::LeadershipPlan,
    structure_catalog::{RepairParameters, StructureCatalog},
};
use std::{
    io::{self, Write},
    path::Path,
    time::Duration,
};

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
}

#[cfg(test)]
#[path = "structural_repair_authority_tests.rs"]
pub(crate) mod tests;
