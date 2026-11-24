//! Resumable world-stream frames and replayable event replies.
//!
//! Horizon receives world frames on one long-lived connection and answers each
//! client event on its own short connection. When either connection breaks,
//! Horizon cannot know what the worker already produced. Without help it must
//! release every player on a stream gap and abandon an unanswered event.
//!
//! A served worker therefore numbers each world frame it sends, within an epoch
//! drawn once per process, and keeps the most recent frames in a bounded
//! journal. A subscriber that asks for positions receives them; one that
//! reconnects to the same process may resume after the last frame it handled
//! and receives exactly the frames it missed, or a refusal when the epoch
//! differs (another process) or the journal no longer holds them. Frames are
//! produced only while a subscriber is registered, so the journal only needs
//! to cover what was in flight when a stream broke.
//!
//! Event replies are kept by request ID for a short while so Horizon can retry
//! an unanswered event with the same ID: the worker applies it once and
//! answers every attempt with the same reply.
use crate::Response;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, VecDeque},
    io::{self, Read},
};

pub const CAPABILITY: &str = "world_stream_resume_v1";
pub const EVENT_REPLAY_CAPABILITY: &str = "event_replay_v1";
pub const RESUME_UNAVAILABLE: &str = "world_resume_unavailable";

/// Frames retained for a resuming subscriber (about 25 seconds at 20 Hz).
pub const JOURNAL_FRAMES: usize = 512;
/// Approximate encoded bytes retained for a resuming subscriber.
pub const JOURNAL_BYTES: usize = 16 * 1024 * 1024;
/// Event replies retained for retries.
pub const EVENT_REPLIES: usize = 512;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Position {
    pub epoch: String,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResumeFrom {
    pub epoch: String,
    pub after: u64,
}

/// A fresh per-process epoch. Another process cannot produce the same value.
pub fn new_epoch() -> io::Result<String> {
    let mut bytes = [0_u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(crate::protocol::encode_hex(&bytes))
}

fn estimated_bytes(frame: &Response) -> usize {
    frame
        .messages
        .iter()
        .map(|message| message.payload_hex.len() + 160)
        .sum::<usize>()
        + 512
}

pub struct Journal {
    epoch: String,
    last: u64,
    frames: VecDeque<(Response, usize)>,
    bytes: usize,
    max_frames: usize,
    max_bytes: usize,
}

impl Journal {
    pub fn new(epoch: String) -> Self {
        Self::with_limits(epoch, JOURNAL_FRAMES, JOURNAL_BYTES)
    }

    pub fn with_limits(epoch: String, max_frames: usize, max_bytes: usize) -> Self {
        Self {
            epoch,
            last: 0,
            frames: VecDeque::new(),
            bytes: 0,
            max_frames: max_frames.max(1),
            max_bytes,
        }
    }

    /// The position a fresh subscriber starts after: it receives later frames.
    pub fn position(&self) -> Position {
        Position {
            epoch: self.epoch.clone(),
            sequence: self.last,
        }
    }

    /// Number the frame, retain a copy, and return the numbered frame.
    pub fn append(&mut self, mut frame: Response) -> Response {
        self.last += 1;
        frame.world_stream = Some(Position {
            epoch: self.epoch.clone(),
            sequence: self.last,
        });
        let size = estimated_bytes(&frame);
        self.frames.push_back((frame.clone(), size));
        self.bytes += size;
        while self.frames.len() > self.max_frames
            || (self.bytes > self.max_bytes && self.frames.len() > 1)
        {
            if let Some((_, size)) = self.frames.pop_front() {
                self.bytes -= size;
            }
        }
        frame
    }

    /// Every retained frame after `from`, or a refusal when that position is
    /// from another process or frames after it are no longer retained.
    pub fn replay(&self, from: &ResumeFrom) -> Result<Vec<Response>, &'static str> {
        if from.epoch != self.epoch || from.after > self.last {
            return Err(RESUME_UNAVAILABLE);
        }
        if from.after == self.last {
            return Ok(Vec::new());
        }
        let first = self
            .frames
            .front()
            .and_then(|(frame, _)| frame.world_stream.as_ref())
            .map(|position| position.sequence);
        if first.is_none_or(|first| first > from.after + 1) {
            return Err(RESUME_UNAVAILABLE);
        }
        Ok(self
            .frames
            .iter()
            .filter(|(frame, _)| {
                frame
                    .world_stream
                    .as_ref()
                    .is_some_and(|position| position.sequence > from.after)
            })
            .map(|(frame, _)| frame.clone())
            .collect())
    }
}

/// Recent event replies by request ID, oldest evicted first.
pub struct EventReplies {
    order: VecDeque<String>,
    replies: HashMap<String, Response>,
    capacity: usize,
}

impl Default for EventReplies {
    fn default() -> Self {
        Self::with_capacity(EVENT_REPLIES)
    }
}

impl EventReplies {
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            order: VecDeque::new(),
            replies: HashMap::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn get(&self, request_id: &str) -> Option<Response> {
        self.replies.get(request_id).cloned()
    }

    pub fn insert(&mut self, request_id: String, reply: Response) {
        if self.replies.insert(request_id.clone(), reply).is_none() {
            self.order.push_back(request_id);
        }
        while self.order.len() > self.capacity {
            if let Some(oldest) = self.order.pop_front() {
                self.replies.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(payload: &str) -> Response {
        let mut response = Response::success("synthetic-world".into());
        response.messages.push(crate::OutboundMessage {
            reservation_id: "synthetic-reservation-0001".into(),
            world_id: 1,
            player_id: 1,
            plugin_id: 1,
            payload_hex: payload.into(),
        });
        response
    }

    fn sequences(frames: &[Response]) -> Vec<u64> {
        frames
            .iter()
            .map(|frame| frame.world_stream.as_ref().unwrap().sequence)
            .collect()
    }

    fn from(epoch: &str, after: u64) -> ResumeFrom {
        ResumeFrom {
            epoch: epoch.into(),
            after,
        }
    }

    #[test]
    fn frames_are_numbered_and_resume_replays_exactly_the_missed_ones() {
        let mut journal = Journal::new("synthetic-epoch".into());
        assert_eq!(journal.position().sequence, 0);
        for payload in ["01", "02", "03", "04"] {
            journal.append(frame(payload));
        }
        assert_eq!(journal.position().sequence, 4);
        let missed = journal.replay(&from("synthetic-epoch", 2)).unwrap();
        assert_eq!(sequences(&missed), [3, 4]);
        assert_eq!(missed[0].messages[0].payload_hex, "03");
        assert!(
            journal
                .replay(&from("synthetic-epoch", 4))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            sequences(&journal.replay(&from("synthetic-epoch", 0)).unwrap()),
            [1, 2, 3, 4]
        );
    }

    #[test]
    fn another_process_or_an_unknown_position_is_refused() {
        let mut journal = Journal::new("synthetic-epoch".into());
        journal.append(frame("01"));
        assert_eq!(
            journal.replay(&from("synthetic-restarted", 1)).err(),
            Some(RESUME_UNAVAILABLE)
        );
        assert_eq!(
            journal.replay(&from("synthetic-epoch", 2)).err(),
            Some(RESUME_UNAVAILABLE)
        );
        assert_ne!(new_epoch().unwrap(), new_epoch().unwrap());
    }

    #[test]
    fn frames_beyond_retention_cannot_be_resumed() {
        let mut journal = Journal::with_limits("synthetic-epoch".into(), 3, usize::MAX);
        for _ in 0..5 {
            journal.append(frame("01"));
        }
        assert_eq!(
            journal.replay(&from("synthetic-epoch", 1)).err(),
            Some(RESUME_UNAVAILABLE)
        );
        assert_eq!(
            sequences(&journal.replay(&from("synthetic-epoch", 2)).unwrap()),
            [3, 4, 5]
        );
        let mut small = Journal::with_limits("synthetic-epoch".into(), 100, 1500);
        for _ in 0..5 {
            small.append(frame(&"ab".repeat(100)));
        }
        assert!(small.replay(&from("synthetic-epoch", 1)).is_err());
        assert_eq!(
            sequences(&small.replay(&from("synthetic-epoch", 4)).unwrap()),
            [5]
        );
    }

    #[test]
    fn event_replies_are_kept_by_request_id_and_bounded() {
        let mut replies = EventReplies::with_capacity(2);
        replies.insert("synthetic-a".into(), frame("0a"));
        replies.insert("synthetic-b".into(), frame("0b"));
        assert_eq!(
            replies.get("synthetic-a").unwrap().messages[0].payload_hex,
            "0a"
        );
        replies.insert("synthetic-c".into(), frame("0c"));
        assert!(replies.get("synthetic-a").is_none());
        assert!(replies.get("synthetic-b").is_some());
        assert!(replies.get("synthetic-c").is_some());
    }
}
