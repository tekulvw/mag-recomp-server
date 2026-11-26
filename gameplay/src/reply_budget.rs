//! Bridge reply budgets for shared replication.
//!
//! Every reply (world frame or event reply) may carry queued shared frames for
//! any ready recipient. The legacy budget is shared by all recipients: 128
//! messages (32 in worlds of at most 8 players) and 24,000 payload bytes per
//! reply, which Horizon enforces. With 32 humans in a 128- or 256-character
//! world, that is four messages per recipient per world frame, less than a
//! joiner's bot snapshots and the bots' movement need.
//!
//! `--per-recipient-reply-budget` gives each recipient its own allowance of
//! shared frames in every reply instead, within larger reply totals, and the
//! worker advertises [`CAPABILITY`] in `hello` so Horizon accepts them. Direct
//! replies to an event keep the legacy bound.

/// Advertised in `hello` when replies may use [`PER_RECIPIENT`].
pub const CAPABILITY: &str = "reply_budget_per_recipient_v1";

/// The legacy total payload bytes per reply.
pub const LEGACY_PAYLOAD_BYTES: usize = 24_000;

/// Limits on the shared frames one reply carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Budget {
    /// Messages per reply, direct and shared together.
    pub messages: usize,
    /// Payload bytes per reply, direct and shared together.
    pub payload_bytes: usize,
    /// Shared messages and payload bytes one recipient receives per reply,
    /// when recipients have their own allowance.
    pub recipient: Option<(usize, usize)>,
}

/// Per-recipient budget totals. They hold 32 recipients' full allowances
/// plus a legacy direct reply. Horizon accepts up to 1,048,576 response bytes
/// with [`CAPABILITY`]; this payload in hex with per-message fields stays
/// well below that.
pub const PER_RECIPIENT: Budget = Budget {
    messages: 1024,
    payload_bytes: 262_144,
    recipient: Some((16, 6_144)),
};

impl Budget {
    /// The legacy shared budget for a world admitting `maximum_players`.
    pub const fn legacy(maximum_players: usize) -> Self {
        Self {
            messages: if maximum_players > 8 { 128 } else { 32 },
            payload_bytes: LEGACY_PAYLOAD_BYTES,
            recipient: None,
        }
    }
}
