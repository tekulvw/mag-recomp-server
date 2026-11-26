//! Lab receipts for the voice capture (docs/superpowers/specs/2026-11-25-voice-relay-design.md).
//!
//! Off unless `MAG_VOICE_DIAG=1`, and never set on live. While on, the event summary of the
//! talk-channel message (`0x0259`, still otherwise undecoded) carries its body bytes in hex so the
//! lab can confirm the message's layout. Nothing else changes.
use serde_json::{Value, json};
use std::sync::OnceLock;

pub const VARIABLE: &str = "MAG_VOICE_DIAG";
/// The client's talk-channel selection.
pub const CHANNEL_SELECT: u16 = 0x0259;

fn enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var(VARIABLE).is_ok_and(|v| v == "1"))
}

/// The event summary for a message gameplay does not decode.
pub fn unknown_message(message_type: u16, body: &[u8]) -> Value {
    summary(enabled(), message_type, body)
}

fn summary(enabled: bool, message_type: u16, body: &[u8]) -> Value {
    if enabled && message_type == CHANNEL_SELECT {
        let hex: String = body.iter().map(|b| format!("{b:02x}")).collect();
        json!({"type":"unknown","message_type":message_type,"voice_diag_body":hex})
    } else {
        json!({"type":"unknown","message_type":message_type})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn off_keeps_the_summary_unchanged() {
        assert_eq!(
            summary(false, CHANNEL_SELECT, &[1]),
            json!({"type":"unknown","message_type":601})
        );
    }

    #[test]
    fn on_adds_only_the_channel_select_body() {
        assert_eq!(
            summary(true, CHANNEL_SELECT, &[0x00, 0x02, 0xff]),
            json!({"type":"unknown","message_type":601,"voice_diag_body":"0002ff"})
        );
        assert_eq!(
            summary(true, 0x0300, &[7]),
            json!({"type":"unknown","message_type":768})
        );
    }
}
