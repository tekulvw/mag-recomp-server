//! World size versus human capacity.
//!
//! A world's character capacity (humans plus bots, `team_capacity * 2`) and
//! the number of humans it admits are separate limits. Worlds at or below
//! [`LEGACY_WORLD_CHARACTERS`] keep the original rule: every seat may be human.
//! Larger worlds admit at most [`MAX_HUMANS`] humans, [`MAX_HUMANS_PER_TEAM`] per
//! side, and bots fill the remaining seats. Those worlds are only advertised
//! with [`CAPABILITY`], so a Horizon that predates the split never sees one.

/// Advertised in `hello` when a profile row describes a world above
/// [`LEGACY_WORLD_CHARACTERS`]; such rows also carry `human_capacity` and
/// `human_team_capacity`.
pub const CAPABILITY: &str = "world_capacity_v1";

/// The client's Character pool: every Character index is below this.
pub const MAX_WORLD_CHARACTERS: usize = 256;
/// The largest world described without [`CAPABILITY`]. Worlds up to this size
/// admit a human in every seat, as before the split.
pub const LEGACY_WORLD_CHARACTERS: usize = 64;
/// The most humans any world admits: a legacy world's full capacity.
pub const MAX_ADMITTED_HUMANS: usize = LEGACY_WORLD_CHARACTERS;
/// Humans admitted by a world above [`LEGACY_WORLD_CHARACTERS`].
pub const MAX_HUMANS: usize = 32;
/// Humans admitted per side by a world above [`LEGACY_WORLD_CHARACTERS`].
pub const MAX_HUMANS_PER_TEAM: usize = 16;
/// The legacy per-recipient reliable queue bound; see [`shared_queue_limit`].
pub const LEGACY_SHARED_QUEUE: usize = 512;

/// Whether a world of `team_capacity` per side needs [`CAPABILITY`].
pub const fn is_large(team_capacity: usize) -> bool {
    team_capacity * 2 > LEGACY_WORLD_CHARACTERS
}

/// Humans one side admits.
pub fn human_team_capacity(team_capacity: usize) -> usize {
    if is_large(team_capacity) {
        team_capacity.min(MAX_HUMANS_PER_TEAM)
    } else {
        team_capacity
    }
}

/// Humans the whole world admits.
pub fn human_capacity(team_capacity: usize) -> usize {
    if is_large(team_capacity) {
        (team_capacity * 2).min(MAX_HUMANS)
    } else {
        team_capacity * 2
    }
}

/// Per-recipient reliable shared-frame bound. A joiner receives one snapshot
/// per other character while every bot keeps publishing, so the bound grows
/// with the world; legacy worlds keep [`LEGACY_SHARED_QUEUE`].
pub fn shared_queue_limit(world_characters: usize) -> usize {
    LEGACY_SHARED_QUEUE.max(world_characters.min(MAX_WORLD_CHARACTERS) * 2 + 256)
}

/// Character indices bots may take, in preference order.
///
/// Legacy worlds keep the original order: every index below the world size,
/// highest first. Larger worlds use the client's whole Character pool and leave
/// the human range (`0..human_capacity`, where Horizon places humans) for last:
/// a bot only borrows a human index when every other index is taken, which
/// happens only when the world fills the whole pool. A human who joins on a
/// borrowed index makes that bot yield its index.
pub fn bot_index_order(
    team_capacity: usize,
    admission_capacity: usize,
) -> impl Iterator<Item = u16> {
    let (reserved, end) = if is_large(team_capacity) {
        (human_capacity(team_capacity), MAX_WORLD_CHARACTERS)
    } else {
        (0, admission_capacity.min(MAX_WORLD_CHARACTERS))
    };
    let reserved = reserved.min(end);
    (reserved..end)
        .rev()
        .chain((0..reserved).rev())
        .map(|i| i as u16)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn legacy_worlds_admit_a_human_in_every_seat() {
        for team in [1, 4, 16, 32] {
            assert!(!is_large(team));
            assert_eq!(human_team_capacity(team), team);
            assert_eq!(human_capacity(team), team * 2);
            assert_eq!(shared_queue_limit(team * 2), LEGACY_SHARED_QUEUE);
        }
    }

    #[test]
    fn large_worlds_cap_humans_independently_of_world_size() {
        for team in [33, 64, 128] {
            assert!(is_large(team));
            assert_eq!(human_team_capacity(team), MAX_HUMANS_PER_TEAM);
            assert_eq!(human_capacity(team), MAX_HUMANS);
        }
        assert_eq!(shared_queue_limit(128), 512);
        assert_eq!(shared_queue_limit(256), 768);
    }

    #[test]
    fn legacy_bot_indices_are_unchanged() {
        let order: Vec<_> = bot_index_order(32, 64).collect();
        assert_eq!(order, (0..64).rev().collect::<Vec<u16>>());
        let order: Vec<_> = bot_index_order(4, 8).collect();
        assert_eq!(order, (0..8).rev().collect::<Vec<u16>>());
    }

    #[test]
    fn large_worlds_leave_the_human_range_for_last() {
        for team in [64, 128] {
            let order: Vec<_> = bot_index_order(team, team * 2).collect();
            assert_eq!(order.len(), MAX_WORLD_CHARACTERS);
            // Every index above the human range comes first, highest first.
            assert!(order[..224].iter().all(|&i| i >= 32));
            assert_eq!(order[0], 255);
            assert_eq!(order[223], 32);
            // Then the human range, from its top, so borrowed indices are the
            // ones Horizon assigns last.
            assert_eq!(order[224..], (0..32).rev().collect::<Vec<u16>>());
        }
    }
}
