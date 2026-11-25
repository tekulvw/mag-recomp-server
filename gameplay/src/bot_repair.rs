//! Bot engineers repair their side's destroyed structural objectives with the
//! mode's repair tool, through the same repair submission a player's tool
//! sends. The behaviour is specified in docs/bot-repair.md; this is an
//! original design.
use super::*;
use crate::replication::ObjectiveState;
use std::collections::BTreeMap;

/// Population-file policy (`repair`). Omitted fields use these defaults.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Policy {
    pub enabled: bool,
    /// Squad role slots whose bots carry the repair tool in place of their
    /// self-healing item. Empty keeps every bot on its default kit.
    pub engineer_role_slots: Vec<u8>,
    /// How far an engineer goes to repair a destroyed objective of its side.
    pub search_radius_m: f64,
}
impl Default for Policy {
    fn default() -> Self {
        Self {
            enabled: true,
            engineer_role_slots: vec![2],
            search_radius_m: 80.,
        }
    }
}
impl Policy {
    pub(crate) fn validate(&self) -> Result<(), &'static str> {
        if self.engineer_role_slots.iter().any(|&slot| slot >= 8)
            || !self.search_radius_m.is_finite()
            || !(1.0..=2048.).contains(&self.search_radius_m)
        {
            return Err("invalid_bot_repair_policy");
        }
        Ok(())
    }
}

/// An engineer's kit: the default kit with its self-healing item replaced by
/// the repair tool. A kit that already carries the tool, or that has no
/// healing slot, stays unchanged.
pub(crate) fn engineer_loadout(
    policy: &Policy,
    role_slot: u8,
    items: &item_catalog::ItemCatalog,
    tool: Option<u16>,
    mut kit: replication::CharacterLoadout,
) -> replication::CharacterLoadout {
    let Some(tool) = tool.filter(|&tool| {
        policy.enabled
            && policy.engineer_role_slots.contains(&role_slot)
            && items.is_retail_repair_tool(tool)
    }) else {
        return kit;
    };
    let index = usize::from(kit.handle.inventory_index);
    let Some(inventory) = kit.inventories.get_mut(index) else {
        return kit;
    };
    if inventory.slots.contains(&Some(tool)) {
        return kit;
    }
    if let Some(slot) = inventory
        .slots
        .iter_mut()
        .find(|slot| slot.is_some_and(|id| items.healing(id).is_ok()))
    {
        *slot = Some(tool);
    }
    kit
}

pub(crate) fn carries(bot: &bot_registry::WorldBot, tool: u16) -> bool {
    let kit = &bot.target.loadout;
    kit.inventories
        .get(usize::from(kit.handle.inventory_index))
        .is_some_and(|inventory| inventory.slots.contains(&Some(tool)))
}

/// A structural objective destroyed by the other side that its owner may
/// repair, and where its owners stand to work on it.
fn repairable<'a>(
    object: &'a objective_catalog::Objective,
    world: &objective_control::ObjectiveWorld,
) -> Option<(u8, crate::objective_range::ObjectiveRange<'a>)> {
    let rule = object.repair.filter(|r| r.can_repair)?;
    let demolition = object.demolition.as_ref()?;
    (object.native_class == Some(objective_catalog::NativeClass::Structural)
        && matches!(
            world.state(object.object_index),
            Some(ObjectiveState::Completed | ObjectiveState::Repairing)
        ))
    .then(|| (rule.owner_team, demolition.range()))
}

/// The range an engineer stands in to repair `object`, when it may.
pub(crate) fn repair_range<'a>(
    object: &'a objective_catalog::Objective,
    world: &objective_control::ObjectiveWorld,
    team: u8,
) -> Option<crate::objective_range::ObjectiveRange<'a>> {
    repairable(object, world)
        .filter(|(owner, _)| *owner == team)
        .map(|(_, range)| range)
}

impl<W: Write> Engine<W> {
    /// The mode's repair tool, when this mode supports structural repair.
    pub(crate) fn bot_repair_tool(&self) -> Option<structure_catalog::RepairParameters> {
        // Only Domination submits bot repairs so far.
        if self.match_profile().game_type != 2 {
            return None;
        }
        Some(self.bunker_world.as_ref()?.repair_tool_profile())
    }

    /// The nearest free engineer of the owning side goes to each destroyed
    /// repairable objective within the search radius.
    pub(crate) fn plan_repair_duties(&self) -> BTreeMap<u16, u16> {
        let mut duties = BTreeMap::new();
        let Some(policy) = self
            .bot_registry
            .config
            .as_ref()
            .map(|c| c.repair.clone())
            .filter(|p| p.enabled)
        else {
            return duties;
        };
        let (Some(tool), Some(catalog), Some(world)) = (
            self.bot_repair_tool(),
            &self.objective_catalog,
            &self.objective_world,
        ) else {
            return duties;
        };
        for object in &catalog.objects {
            let Some((owner, range)) = repairable(object, world) else {
                continue;
            };
            let anchor = range.anchor();
            let nearest = self
                .bot_registry
                .bots
                .iter()
                .filter(|(id, bot)| {
                    !duties.contains_key(*id)
                        && bot.health > 0
                        && bot.target.squad.team() == owner
                        && !bot.has_supply_action()
                        && carries(bot, tool.item_id)
                })
                .filter_map(|(&id, bot)| {
                    let p = bot.target.runtime?.physical_position();
                    let d = (p[0] - anchor[0]).hypot(p[2] - anchor[2]);
                    (d <= policy.search_radius_m).then_some((d, id))
                })
                .min_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            if let Some((_, id)) = nearest {
                duties.insert(id, object.object_index);
            }
        }
        duties
    }
}
