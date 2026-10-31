//! Bounded measured passage service and excess-over-geometry observations.

use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, time::Duration};
fn default_reversal_ms() -> u64 {
    serde_json::from_str::<serde_json::Value>(include_str!(
        "../data/synthetic-bot-objective-policy.json"
    ))
    .unwrap()["drop_delay"]["reversal_ms"]
        .as_u64()
        .unwrap()
}
fn default_reversal_history_limit() -> usize {
    serde_json::from_str::<serde_json::Value>(include_str!(
        "../data/synthetic-bot-objective-policy.json"
    ))
    .unwrap()["drop_delay"]["reversal_history_limit"]
        .as_u64()
        .unwrap() as usize
}
#[derive(Clone, Debug, PartialEq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Policy {
    pub alpha: f64,
    pub decay_ms: u64,
    pub maximum_s: f64,
    pub cache_limit: usize,
    pub reconsider_ms: u64,
    pub minimum_gain_s: f64,
    pub minimum_gain_ratio: f64,
    #[serde(default = "default_reversal_ms")]
    pub reversal_ms: u64,
    #[serde(default = "default_reversal_history_limit")]
    pub reversal_history_limit: usize,
}
impl Policy {
    pub fn valid(&self) -> bool {
        (0.05..=1.).contains(&self.alpha)
            && (1000..=120000).contains(&self.decay_ms)
            && (4. ..=120.).contains(&self.maximum_s)
            && (16..=2048).contains(&self.cache_limit)
            && (1000..=30000).contains(&self.reconsider_ms)
            && (1. ..=30.).contains(&self.minimum_gain_s)
            && (0.05..=0.75).contains(&self.minimum_gain_ratio)
            && (1000..=60000).contains(&self.reversal_ms)
            && (1..=32).contains(&self.reversal_history_limit)
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize)]
pub(super) struct Sample {
    pub service_s: f64,
    pub approach_excess_s: f64,
    pub traversal_excess_s: f64,
    pub departure_excess_s: f64,
    pub queue_wait_s: f64,
}
#[derive(Clone, Copy, Debug, Serialize)]
pub(super) struct Estimate {
    pub service_s: f64,
    pub own_excess_s: f64,
    pub samples: u32,
}
#[derive(Default)]
pub(super) struct Book {
    entries: BTreeMap<u32, (Duration, Estimate)>,
    pub approaches: BTreeMap<u32, Vec<[[f64; 3]; 2]>>,
    pub revision: u64,
}
impl Book {
    pub fn estimate(&self, id: u32, now: Duration, fallback: f64, p: &Policy) -> Estimate {
        let Some((at, old)) = self.entries.get(&id) else {
            return Estimate {
                service_s: fallback,
                own_excess_s: 0.,
                samples: 0,
            };
        };
        let age = now.saturating_sub(*at).as_secs_f64();
        let decay = (-age / (p.decay_ms as f64 / 1000.)).exp();
        Estimate {
            service_s: fallback + (old.service_s - fallback) * decay,
            own_excess_s: old.own_excess_s * decay,
            samples: old.samples,
        }
    }
    pub fn record(&mut self, id: u32, now: Duration, sample: Sample, fallback: f64, p: &Policy) {
        let excess =
            sample.approach_excess_s + sample.traversal_excess_s + sample.departure_excess_s;
        if !sample.service_s.is_finite()
            || !excess.is_finite()
            || sample.service_s < 0.
            || excess < 0.
        {
            return;
        }
        self.entries
            .retain(|_, (at, _)| now.saturating_sub(*at).as_millis() <= u128::from(p.decay_ms) * 8);
        let old = self.estimate(id, now, fallback, p);
        let estimate = Estimate {
            service_s: (old.service_s * (1. - p.alpha)
                + sample.service_s.clamp(0.25, p.maximum_s) * p.alpha)
                .clamp(0.25, p.maximum_s),
            own_excess_s: (old.own_excess_s * (1. - p.alpha) + excess.min(p.maximum_s) * p.alpha)
                .min(p.maximum_s),
            samples: old.samples.saturating_add(1),
        };
        if !self.entries.contains_key(&id)
            && self.entries.len() >= p.cache_limit
            && let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, value)| value.0)
                .map(|(&key, _)| key)
        {
            self.entries.remove(&oldest);
        }
        self.approaches
            .retain(|key, _| self.entries.contains_key(key));
        self.entries.insert(id, (now, estimate));
        self.revision = self.revision.wrapping_add(1);
    }
}
/// Every reserved actor contributes once. An active owner's remaining service
/// replaces that owner's full slot. Observed total queue wait is never added.
pub(super) fn forecast(
    slots: usize,
    service: f64,
    excess: f64,
    owner_remaining: Option<f64>,
) -> f64 {
    slots.saturating_sub(usize::from(owner_remaining.is_some())) as f64 * service
        + excess
        + owner_remaining.unwrap_or(0.)
}
pub(super) struct Passage {
    pub id: u32,
    pub protected: crate::actor_collision::Bounds,
    pub approaches: Vec<Vec<[[f64; 3]; 2]>>,
    pub slots: usize,
    pub owner_remaining: Option<f64>,
    pub estimate: Estimate,
    pub cooldown_s: f64,
}
#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct Cost {
    pub reserved_slots: usize,
    pub active_owner_slots: usize,
    pub active_owner_remaining_s: f64,
    pub queued_service_s: f64,
    pub own_excess_s: f64,
    pub cooldown_s: f64,
    pub total_s: f64,
    pub related: Vec<u32>,
}
pub(super) type Relations = BTreeMap<u32, Vec<u32>>;
fn intersects(a: crate::actor_collision::Bounds, b: crate::actor_collision::Bounds) -> bool {
    (0..3).all(|axis| a.min[axis] <= b.max[axis] && a.max[axis] >= b.min[axis])
}
pub(super) fn relations(passages: &[Passage], budget: &mut usize) -> Relations {
    let body = crate::bot_physics::Body {
        radius: crate::bot_physics::Body::default().radius * std::f64::consts::SQRT_2,
        ..Default::default()
    };
    let envelopes: Vec<Vec<_>> = passages
        .iter()
        .map(|p| {
            p.approaches
                .iter()
                .map(|lane| {
                    lane.iter()
                        .flatten()
                        .map(|&point| crate::actor_collision::Bounds::body(point, body))
                        .reduce(|a, b| a.union(b))
                })
                .collect()
        })
        .collect();
    let mut result: Relations = passages.iter().map(|p| (p.id, vec![p.id])).collect();
    for (i, p) in passages.iter().enumerate() {
        for (j, other) in passages.iter().enumerate().skip(i + 1) {
            let overlap = intersects(p.protected, other.protected)
                || p.approaches.iter().enumerate().any(|(a, lane)| {
                    other.approaches.iter().enumerate().any(|(b, other_lane)| {
                        *budget > 0
                            && envelopes[i][a]
                                .zip(envelopes[j][b])
                                .is_some_and(|(a, b)| intersects(a, b))
                            && crate::bot_traffic_flow::lanes_conflict(lane, other_lane, budget)
                    })
                });
            if overlap {
                result.get_mut(&p.id).unwrap().push(other.id);
                result.get_mut(&other.id).unwrap().push(p.id);
            }
        }
    }
    for related in result.values_mut() {
        related.sort_unstable();
    }
    result
}
pub(super) fn costs_with_relations(
    passages: &[Passage],
    relations: &Relations,
) -> BTreeMap<u32, Cost> {
    let indexed: BTreeMap<_, _> = passages.iter().map(|p| (p.id, p)).collect();
    passages
        .iter()
        .map(|p| {
            let mut cost = Cost {
                cooldown_s: p.cooldown_s,
                ..Default::default()
            };
            for &related in &relations[&p.id] {
                let other = indexed[&related];
                cost.related.push(related);
                cost.reserved_slots += other.slots;
                if let Some(remaining) = other.owner_remaining {
                    cost.active_owner_slots += 1;
                    cost.active_owner_remaining_s += remaining;
                }
                cost.queued_service_s += forecast(
                    other.slots,
                    other.estimate.service_s,
                    0.,
                    other.owner_remaining,
                );
                cost.own_excess_s = cost.own_excess_s.max(other.estimate.own_excess_s);
            }
            cost.total_s = cost.queued_service_s + cost.own_excess_s + cost.cooldown_s;
            (p.id, cost)
        })
        .collect()
}
#[cfg(test)]
fn costs(passages: &[Passage], budget: &mut usize) -> BTreeMap<u32, Cost> {
    costs_with_relations(passages, &relations(passages, budget))
}
pub(super) struct Reconsideration {
    pub key: Key,
    pub goal: crate::replication::CharacterPosition,
    pub origin: [f64; 3],
    pub baseline: crate::navmesh::Route,
}
pub(super) fn route_seconds(
    route: &crate::navmesh::Route,
    cursor: usize,
    actual: [f64; 3],
    costs: &BTreeMap<u32, f64>,
) -> f64 {
    let mut from = actual;
    let mut length = 0.;
    let mut drops = std::collections::BTreeSet::new();
    for i in cursor..route.waypoints.len() {
        let to = route.waypoints[i];
        length += (to[0] - from[0]).hypot(to[2] - from[2]);
        from = to;
        if let Some(id) = route.waypoint_drop_ids[i] {
            drops.insert(id);
        }
    }
    (length
        + drops
            .into_iter()
            .map(|id| costs.get(&id).copied().unwrap_or(0.))
            .sum::<f64>())
        / crate::retail_movement::WALK_FORWARD_MPS
}
pub(super) fn worthwhile(
    old: f64,
    new: f64,
    old_hp: Option<u16>,
    new_hp: Option<u16>,
    old_drop: u32,
    new_drop: Option<u32>,
    p: &Policy,
) -> bool {
    new_drop != Some(old_drop)
        && old.is_finite()
        && new.is_finite()
        && new_hp
            .zip(old_hp)
            .is_some_and(|(new, old)| new >= old && new > 0)
        && old - new >= p.minimum_gain_s.max(old * p.minimum_gain_ratio)
}
pub(super) fn next_reconsideration(now: Duration, actor: u16, p: &Policy) -> Duration {
    now + Duration::from_millis(p.reconsider_ms + (u64::from(actor) * 2654435761) % p.reconsider_ms)
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Key {
    pub generation: u32,
    pub objective: Option<u16>,
    pub role_slot: u8,
    pub passage: u32,
}
pub(super) struct Episode {
    pub key: Key,
    pub assigned: Duration,
    pub queued: Option<Duration>,
    pub granted: Option<Duration>,
    pub entered: Option<Duration>,
    pub landed: Option<Duration>,
    pub free_approach_s: f64,
    pub free_traversal_s: f64,
    pub approach: Vec<[[f64; 3]; 2]>,
}
impl Episode {
    pub fn new(key: Key, now: Duration) -> Self {
        Self {
            key,
            assigned: now,
            queued: None,
            granted: None,
            entered: None,
            landed: None,
            free_approach_s: 0.,
            free_traversal_s: 0.,
            approach: Vec::new(),
        }
    }
    pub fn grant(&mut self, now: Duration, approach_s: f64, traversal_s: f64) {
        self.granted = Some(now);
        self.entered = None;
        self.landed = None;
        self.free_approach_s = approach_s;
        self.free_traversal_s = traversal_s;
    }
    pub fn sample(&self, key: Key, now: Duration, free_departure_s: f64) -> Option<Sample> {
        if key != self.key {
            return None;
        }
        let granted = self.granted?;
        let entered = self.entered?;
        let landed = self.landed?;
        if self.assigned > granted || granted > entered || entered > landed || landed > now {
            return None;
        }
        Some(Sample {
            service_s: now.saturating_sub(granted).as_secs_f64(),
            approach_excess_s: (entered.saturating_sub(granted).as_secs_f64()
                - self.free_approach_s)
                .max(0.),
            traversal_excess_s: (landed.saturating_sub(entered).as_secs_f64()
                - self.free_traversal_s)
                .max(0.),
            departure_excess_s: (now.saturating_sub(landed).as_secs_f64() - free_departure_s)
                .max(0.),
            queue_wait_s: self.queued.map_or(0., |queued| {
                granted
                    .saturating_sub(queued.max(self.assigned))
                    .as_secs_f64()
            }),
        })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn policy() -> Policy {
        Policy {
            alpha: 0.5,
            decay_ms: 10000,
            maximum_s: 30.,
            cache_limit: 2,
            reconsider_ms: 3000,
            minimum_gain_s: 3.,
            minimum_gain_ratio: 0.2,
            reversal_ms: 15000,
            reversal_history_limit: 8,
        }
    }
    #[test]
    fn reconsideration_requires_meaningful_gain_and_preserves_health_without_churn() {
        let p = policy();
        assert!(worthwhile(30., 20., Some(400), Some(400), 9, Some(10), &p));
        assert!(!worthwhile(30., 28., Some(400), Some(400), 9, Some(10), &p));
        assert!(!worthwhile(
            100.,
            95.,
            Some(400),
            Some(400),
            9,
            Some(10),
            &p
        ));
        assert!(!worthwhile(30., 20., Some(400), Some(399), 9, Some(10), &p));
        assert!(!worthwhile(30., 20., Some(400), Some(400), 9, Some(9), &p));
        let a = next_reconsideration(Duration::ZERO, 3, &p);
        let b = next_reconsideration(Duration::ZERO, 4, &p);
        assert_ne!(a, b);
        assert!(a >= Duration::from_millis(p.reconsider_ms));
    }
    #[test]
    fn intersecting_approaches_share_unique_direct_demand_without_transitive_double_counting() {
        let make = |id, position, lane, slots, excess| Passage {
            id,
            protected: crate::actor_collision::Bounds::body(position, Default::default()),
            approaches: vec![lane],
            slots,
            owner_remaining: None,
            estimate: Estimate {
                service_s: 4.,
                own_excess_s: excess,
                samples: 1,
            },
            cooldown_s: 0.,
        };
        let a = make(9, [20., 0., 0.], vec![[[-4., 0., 0.], [4., 0., 0.]]], 1, 2.);
        let b = make(
            10,
            [40., 0., 0.],
            vec![[[0., 0., -4.], [0., 0., 8.]]],
            1,
            3.,
        );
        let c = make(
            11,
            [60., 0., 0.],
            vec![[[-4., 0., 8.], [4., 0., 8.]]],
            1,
            5.,
        );
        let quotes = costs(&[a, b, c], &mut 8192);
        assert_eq!(quotes[&9].related, vec![9, 10]);
        assert_eq!(quotes[&9].reserved_slots, 2);
        assert_eq!(
            quotes[&9].total_s, 11.,
            "two unique4s slots + maximum shared3s excess, no transitive queue"
        );
    }
    #[test]
    fn measured_cost_replaces_owner_slot_without_double_charging_queue_wait() {
        let mut book = Book::default();
        let p = policy();
        book.record(
            9,
            Duration::ZERO,
            Sample {
                service_s: 8.,
                approach_excess_s: 2.,
                queue_wait_s: 100.,
                ..Default::default()
            },
            4.,
            &p,
        );
        let e = book.estimate(9, Duration::ZERO, 4., &p);
        assert_eq!(e.service_s, 6.);
        assert_eq!(e.own_excess_s, 1.);
        assert_eq!(
            forecast(3, e.service_s, e.own_excess_s, Some(2.)),
            15.,
            "two full slots + one remaining owner + own excess; no100s observed queue wait"
        );
        assert_eq!(
            forecast(0, e.service_s, e.own_excess_s, None),
            1.,
            "the querying actor is excluded from reserved slots"
        );
    }
    #[test]
    fn episode_uses_actual_stages_and_discards_obsolete_life_or_intent() {
        let key = Key {
            generation: 4,
            objective: Some(2),
            role_slot: 1,
            passage: 9,
        };
        let mut episode = Episode::new(key, Duration::from_secs(10));
        episode.queued = Some(Duration::from_secs(11));
        episode.grant(Duration::from_secs(20), 1., 2.);
        assert!(episode.sample(key, Duration::from_secs(21), 0.5).is_none());
        episode.entered = Some(Duration::from_secs(22));
        episode.landed = Some(Duration::from_secs(25));
        let sample = episode.sample(key, Duration::from_secs(26), 0.5).unwrap();
        assert_eq!(sample.service_s, 6.);
        assert_eq!(sample.queue_wait_s, 9.);
        assert_eq!(sample.approach_excess_s, 1.);
        assert_eq!(sample.traversal_excess_s, 1.);
        assert_eq!(sample.departure_excess_s, 0.5);
        assert!(
            episode
                .sample(
                    Key {
                        generation: 5,
                        ..key
                    },
                    Duration::from_secs(26),
                    0.5
                )
                .is_none()
        );
        assert!(
            episode
                .sample(
                    Key {
                        objective: Some(3),
                        ..key
                    },
                    Duration::from_secs(26),
                    0.5
                )
                .is_none()
        );
        assert!(
            episode
                .sample(Key { passage: 10, ..key }, Duration::from_secs(26), 0.5)
                .is_none()
        );
    }
    #[test]
    fn stale_measurements_decay_to_fallback_and_storage_is_bounded() {
        let mut book = Book::default();
        let p = policy();
        book.record(
            9,
            Duration::ZERO,
            Sample {
                service_s: 8.,
                approach_excess_s: 2.,
                ..Default::default()
            },
            4.,
            &p,
        );
        let initial = book.estimate(9, Duration::ZERO, 4., &p);
        let stale = book.estimate(9, Duration::from_secs(10), 4., &p);
        assert!(stale.service_s < initial.service_s && stale.service_s > 4.);
        assert!(stale.own_excess_s < initial.own_excess_s);
        book.record(
            10,
            Duration::from_secs(1),
            Sample {
                service_s: 8.,
                ..Default::default()
            },
            4.,
            &p,
        );
        book.record(
            11,
            Duration::from_secs(2),
            Sample {
                service_s: 8.,
                ..Default::default()
            },
            4.,
            &p,
        );
        assert_eq!(book.entries.len(), 2);
        assert_eq!(book.estimate(9, Duration::from_secs(2), 4., &p).samples, 0);
    }
}
