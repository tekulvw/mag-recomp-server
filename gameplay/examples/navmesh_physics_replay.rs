//! Offline, fixed-step physical feedback replay of a source-bound navmesh route.
//! Does not start an Engine, listener, service, game, or autonomous scheduler.
use mag_gameplay::{
    bot_physics::{Body, Motion, Point, WorldTerrain},
    collision::{CollisionLimits, CollisionWorld},
};
use serde::Deserialize;
use serde_json::json;
use std::{env, fs, path::PathBuf, process::Command, time::Instant};

const TICK_SECONDS: f64 = 0.05;
const ARRIVAL_XZ: f64 = 1.0 / 32.0;

#[derive(Clone, Debug, Deserialize)]
struct Profile {
    radius: f64,
    height: f64,
    max_step: f64,
    max_slope_degrees: f64,
    skin: f64,
    #[serde(default = "default_ground_snap")]
    ground_snap: f64,
    bake_radius: f64,
    cell_size: f64,
    cell_height: f64,
}
fn default_ground_snap() -> f64 {
    Body::default().ground_snap
}
impl Profile {
    fn body(&self) -> Result<Body, String> {
        let body = Body {
            radius: self.radius,
            height: self.height,
            max_step: self.max_step,
            max_slope_degrees: self.max_slope_degrees,
            ground_snap: self.ground_snap,
            skin: self.skin,
        };
        body.validate().map_err(|e| e.code().to_owned())?;
        if !self.bake_radius.is_finite()
            || self.bake_radius + 1e-8 < self.radius * 2.0_f64.sqrt() + self.skin
            || !self.cell_size.is_finite()
            || !(0.01..=1.0).contains(&self.cell_size)
            || !self.cell_height.is_finite()
            || !(0.005..=0.5).contains(&self.cell_height)
        {
            return Err("navmesh profile does not conservatively cover the physical box".into());
        }
        Ok(body)
    }
}

#[derive(Debug, Deserialize)]
struct Route {
    format: String,
    status: String,
    world_sha256: String,
    profile: Profile,
    start: Point,
    requested_goal: Point,
    projected_start: Point,
    projected_goal: Point,
    waypoints: Vec<Point>,
    corridor_refs: Vec<u64>,
    length_m: f64,
    query_us: u64,
}
fn valid_point(point: Point) -> bool {
    point
        .iter()
        .all(|v| v.is_finite() && v.abs() <= 10_000_000.0)
}
fn distance(a: Point, b: Point) -> f64 {
    (0..3).map(|i| (a[i] - b[i]).powi(2)).sum::<f64>().sqrt()
}
fn xz_distance(a: Point, b: Point) -> f64 {
    (a[0] - b[0]).hypot(a[2] - b[2])
}
impl Route {
    fn validate(&self, world_sha256: &str, goal_radius: f64) -> Result<Body, String> {
        if self.format != "mag_navmesh_route_v1" || self.status != "complete" {
            return Err("only a complete mag_navmesh_route_v1 is executable".into());
        }
        if self.world_sha256 != world_sha256
            || world_sha256.len() != 64
            || !world_sha256.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("route/world SHA256 mismatch".into());
        }
        if !goal_radius.is_finite()
            || !(0.01..=5.0).contains(&goal_radius)
            || !self.length_m.is_finite()
            || self.length_m < 0.0
            || self.waypoints.is_empty()
            || self.waypoints.len() > 4096
            || self.corridor_refs.is_empty()
            || self.corridor_refs.len() > 16384
            || self.corridor_refs.contains(&0)
            || [
                self.start,
                self.requested_goal,
                self.projected_start,
                self.projected_goal,
            ]
            .into_iter()
            .chain(self.waypoints.iter().copied())
            .any(|p| !valid_point(p))
        {
            return Err("invalid route bounds, corridor, or goal radius".into());
        }
        if distance(self.projected_goal, self.requested_goal) > goal_radius {
            return Err("projected goal is outside the requested physical goal radius".into());
        }
        if distance(*self.waypoints.last().unwrap(), self.projected_goal) > 0.25 {
            return Err("route endpoint does not reach its projected goal".into());
        }
        self.profile.body()
    }
}

#[derive(Debug)]
struct Options {
    world: PathBuf,
    route: PathBuf,
    counts: Vec<usize>,
    speed: f64,
    seconds: f64,
    goal_radius: f64,
    wall_seconds: f64,
    benchmark: bool,
}
fn options() -> Result<Options, String> {
    let mut args = env::args().skip(1);
    let (mut world, mut route, mut counts) = (None, None, None);
    let (mut speed, mut seconds, mut goal_radius, mut wall_seconds) = (3.0, None, 5.0, 60.0);
    let mut benchmark = false;
    while let Some(arg) = args.next() {
        if arg == "--benchmark" {
            benchmark = true;
            continue;
        }
        let value = args
            .next()
            .ok_or_else(|| format!("{arg} requires a value"))?;
        match arg.as_str() {
            "--world" => world = Some(PathBuf::from(value)),
            "--route" => route = Some(PathBuf::from(value)),
            "--counts" => {
                counts = Some(
                    value
                        .split(',')
                        .map(|s| s.parse::<usize>().map_err(|e| e.to_string()))
                        .collect::<Result<Vec<_>, _>>()?,
                )
            }
            "--speed" => speed = value.parse::<f64>().map_err(|e| e.to_string())?,
            "--seconds" => seconds = Some(value.parse::<f64>().map_err(|e| e.to_string())?),
            "--goal-radius" => goal_radius = value.parse::<f64>().map_err(|e| e.to_string())?,
            "--wall-seconds" => wall_seconds = value.parse::<f64>().map_err(|e| e.to_string())?,
            _ => return Err(format!("unknown option {arg}")),
        }
    }
    let options = Options {
        world: world.ok_or("--world is required")?,
        route: route.ok_or("--route is required")?,
        counts: counts.unwrap_or_else(|| {
            if benchmark {
                vec![1, 64, 128, 256]
            } else {
                vec![1]
            }
        }),
        speed,
        seconds: seconds.unwrap_or(if benchmark { 10.0 } else { 180.0 }),
        goal_radius,
        wall_seconds,
        benchmark,
    };
    if options.counts.is_empty()
        || options.counts.len() > 4
        || options.counts.iter().any(|n| !(1..=256).contains(n))
        || !options.speed.is_finite()
        || !(0.05..=6.0).contains(&options.speed)
        || !options.seconds.is_finite()
        || !(0.05..=600.0).contains(&options.seconds)
        || !options.wall_seconds.is_finite()
        || !(1.0..=300.0).contains(&options.wall_seconds)
    {
        return Err("invalid workload bounds".into());
    }
    Ok(options)
}

// Offline host utility, outside all measured movement work. Uses an argument
// vector (never a shell) and accepts only a successful 64-character digest.
fn sha256(path: &PathBuf) -> Result<String, String> {
    for (name, prefix) in [("sha256sum", vec![]), ("shasum", vec!["-a", "256"])] {
        let result = Command::new(name).args(prefix).arg(path).output();
        let Ok(output) = result else { continue };
        if output.status.success() {
            let digest = String::from_utf8_lossy(&output.stdout)
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_owned();
            if digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Ok(digest);
            }
        }
    }
    Err("a successful sha256sum or shasum -a 256 is required".into())
}

struct State {
    motion: Motion,
    corner: usize,
    start_delay: usize,
    calls: usize,
    reached: bool,
    stopped: Option<String>,
}
impl State {
    fn step(
        &mut self,
        terrain: &WorldTerrain<'_>,
        route: &Route,
        body: Body,
        speed: f64,
        goal_radius: f64,
    ) {
        while self.corner < route.waypoints.len()
            && xz_distance(self.motion.position, route.waypoints[self.corner]) <= ARRIVAL_XZ
        {
            self.corner += 1;
        }
        if self.motion.grounded
            && self.corner + 1 >= route.waypoints.len()
            && distance(self.motion.position, route.requested_goal) <= goal_radius
        {
            self.reached = true;
            return;
        }
        let mut velocity = [0.0; 3];
        if self.motion.grounded && self.corner < route.waypoints.len() {
            let target = route.waypoints[self.corner];
            let d = xz_distance(self.motion.position, target);
            if d > 1e-9 {
                // Consume the full 50ms tick. Slow the final step to a corner;
                // do not shorten elapsed time or overwrite the solver's pose.
                let speed = speed.min(d / TICK_SECONDS);
                velocity = [
                    (target[0] - self.motion.position[0]) / d * speed,
                    0.0,
                    (target[2] - self.motion.position[2]) / d * speed,
                ];
            }
        }
        self.calls += 1;
        match terrain.advance(self.motion, velocity, TICK_SECONDS, body) {
            Ok(motion) => {
                self.motion = motion;
                if motion.blocked {
                    self.stopped = Some("local_collision_blocked".into());
                }
            }
            Err(error) => self.stopped = Some(error.code().into()),
        }
    }
}

fn run(options: Options) -> Result<bool, String> {
    if fs::metadata(&options.route)
        .map_err(|e| e.to_string())?
        .len()
        > 1024 * 1024
    {
        return Err("route JSON exceeds one MiB".into());
    }
    let route: Route =
        serde_json::from_slice(&fs::read(&options.route).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    let world_sha256 = sha256(&options.world)?;
    let body = route.validate(&world_sha256, options.goal_radius)?;
    let loading = Instant::now();
    let world = CollisionWorld::load(&options.world, CollisionLimits::default())
        .map_err(|e| e.to_string())?;
    let load_us = loading.elapsed().as_micros();
    let terrain = WorldTerrain::new(Some(&world), &world.metadata().map_name)
        .map_err(|e| e.code().to_owned())?;
    let initial = terrain
        .start_motion(route.start, body)
        .map_err(|e| e.code().to_owned())?;
    let source = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|r| r.status.success())
        .map(|r| String::from_utf8_lossy(&r.stdout).trim().to_owned());
    let ticks = (options.seconds / TICK_SECONDS).ceil() as usize;
    let mut results = Vec::new();
    let mut all_reached = true;
    for &count in &options.counts {
        let mut states: Vec<_> = (0..count)
            .map(|i| State {
                motion: initial,
                corner: 0,
                start_delay: if count > 1 { i % 20 } else { 0 },
                calls: 0,
                reached: false,
                stopped: None,
            })
            .collect();
        let started = Instant::now();
        let mut ran_ticks = 0;
        let mut wall_limit = false;
        for tick in 0..ticks {
            if started.elapsed().as_secs_f64() >= options.wall_seconds {
                wall_limit = true;
                break;
            }
            let mut active = false;
            for state in &mut states {
                if state.reached || state.stopped.is_some() {
                    continue;
                }
                active = true;
                if tick < state.start_delay {
                    continue;
                }
                state.step(&terrain, &route, body, options.speed, options.goal_radius);
            }
            ran_ticks = tick + 1;
            if !active {
                break;
            }
        }
        let elapsed = started.elapsed();
        let calls: usize = states.iter().map(|s| s.calls).sum();
        let reached = states.iter().filter(|s| s.reached).count();
        let stopped = states.iter().filter(|s| s.stopped.is_some()).count();
        all_reached &= reached == count;
        results.push(json!({"states":count,"independent_motion_states":true,
            "elapsed_us":elapsed.as_micros(),"physics_calls":calls,
            "mean_movement_us_per_call":if calls>0 {Some(elapsed.as_secs_f64()*1e6/calls as f64)}else{None},
            "outer_ticks":ran_ticks,"tick_seconds":TICK_SECONDS,"wall_limit_reached":wall_limit,
            "reached":reached,"stopped":stopped,"unfinished":count-reached-stopped,
            "samples":states.iter().take(8).map(|s|json!({"position":s.motion.position,
                "grounded":s.motion.grounded,"blocked":s.motion.blocked,"vertical_velocity":s.motion.vertical_velocity,
                "corner":s.corner,"physics_calls":s.calls,"simulated_seconds":s.calls as f64*TICK_SECONDS,
                "distance_to_requested_goal":distance(s.motion.position,route.requested_goal),
                "reached":s.reached,"stopped":s.stopped})).collect::<Vec<_>>() }));
    }
    println!(
        "{}",
        json!({"format":"mag_navmesh_physics_replay_v1",
        "build_profile":if cfg!(debug_assertions){"debug"}else{"release"},"source_revision":source,
        "platform":format!("{}/{}",env::consts::OS,env::consts::ARCH),
        "mode":if options.benchmark{"benchmark"}else{"full_route_replay"},
        "world_sha256":world_sha256,"world_stats":world.stats(),"world_load_us":load_us,
        "route_query_us":route.query_us,"route_length_m":route.length_m,"corridor_polygons":route.corridor_refs.len(),
        "waypoints":route.waypoints.len(),"start":route.start,"requested_goal":route.requested_goal,
        "projected_start":route.projected_start,"projected_goal":route.projected_goal,
        "goal_radius":options.goal_radius,"speed_mps":options.speed,"results":results,
        "scope":"Sequential independent physical states sharing immutable world and route; actual solver feedback. No Detour local tracking/query time, inter-agent avoidance, Engine replication, network, autonomous scheduling, rendering, or game FPS included."})
    );
    Ok(options.benchmark || all_reached)
}
fn main() {
    match options().and_then(run) {
        Ok(true) => {}
        Ok(false) => std::process::exit(2),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn route() -> Route {
        serde_json::from_value(json!({"format":"mag_navmesh_route_v1","status":"complete",
            "world_sha256":"a".repeat(64),"profile":{"radius":0.35,"height":1.8,"max_step":0.4,
                "max_slope_degrees":45.,"skin":0.005,"bake_radius":0.6,"cell_size":0.1,"cell_height":0.05},
            "start":[0.,0.,0.],"requested_goal":[10.,0.,0.],"projected_start":[0.,0.,0.],
            "projected_goal":[10.,0.,0.],"waypoints":[[0.,0.,0.],[10.,0.,0.]],
            "corridor_refs":[1,2],"length_m":10.,"query_us":0})).unwrap()
    }
    #[test]
    fn executable_route_requires_matching_world_complete_status_and_box_clearance() {
        let mut r = route();
        assert!(r.validate(&"a".repeat(64), 5.0).is_ok());
        assert!(r.validate(&"b".repeat(64), 5.0).is_err());
        r.status = "partial".into();
        assert!(r.validate(&"a".repeat(64), 5.0).is_err());
        r.status = "complete".into();
        r.profile.bake_radius = 0.35;
        assert!(r.validate(&"a".repeat(64), 5.0).is_err());
    }
    #[test]
    fn projected_goal_cannot_fabricate_requested_arrival() {
        let mut r = route();
        r.projected_goal = [4.0, 0.0, 0.0];
        *r.waypoints.last_mut().unwrap() = r.projected_goal;
        assert!(r.validate(&"a".repeat(64), 5.0).is_err());
    }
}
