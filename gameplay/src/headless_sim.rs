//! LOCAL DIAGNOSTIC ONLY. Runs a bot match headless on virtual time with
//! synthetic observers that never spawn, and measures each world tick and each
//! observer's outbound traffic. Not used by the server loop.
use super::*;
use crate::tick_timing::Phase;
use crate::world_tick::WORLD_TICK_INTERVAL;
use std::collections::BTreeMap;

/// Highest simulated client event rate per observer.
pub const MAX_SIM_EVENT_HZ: u16 = 60;

/// Options read from `MAG_SIM_*` (see `docs/headless-simulation.md`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeadlessSimOptions {
    /// Simulated seconds.
    pub seconds: u64,
    /// Registry profile to select, as Horizon would; `None` configures the
    /// loaded spawn catalog's match for every human seat it admits.
    pub profile: Option<String>,
    /// Ready observers that drain their queues like connected clients.
    pub observers: u16,
    /// Client events per second per observer. Each event's bridge reply
    /// drains the shared queues once more, as a client event does live. Zero
    /// leaves world frames as the only drain.
    pub event_hz: u16,
    /// Pins every observer to one faction. Otherwise observers alternate
    /// between the attacking and defending factions, attacking first.
    pub faction: Option<u8>,
    /// Record `sim_bot_snapshot` every 500 ms.
    pub snapshots: bool,
    /// Leading simulated seconds left out of the summary statistics.
    pub warmup_seconds: u64,
    /// Acquisition Structural objectives completed by a scripted request at
    /// a simulated second, as (object index, second). Diagnostic only: it
    /// stands in for attackers destroying the structure.
    pub structural_completions: Vec<(u16, u64)>,
}

impl HeadlessSimOptions {
    pub fn new(seconds: u64) -> Self {
        Self {
            seconds,
            profile: None,
            observers: 1,
            event_hz: 0,
            faction: None,
            snapshots: false,
            warmup_seconds: 0,
            structural_completions: Vec::new(),
        }
    }

    /// Read the `MAG_SIM_*` settings through `lookup`. A set but invalid value
    /// fails instead of falling back to a default.
    pub fn from_lookup(
        seconds: u64,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<Self, String> {
        fn parse<T: std::str::FromStr>(
            lookup: &impl Fn(&str) -> Option<String>,
            name: &str,
        ) -> Result<Option<T>, String> {
            lookup(name)
                .map(|value| {
                    value
                        .trim()
                        .parse()
                        .map_err(|_| format!("invalid {name}: {value:?}"))
                })
                .transpose()
        }
        let mut options = Self::new(seconds);
        if let Some(profile) = lookup("MAG_SIM_PROFILE") {
            if profile.trim().is_empty() {
                return Err("invalid MAG_SIM_PROFILE: empty".into());
            }
            options.profile = Some(profile.trim().to_owned());
        }
        if let Some(observers) = parse::<u16>(&lookup, "MAG_SIM_OBSERVERS")? {
            if observers == 0 {
                return Err("invalid MAG_SIM_OBSERVERS: at least one observer is required".into());
            }
            options.observers = observers;
        }
        if let Some(hz) = parse::<u16>(&lookup, "MAG_SIM_EVENT_HZ")? {
            if hz > MAX_SIM_EVENT_HZ {
                return Err(format!(
                    "invalid MAG_SIM_EVENT_HZ: {hz} exceeds {MAX_SIM_EVENT_HZ}"
                ));
            }
            options.event_hz = hz;
        }
        options.faction = parse(&lookup, "MAG_SIM_FACTION")?;
        options.snapshots = match lookup("MAG_SIM_SNAPSHOTS").as_deref() {
            None | Some("0") => false,
            Some("1") => true,
            Some(other) => return Err(format!("invalid MAG_SIM_SNAPSHOTS: {other:?}")),
        };
        if let Some(warmup) = parse::<u64>(&lookup, "MAG_SIM_WARMUP_S")? {
            if warmup >= seconds {
                return Err(format!(
                    "invalid MAG_SIM_WARMUP_S: {warmup} leaves nothing of {seconds} simulated seconds"
                ));
            }
            options.warmup_seconds = warmup;
        }
        if let Some(list) = lookup("MAG_SIM_COMPLETE_STRUCTURALS") {
            let invalid = || format!("invalid MAG_SIM_COMPLETE_STRUCTURALS: {list:?}");
            for item in list.split(',') {
                let (index, second) = item.trim().split_once('@').ok_or_else(invalid)?;
                let index: u16 = index.trim().parse().map_err(|_| invalid())?;
                let second: u64 = second.trim().parse().map_err(|_| invalid())?;
                if second == 0 || second > seconds {
                    return Err(invalid());
                }
                options.structural_completions.push((index, second));
            }
        }
        Ok(options)
    }
}

/// Mean and tail of one per-tick series.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Distribution {
    pub mean: f64,
    pub p50: f64,
    pub p99: f64,
    pub p999: f64,
    pub max: f64,
}

impl Distribution {
    pub fn of(values: &[f64]) -> Self {
        if values.is_empty() {
            return Self::default();
        }
        let mut sorted = values.to_vec();
        sorted.sort_by(f64::total_cmp);
        let at = |q: f64| sorted[((q * sorted.len() as f64) as usize).min(sorted.len() - 1)];
        Self {
            mean: sorted.iter().sum::<f64>() / sorted.len() as f64,
            p50: at(0.5),
            p99: at(0.99),
            p999: at(0.999),
            max: sorted[sorted.len() - 1],
        }
    }
}

/// One observer's outbound traffic over the measured window.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct ObserverTraffic {
    pub reservation_id: String,
    pub player_id: u16,
    pub faction: u8,
    /// Measured seconds this observer was connected.
    pub measured_s: f64,
    /// Plugin messages released to this observer, and their payload bytes
    /// (no RT, DME or TCP framing).
    pub sent_packets_per_s: f64,
    pub sent_bytes_per_s: f64,
    /// What the world produced for it: sent plus queue growth.
    pub offered_packets_per_s: f64,
    pub offered_bytes_per_s: f64,
    /// Messages waiting in its reliable FIFO and notification queue after each tick.
    pub queue_depth: Distribution,
    /// Simulated time it was evicted, for example as `replication_backlogged`.
    pub evicted_at_ms: Option<u64>,
}

/// The end-of-run summary printed as `headless_sim_summary`.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct HeadlessSimReport {
    pub profile: Option<String>,
    pub observers: u16,
    pub simulated_s: u64,
    pub warmup_s: u64,
    pub measured_ticks: usize,
    pub event_hz: u16,
    pub tick_errors: u64,
    pub first_tick_errors: Vec<String>,
    pub bots_max: usize,
    pub bots_alive_mean: f64,
    /// Wall time of `tick_world`, in milliseconds.
    pub tick_ms: Distribution,
    pub ticks_over_25ms: usize,
    pub ticks_over_50ms: usize,
    /// Wall time per phase, in milliseconds (`bots.*` are included in `bots`).
    pub phase_ms: BTreeMap<&'static str, Distribution>,
    /// Wall time per tick interval of the simulated event-reply drains, in
    /// milliseconds; not part of `tick_ms`.
    pub event_drain_ms: Distribution,
    /// Simulated event-reply drains per second.
    pub event_drains_per_s: f64,
    pub recipients: Vec<ObserverTraffic>,
    /// Sent packets and bytes per second, summed over all observers.
    pub total_sent_packets_per_s: f64,
    pub total_sent_bytes_per_s: f64,
    pub evictions: usize,
}

/// Per-observer accounting while the run is in progress.
struct Observer {
    id: String,
    player: u16,
    faction: u8,
    sent_packets: u64,
    sent_bytes: u64,
    offered_packets: i64,
    offered_bytes: i64,
    depth: Vec<f64>,
    /// Queued messages and bytes after the previous tick.
    previous: (usize, usize),
    measured_ticks: usize,
    evicted_at_ms: Option<u64>,
}

impl<W: Write> Engine<W> {
    /// Issue the bridge setup Horizon would send: `hello`, then either the
    /// profile selection (which configures the match) or `configure_match`.
    fn headless_sim_configure(
        &mut self,
        profile: Option<&str>,
        now: Duration,
        wall: u64,
    ) -> Result<(), String> {
        let setup = match profile {
            Some(profile_id) => Request::SelectMatchProfile {
                v: BRIDGE_VERSION,
                request_id: "headless-sim-select".into(),
                formation_id: "headless-sim-formation".into(),
                profile_id: profile_id.into(),
            },
            None => Request::ConfigureMatch {
                v: BRIDGE_VERSION,
                request_id: "headless-sim-configure".into(),
                minimum_players: 1,
                minimum_players_per_team: [0, 0],
                // Every human seat the loaded world admits: 64 in a legacy
                // 64-character world, the human cap in a larger one.
                maximum_players: self
                    .human_admission_capacity_for(20231)
                    .min(world_capacity::MAX_ADMITTED_HUMANS),
                application_id: 20231,
            },
        };
        let hello = Request::Hello {
            v: BRIDGE_VERSION,
            request_id: "headless-sim-hello".into(),
        };
        for request in [hello, setup] {
            let response = self.handle(request, now, wall);
            if !response.ok {
                return Err(format!("bridge setup failed: {:?}", response.error));
            }
        }
        Ok(())
    }

    fn headless_sim_actor(&mut self, id: &str) -> Option<&mut Active> {
        self.active
            .iter_mut()
            .chain(self.peers.values_mut())
            .find(|actor| actor.reservation.id == id)
    }

    /// Configure the match and reserve the observers. Each is ready for
    /// updates with its one-time creation bootstrap marked delivered (a real
    /// client receives it in replies to its own events, which the sim does not
    /// send), so it drains world frames like a connected client. The first
    /// observer briefly has a body far outside the map, which starts the bot
    /// population; every observer then stays in its first-spawn wait.
    /// Returns each observer's reservation id, player id and faction.
    pub(crate) fn headless_sim_join(
        &mut self,
        options: &HeadlessSimOptions,
        now: Duration,
        wall: u64,
    ) -> Result<Vec<(String, u16, u8)>, String> {
        self.headless_sim_configure(options.profile.as_deref(), now, wall)?;
        if usize::from(options.observers) > self.maximum_players {
            return Err(format!(
                "{} observers exceed the match's {} players",
                options.observers, self.maximum_players
            ));
        }
        let policy = self.match_profile();
        let ground_batches = self
            .ground_vehicles
            .as_ref()
            .map_or(0, |catalog| catalog.batch_count());
        let mut joined = Vec::new();
        for index in 0..options.observers {
            let faction =
                options
                    .faction
                    .unwrap_or(if policy.auto_assign_teams && index.is_multiple_of(2) {
                        policy.attacking_faction
                    } else {
                        policy.defending_faction
                    });
            let id = format!("headless-sim-reservation-{index:04}");
            let response = self.handle(
                Request::Reserve {
                    v: BRIDGE_VERSION,
                    request_id: format!("headless-sim-reserve-{index}"),
                    reservation_id: id.clone(),
                    world_id: 1,
                    player_id: index,
                    account_id: u32::from(index) + 1,
                    session_key: "aaaaaaaaaaaaaaaa".into(),
                    access_key: "bbbbbbbbbbbbbbbb".into(),
                    character: serde_json::from_value(json!({
                        "name": format!("Headless Observer {index}"),
                        "faction": faction, "head": 0, "voice": 0,
                        "universe_id": 26, "character_id": u32::from(index) + 1}))
                    .map_err(|e| e.to_string())?,
                },
                now,
                wall,
            );
            if !response.ok {
                return Err(format!(
                    "observer {index} (faction {faction}) reserve failed: {:?}",
                    response.error
                ));
            }
            let actor = self
                .headless_sim_actor(&id)
                .ok_or("reserved observer is missing")?;
            actor.ready_for_updates = true;
            actor.entered_game = true;
            actor.initial_objects = Some(Vec::new());
            actor.emplacement_creation = Some(Vec::new());
            actor.objective_delta = Some(Vec::new());
            actor.objective_assigned = true;
            actor.objective_needs_clear = false;
            actor.ground_vehicle_batch = ground_batches;
            actor.helo_creation_sent = true;
            actor.outbound_sequence = 1;
            if index == 0 {
                actor.life = CharacterLife::Alive {
                    generation: 1,
                    health_quarter_points: 400,
                };
                actor.tracking.server_position(
                    replication::CharacterPosition {
                        x: 1.,
                        y: 500.,
                        z: 1.,
                    },
                    player_tracking::PositionOrigin::ServerSpawn,
                    0,
                );
            }
            joined.push((id, index, faction));
        }
        Ok(joined)
    }

    /// Keep observers from idle eviction, and return the starting body to its
    /// first-spawn wait once bots exist so they never engage it. Backlog
    /// eviction stays in force: it is part of what the sim measures.
    fn headless_sim_keepalive(&mut self, now: Duration) {
        let bots = !self.bot_registry.bots.is_empty();
        for actor in self.active.iter_mut().chain(self.peers.values_mut()) {
            actor.last_event = now;
            if bots && actor.life.is_alive() {
                actor.life = CharacterLife::AwaitingFirstSpawn;
            }
        }
    }

    /// Record every live bot's position and drop state (`sim_bot_snapshot`).
    fn headless_sim_snapshot(&mut self, now: Duration, wall: u64) -> Result<(), &'static str> {
        let structures = self
            .collision_world
            .as_ref()
            .and_then(|w| w.bot_structures_arc().cloned());
        let bots: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter(|(_, b)| b.health > 0)
            .filter_map(|(&id, b)| {
                let r = b.target.runtime?;
                let p = r.physical_position();
                // A body standing inside a placed structure's solid, and one
                // leaving the enclosed room it started in (which may cross
                // that room's walls).
                let leaving_room = structures.as_ref().map(|s| s.enclosed(p));
                let in_structure = structures.as_ref().map(|s| {
                    !s.enclosed(p)
                        && s.body_penetrates(p, crate::bot_physics::Body::default())
                            .unwrap_or(true)
                });
                Some(json!({
                    "id": id,
                    "p": [(p[0] * 10.).round() / 10., (p[1] * 10.).round() / 10., (p[2] * 10.).round() / 10.],
                    "complete": matches!(r.phase, crate::development_target::BotPhase::Complete),
                    "waiting": b.drop_waiting,
                    "permit": b.drop_permit,
                    "holding": b.drop_holding_goal.map(|g| [g.x, g.y, g.z]),
                    "goal": b.task.goal.map(|g| [g.x, g.y, g.z]),
                    "in_structure": in_structure,
                    "leaving_room": leaving_room,
                }))
            })
            .collect();
        self.record(
            json!({"op":"sim_bot_snapshot","timestamp_ms":now.as_millis() as u64,"bots":bots}),
            wall,
        )
    }

    /// The shared drain that ends every multiplayer event reply, without the
    /// event itself.
    fn headless_sim_event_reply(
        &mut self,
        wall: u64,
    ) -> Result<Vec<OutboundMessage>, &'static str> {
        self.drain_shared(Vec::new(), wall)
    }

    /// One observer's queued messages and their bytes, or `None` once evicted.
    fn headless_sim_queue(&self, id: &str) -> Option<(usize, usize)> {
        let actor = self
            .active
            .iter()
            .chain(self.peers.values())
            .find(|actor| actor.reservation.id == id)?;
        let frames = actor.pending_shared.iter().map(Vec::len);
        let notifications = actor.pending_notifications.iter().map(|p| match p {
            match_flow::Publication::Application(wire)
            | match_flow::Publication::Replication(wire) => wire.len(),
        });
        Some((
            actor.pending_shared.len() + actor.pending_notifications.len(),
            frames.chain(notifications).sum(),
        ))
    }

    /// Run the whole match on virtual 50 ms ticks and summarise it. With
    /// `ticks`, also write one JSON line per tick: wall time, phase times,
    /// bots and, per observer, `[player_id, sent_packets, sent_bytes, queue_depth]`.
    pub fn run_headless_sim(
        &mut self,
        options: &HeadlessSimOptions,
        wall0: u64,
        ticks: Option<&mut dyn Write>,
    ) -> Result<HeadlessSimReport, String> {
        self.enable_world_ticks();
        self.enable_tick_timing();
        self.bot_cover.blocking_builds = true;
        let joined = self.headless_sim_join(options, Duration::ZERO, wall0)?;
        self.headless_sim_measure(options, joined, wall0, ticks)
    }

    /// The tick loop of `run_headless_sim`, for observers already joined.
    pub(crate) fn headless_sim_measure(
        &mut self,
        options: &HeadlessSimOptions,
        joined: Vec<(String, u16, u8)>,
        wall0: u64,
        mut ticks: Option<&mut dyn Write>,
    ) -> Result<HeadlessSimReport, String> {
        let mut observers: Vec<Observer> = joined
            .into_iter()
            .map(|(id, player, faction)| Observer {
                previous: self.headless_sim_queue(&id).unwrap_or_default(),
                id,
                player,
                faction,
                sent_packets: 0,
                sent_bytes: 0,
                offered_packets: 0,
                offered_bytes: 0,
                depth: Vec::new(),
                measured_ticks: 0,
                evicted_at_ms: None,
            })
            .collect();
        let warmup = Duration::from_secs(options.warmup_seconds);
        let mut report = HeadlessSimReport {
            profile: options.profile.clone(),
            observers: options.observers,
            simulated_s: options.seconds,
            warmup_s: options.warmup_seconds,
            event_hz: options.event_hz,
            ..Default::default()
        };
        // Event credit in thousandths of an event, so the schedule is exact.
        let mut event_credit = 0u64;
        let mut event_drain_ms = Vec::new();
        let mut event_drains = 0u64;
        let mut tick_ms = Vec::new();
        let mut phase_ms: Vec<Vec<f64>> = vec![Vec::new(); Phase::ALL.len()];
        let mut alive_sum = 0usize;
        let mut now = WORLD_TICK_INTERVAL;
        while now <= Duration::from_secs(options.seconds) {
            let wall = wall0 + now.as_secs();
            self.headless_sim_keepalive(now);
            for &(index, second) in &options.structural_completions {
                if now == Duration::from_secs(second) {
                    let result = self.queue_acquisition_structural_state(
                        index,
                        replication::ObjectiveState::Completed,
                    );
                    self.record(
                        json!({"op":"sim_structural_completion_requested",
                            "timestamp_ms":now.as_millis() as u64,"object_index":index,
                            "error":result.err()}),
                        wall,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            if options.snapshots && now.as_millis().is_multiple_of(500) {
                self.headless_sim_snapshot(now, wall)
                    .map_err(|e| e.to_string())?;
            }
            let started = Instant::now();
            let response = self.tick_world(now, wall, "headless-sim".into());
            let elapsed = started.elapsed();
            let timing = self.take_tick_timing().unwrap_or_default();
            let mut errors = Vec::new();
            if !response.ok {
                errors.push(format!("{} ms: {:?}", now.as_millis(), response.error));
            }
            // Client events arriving before the next tick, spread evenly.
            let connected = observers
                .iter()
                .filter(|o| o.evicted_at_ms.is_none())
                .count() as u64;
            event_credit +=
                connected * u64::from(options.event_hz) * WORLD_TICK_INTERVAL.as_millis() as u64;
            let mut replies = Vec::new();
            let started = Instant::now();
            let mut passes = 0u64;
            while event_credit >= 1000 {
                event_credit -= 1000;
                passes += 1;
                match self.headless_sim_event_reply(wall) {
                    Ok(messages) => replies.extend(messages),
                    Err(code) => errors.push(format!("{} ms event reply: {code}", now.as_millis())),
                }
            }
            let event_elapsed = started.elapsed();
            for error in errors {
                report.tick_errors += 1;
                if report.first_tick_errors.len() < 5 {
                    report.first_tick_errors.push(error);
                }
            }
            let mut sent: BTreeMap<&str, (u64, u64)> = BTreeMap::new();
            for message in response.messages.iter().chain(&replies) {
                let entry = sent.entry(message.reservation_id.as_str()).or_default();
                entry.0 += 1;
                entry.1 += (message.payload_hex.len() / 2) as u64;
            }
            let measured = now >= warmup;
            let mut per_observer = Vec::new();
            for observer in &mut observers {
                if observer.evicted_at_ms.is_some() {
                    continue;
                }
                let Some(queue) = self.headless_sim_queue(&observer.id) else {
                    observer.evicted_at_ms = Some(now.as_millis() as u64);
                    continue;
                };
                let (packets, bytes) = sent.get(observer.id.as_str()).copied().unwrap_or_default();
                if measured {
                    observer.sent_packets += packets;
                    observer.sent_bytes += bytes;
                    observer.offered_packets +=
                        packets as i64 + queue.0 as i64 - observer.previous.0 as i64;
                    observer.offered_bytes +=
                        bytes as i64 + queue.1 as i64 - observer.previous.1 as i64;
                    observer.depth.push(queue.0 as f64);
                    observer.measured_ticks += 1;
                }
                observer.previous = queue;
                per_observer.push(json!([observer.player, packets, bytes, queue.0]));
            }
            let bots = self.bot_registry.bots.len();
            let alive = self
                .bot_registry
                .bots
                .values()
                .filter(|bot| bot.health > 0)
                .count();
            if measured {
                tick_ms.push(elapsed.as_secs_f64() * 1e3);
                event_drain_ms.push(event_elapsed.as_secs_f64() * 1e3);
                event_drains += passes;
                for (slot, (_, time)) in timing.phases().enumerate() {
                    phase_ms[slot].push(time.as_secs_f64() * 1e3);
                }
                report.bots_max = report.bots_max.max(bots);
                alive_sum += alive;
            }
            if let Some(out) = ticks.as_mut() {
                let phases: serde_json::Map<String, Value> = timing
                    .phases()
                    .map(|(phase, time)| (phase.name().to_owned(), json!(time.as_nanos() as u64)))
                    .collect();
                writeln!(
                    out,
                    "{}",
                    json!({"t_ms": now.as_millis() as u64, "tick_ns": elapsed.as_nanos() as u64,
                        "phases_ns": phases, "bots": bots, "bots_alive": alive,
                        "event_drains": passes,
                        "event_drain_ns": event_elapsed.as_nanos() as u64,
                        "sent_packets": response.messages.len() + replies.len(),
                        "sent_bytes": sent.values().map(|s| s.1).sum::<u64>(),
                        "observers": per_observer, "ok": response.ok})
                )
                .map_err(|e| e.to_string())?;
            }
            now += WORLD_TICK_INTERVAL;
        }
        report.measured_ticks = tick_ms.len();
        report.bots_alive_mean = alive_sum as f64 / tick_ms.len().max(1) as f64;
        report.ticks_over_25ms = tick_ms.iter().filter(|&&ms| ms > 25.).count();
        report.ticks_over_50ms = tick_ms.iter().filter(|&&ms| ms > 50.).count();
        report.tick_ms = Distribution::of(&tick_ms);
        report.event_drain_ms = Distribution::of(&event_drain_ms);
        report.event_drains_per_s =
            event_drains as f64 / (tick_ms.len().max(1) as f64 * WORLD_TICK_INTERVAL.as_secs_f64());
        report.phase_ms = Phase::ALL
            .iter()
            .zip(&phase_ms)
            .map(|(phase, values)| (phase.name(), Distribution::of(values)))
            .collect();
        let tick_s = WORLD_TICK_INTERVAL.as_secs_f64();
        for observer in observers {
            let seconds = observer.measured_ticks as f64 * tick_s;
            let rate = |value: f64| if seconds > 0. { value / seconds } else { 0. };
            report.total_sent_packets_per_s += rate(observer.sent_packets as f64);
            report.total_sent_bytes_per_s += rate(observer.sent_bytes as f64);
            report.evictions += usize::from(observer.evicted_at_ms.is_some());
            report.recipients.push(ObserverTraffic {
                measured_s: seconds,
                sent_packets_per_s: rate(observer.sent_packets as f64),
                sent_bytes_per_s: rate(observer.sent_bytes as f64),
                offered_packets_per_s: rate(observer.offered_packets as f64),
                offered_bytes_per_s: rate(observer.offered_bytes as f64),
                queue_depth: Distribution::of(&observer.depth),
                evicted_at_ms: observer.evicted_at_ms,
                reservation_id: observer.id,
                player_id: observer.player,
                faction: observer.faction,
            });
        }
        Ok(report)
    }
}
