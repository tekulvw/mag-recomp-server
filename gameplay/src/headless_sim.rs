//! LOCAL DIAGNOSTIC ONLY. Drives one synthetic, never-spawning observer so a
//! bot match runs headless on virtual time. Not used by the server loop.
use super::*;

impl<W: Write> Engine<W> {
    /// Reserve and ready one observer that stays in its first-spawn wait, so
    /// quorum is met and bots fill the match without a human body to engage.
    #[doc(hidden)]
    pub fn headless_sim_join(
        &mut self,
        now: Duration,
        wall: u64,
        faction: u8,
    ) -> Result<(), String> {
        for request in [
            Request::Hello {
                v: BRIDGE_VERSION,
                request_id: "headless-sim-hello".into(),
            },
            Request::ConfigureMatch {
                v: BRIDGE_VERSION,
                request_id: "headless-sim-configure".into(),
                minimum_players: 1,
                minimum_players_per_team: [0, 0],
                maximum_players: 64,
                application_id: 20231,
            },
        ] {
            let response = self.handle(request, now, wall);
            if !response.ok {
                return Err(format!("bridge setup failed: {:?}", response.error));
            }
        }
        let response = self.handle(
            Request::Reserve {
                v: BRIDGE_VERSION,
                request_id: "headless-sim-reserve".into(),
                reservation_id: "headless-sim-reservation-0000".into(),
                world_id: 1,
                player_id: 0,
                account_id: 1,
                session_key: "aaaaaaaaaaaaaaaa".into(),
                access_key: "bbbbbbbbbbbbbbbb".into(),
                character: serde_json::from_value(json!({"name":"Headless Observer",
                    "faction":faction,"head":0,"voice":0,"universe_id":26,"character_id":1}))
                .map_err(|e| e.to_string())?,
            },
            now,
            wall,
        );
        if !response.ok {
            return Err(format!("reserve failed: {:?}", response.error));
        }
        let actor = self.active.as_mut().ok_or("no active actor")?;
        actor.ready_for_updates = true;
        actor.entered_game = true;
        actor.initial_objects = Some(Vec::new());
        actor.emplacement_creation = Some(Vec::new());
        actor.objective_delta = Some(Vec::new());
        actor.objective_assigned = true;
        actor.objective_needs_clear = false;
        actor.outbound_sequence = 1;
        // A live, on-foot human starts the first population; it is retired as
        // soon as bots exist (see keepalive) so bots never engage it.
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
        Ok(())
    }
    /// Keep the observer from idle or backlog eviction.
    #[doc(hidden)]
    pub fn headless_sim_keepalive(&mut self, now: Duration) {
        if let Some(actor) = self.active.as_mut() {
            actor.last_event = now;
            actor.replication_backlogged = false;
            if !self.bot_registry.bots.is_empty() && actor.life.is_alive() {
                actor.life = CharacterLife::AwaitingFirstSpawn;
            }
        }
    }
    /// Record every live bot's position and drop state (`sim_bot_snapshot`).
    #[doc(hidden)]
    pub fn headless_sim_snapshot(&mut self, now: Duration, wall: u64) -> Result<(), &'static str> {
        let bots: Vec<_> = self
            .bot_registry
            .bots
            .iter()
            .filter(|(_, b)| b.health > 0)
            .filter_map(|(&id, b)| {
                let r = b.target.runtime?;
                let p = r.physical_position();
                Some(json!({
                    "id": id,
                    "p": [(p[0] * 10.).round() / 10., (p[1] * 10.).round() / 10., (p[2] * 10.).round() / 10.],
                    "complete": matches!(r.phase, crate::development_target::BotPhase::Complete),
                    "waiting": b.drop_waiting,
                    "permit": b.drop_permit,
                    "holding": b.drop_holding_goal.map(|g| [g.x, g.y, g.z]),
                    "goal": b.task.goal.map(|g| [g.x, g.y, g.z]),
                }))
            })
            .collect();
        self.record(
            json!({"op":"sim_bot_snapshot","timestamp_ms":now.as_millis() as u64,"bots":bots}),
            wall,
        )
    }
}
