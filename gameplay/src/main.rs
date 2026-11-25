use mag_gameplay::collision::{CollisionLimits, CollisionWorld};
use mag_gameplay::telemetry::{TickRecorder, TickSample};
use mag_gameplay::worker_identity::{WORKER_ID_ERROR, WorkerIdentity};
use mag_gameplay::world_stream::{self, EventReplies, Journal, ResumeFrom};
use mag_gameplay::{
    Engine, EngineResources, IO_DEADLINE, MAX_LOG, Request, Response, parse_request, read_line,
};
use sha2::{Digest, Sha256};
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, SyncSender, TrySendError};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

fn parse_application_id(value: &str) -> io::Result<u32> {
    match value {
        "20230" => Ok(20230),
        "20231" => Ok(20231),
        _ => Err(io::Error::other("--application-id requires 20230 or 20231")),
    }
}

fn validate_runtime_paths(
    application_id: Option<u32>,
    paths: &[(&str, Option<&Path>)],
) -> io::Result<()> {
    if application_id != Some(20231) {
        return Ok(());
    }
    for (flag, path) in paths {
        match path {
            Some(path) if path.is_absolute() => {}
            _ => {
                return Err(io::Error::other(format!(
                    "retail startup requires an explicit absolute {flag}"
                )));
            }
        }
    }
    Ok(())
}

fn validate_item_catalog_links(items: &Path, linked: &[(&str, &Path, &str)]) -> io::Result<()> {
    let digest = format!("{:x}", Sha256::digest(std::fs::read(items)?));
    for (name, path, pointer) in linked {
        let metadata: serde_json::Value = serde_json::from_reader(std::fs::File::open(path)?)
            .map_err(|_| io::Error::other(format!("invalid {name} metadata")))?;
        if metadata
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            != Some(digest.as_str())
        {
            return Err(io::Error::other(format!(
                "{name} does not match the selected retail item catalog"
            )));
        }
    }
    Ok(())
}

type LogWriter = Box<dyn std::io::Write + Send>;

/// Open a NEW log file, segmented when a rotation policy is configured.
fn open_log(
    path: &std::path::Path,
    rotation: Option<mag_gameplay::rotating_log::RotationPolicy>,
) -> io::Result<LogWriter> {
    Ok(match rotation {
        Some(policy) => Box::new(mag_gameplay::rotating_log::RotatingLog::create(
            path.to_path_buf(),
            policy,
        )?),
        None => Box::new(
            OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(path)?,
        ),
    })
}

/// One delay for both teams, or "TEAM0,TEAM1". Range checks happen in the catalog.
fn parse_team_delays(value: &str) -> Option<[u32; 2]> {
    let parts: Vec<u32> = value
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    match parts.as_slice() {
        [both] => Some([*both, *both]),
        [team0, team1] => Some([*team0, *team1]),
        _ => None,
    }
}
fn main() -> io::Result<()> {
    run(std::env::args().skip(1))
}

fn validate_acquisition_static_partition(
    map_name: &str,
    partitions: &[mag_gameplay::static_collision_partition::Partition],
    filter_rows: &[u32; 32],
) -> io::Result<()> {
    partitions
        .iter()
        .find(|partition| partition.map_name() == map_name)
        .ok_or_else(|| io::Error::other("acquisition_gameplay_requires_static_partition"))?
        .require_excluded(6, filter_rows)
        .map_err(io::Error::other)
}

// Keep the by-value Engine builders on the runtime-only path. In debug
// builds their temporaries otherwise dominate the catalog validation frame.
#[inline(never)]
fn run_worker_startup(start: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    start()
}

fn run(args: impl IntoIterator<Item = String>) -> io::Result<()> {
    let mut application_id: Option<u32> = None;
    let mut validate_catalogs = false;
    let mut headless_sim_seconds: Option<u64> = None;
    let mut worker_identity = None;
    let mut bind = SocketAddr::from(([127, 0, 0, 1], 31000));
    let mut log: Option<PathBuf> = None;
    let mut log_segment_bytes: Option<u64> = None;
    let mut log_retention_s: Option<u64> = None;
    let mut telemetry_path: Option<PathBuf> = None;
    let mut tick_budget_us = None;
    let mut collision_path: Option<PathBuf> = None;
    let mut collision_coverage_path: Option<PathBuf> = None;
    let mut navmesh_path: Option<PathBuf> = None;
    let mut regional_walk_path: Option<PathBuf> = None;
    let mut movement_profile_path: Option<PathBuf> = None;
    let mut bot_population_path: Option<PathBuf> = None;
    let mut bot_resupply_catalog_path: Option<PathBuf> = None;
    let mut bot_resupply_approach_policy_path: Option<PathBuf> = None;
    let mut bot_names_path: Option<PathBuf> = None;
    let mut bot_fill_total: usize = 0;
    let mut bot_perception: Option<bool> = None;
    let mut initial_spawn_delay_ms: Option<[u32; 2]> = None;
    let mut match_profiles_path: Option<PathBuf> = None;
    let mut party_reservations = false;
    let mut active_abilities_path: Option<PathBuf> = None;
    let mut squad_orders_path: Option<PathBuf> = None;
    let mut spawn_path: Option<PathBuf> = None;
    let mut ground_vehicles_path: Option<PathBuf> = None;
    let mut dynamic_bodies_path: Option<PathBuf> = None;
    let mut static_partition_paths: Vec<PathBuf> = Vec::new();
    let mut acquisition_gameplay_path: Option<PathBuf> = None;
    let mut objective_path: Option<PathBuf> = None;
    let mut item_path: Option<PathBuf> = None;
    let mut blast_rules_path: Option<PathBuf> = None;
    let mut retail_metadata_path: Option<PathBuf> = None;
    let mut bootstrap_loadouts_path: Option<PathBuf> = None;
    let mut fall_rules_path: Option<PathBuf> = None;
    let mut incapacitation_rules_path: Option<PathBuf> = None;
    let mut character_damage_rules_path: Option<PathBuf> = None;
    let mut firearm_assist_rules_path: Option<PathBuf> = None;
    let mut skill_path: Option<PathBuf> = None;
    let mut retail_armory_path: Option<PathBuf> = None;
    let mut smoke_grenade_rules_path: Option<PathBuf> = None;
    let mut leadership_passives_path: Option<PathBuf> = None;
    let mut leadership_role_policy_path: Option<PathBuf> = None;
    let mut structure_path: Option<PathBuf> = None;
    let mut bunker_path: Option<PathBuf> = None;
    let mut bunker_exit_policy_path: Option<PathBuf> = None;
    let mut wire_profiles_path: Option<PathBuf> = None;
    let mut collision_query = None;
    let mut development_dummy = false;
    let mut development_dummy_position = None;
    let mut development_control = false;
    let mut development_grenade_radius = false;
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--acquisition-gameplay" => {
                let path = PathBuf::from(args.next().ok_or_else(|| {
                    io::Error::other(
                        "--acquisition-gameplay requires an absolute configuration path",
                    )
                })?);
                if !path.is_absolute() {
                    return Err(io::Error::other(
                        "--acquisition-gameplay requires an absolute configuration path",
                    ));
                }
                acquisition_gameplay_path = Some(path);
            }
            "--active-abilities" => {
                active_abilities_path = Some(PathBuf::from(args.next().ok_or_else(|| {
                    io::Error::other("--active-abilities requires an absolute manifest path")
                })?));
            }
            "--party-reservations" => party_reservations = true,
            "--match-profiles" => {
                match_profiles_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--match-profiles requires an absolute manifest path")
                        })?
                        .into(),
                );
            }
            "--application-id" => {
                application_id = Some(parse_application_id(&args.next().ok_or_else(|| {
                    io::Error::other("--application-id requires 20230 or 20231")
                })?)?);
            }
            "--validate-catalogs" => validate_catalogs = true,
            "--headless-sim-seconds" => {
                headless_sim_seconds = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--headless-sim-seconds requires N"))?
                        .parse::<u64>()
                        .map_err(|_| io::Error::other("invalid --headless-sim-seconds"))?,
                );
            }
            "--bot-perception" => {
                bot_perception = Some(
                    match args
                        .next()
                        .ok_or_else(|| io::Error::other("--bot-perception requires on or off"))?
                        .as_str()
                    {
                        "on" => true,
                        "off" => false,
                        _ => {
                            return Err(io::Error::other(
                                "invalid --bot-perception (use on or off)",
                            ));
                        }
                    },
                );
            }
            "--bot-names" => {
                bot_names_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bot-names requires a path"))?
                        .into(),
                );
            }
            "--bot-resupply-catalog" => {
                bot_resupply_catalog_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--bot-resupply-catalog requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--bot-resupply-approach-policy" => {
                bot_resupply_approach_policy_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other(
                                "--bot-resupply-approach-policy requires an absolute path",
                            )
                        })?
                        .into(),
                );
            }
            "--bot-population" => {
                bot_population_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bot-population requires a path"))?
                        .into(),
                );
            }
            "--initial-spawn-delay-ms" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::other("--initial-spawn-delay-ms requires milliseconds")
                })?;
                initial_spawn_delay_ms = Some(parse_team_delays(&value).ok_or_else(|| {
                    io::Error::other("invalid --initial-spawn-delay-ms (use N or TEAM0,TEAM1)")
                })?);
            }
            "--bot-fill-total" => {
                bot_fill_total = args
                    .next()
                    .ok_or_else(|| io::Error::other("--bot-fill-total requires a count"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --bot-fill-total"))?;
            }
            "--navmesh-world" => {
                navmesh_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--navmesh-world requires a path"))?
                        .into(),
                );
            }
            "--regional-walk-coverage" => {
                regional_walk_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--regional-walk-coverage requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--movement-profile" => {
                movement_profile_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--movement-profile requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--retail-combat-metadata" => {
                retail_metadata_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--retail-combat-metadata requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--character-blast-rules" => {
                blast_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--character-blast-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--wire-profiles" => {
                wire_profiles_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--wire-profiles requires a path"))?
                        .into(),
                );
            }
            "--tick-telemetry" => {
                telemetry_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--tick-telemetry requires a NEW path"))?
                        .into(),
                );
            }
            "--tick-budget-ms" => {
                let value: f64 = args
                    .next()
                    .ok_or_else(|| io::Error::other("--tick-budget-ms requires a number"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --tick-budget-ms"))?;
                if !value.is_finite() || !(0.001..=1000.0).contains(&value) {
                    return Err(io::Error::other("tick budget must be .001..1000 ms"));
                }
                tick_budget_us = Some((value * 1000.0).round() as u64);
            }
            "--firearm-assist-rules" => {
                firearm_assist_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--firearm-assist-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--character-damage-rules" => {
                character_damage_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--character-damage-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--incapacitation-rules" => {
                incapacitation_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--incapacitation-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--fall-damage-rules" => {
                fall_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--fall-damage-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--leadership-passives" => {
                leadership_passives_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--leadership-passives requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--leadership-role-policy" => {
                leadership_role_policy_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--leadership-role-policy requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--squad-orders" => {
                squad_orders_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--squad-orders requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--retail-armory-catalog" => {
                retail_armory_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--retail-armory-catalog requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--smoke-grenade-rules" => {
                smoke_grenade_rules_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--smoke-grenade-rules requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--skill-catalog" => {
                skill_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--skill-catalog requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--structure-catalog" => {
                structure_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--structure-catalog requires a path"))?
                        .into(),
                );
            }
            "--bunker-exit-policy" => {
                bunker_exit_policy_path =
                    Some(PathBuf::from(args.next().ok_or_else(|| {
                        io::Error::other("--bunker-exit-policy requires a path")
                    })?));
            }
            "--bunker-catalog" => {
                bunker_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--bunker-catalog requires a path"))?
                        .into(),
                );
            }
            "--bootstrap-loadouts" => {
                bootstrap_loadouts_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--bootstrap-loadouts requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--item-catalog" => {
                item_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--item-catalog requires a path"))?
                        .into(),
                );
            }
            "--objective-catalog" => {
                objective_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--objective-catalog requires a path"))?
                        .into(),
                );
            }
            "--dynamic-bodies" => {
                dynamic_bodies_path = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--dynamic-bodies requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--static-partition" => {
                if static_partition_paths.len() >= 64 {
                    return Err(io::Error::other("static_partition_limit"));
                }
                static_partition_paths.push(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--static-partition requires an absolute path")
                        })?
                        .into(),
                );
            }
            "--ground-vehicles" => {
                ground_vehicles_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--ground-vehicles requires a path"))?
                        .into(),
                );
            }
            "--spawn-catalog" => {
                spawn_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--spawn-catalog requires a path"))?
                        .into(),
                );
            }
            "--collision-world" => {
                collision_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--collision-world requires a path"))?
                        .into(),
                );
            }
            "--collision-coverage" => {
                collision_coverage_path = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--collision-coverage requires a path"))?
                        .into(),
                );
            }
            "--collision-query" => {
                let start = args.next().ok_or_else(|| {
                    io::Error::other("--collision-query requires START END as x,y,z")
                })?;
                let end = args.next().ok_or_else(|| {
                    io::Error::other("--collision-query requires START END as x,y,z")
                })?;
                collision_query = Some((parse_point(&start)?, parse_point(&end)?));
            }
            "--development-dummy-position" => {
                let value = args.next().ok_or_else(|| {
                    io::Error::other("--development-dummy-position requires x,y,z")
                })?;
                let [x, y, z] = parse_point(&value)?;
                let position = mag_gameplay::replication::CharacterPosition {
                    x: x as f32,
                    y: y as f32,
                    z: z as f32,
                };
                development_dummy_position = Some(position);
            }
            "--development-dummy" => development_dummy = true,
            "--development-control" => development_control = true,
            "--development-grenade-radius" => development_grenade_radius = true,
            "--worker-id" => {
                let id = args
                    .next()
                    .ok_or_else(|| io::Error::other(WORKER_ID_ERROR))?;
                worker_identity = Some(WorkerIdentity::start(id)?);
            }
            "--bind" => {
                bind = args
                    .next()
                    .ok_or_else(|| io::Error::other("--bind requires an address"))?
                    .parse()
                    .map_err(|_| io::Error::other("invalid --bind address"))?
            }
            "--log" => {
                log = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--log requires a new path"))?
                        .into(),
                )
            }
            "--log-segment-bytes" => {
                log_segment_bytes = Some(
                    args.next()
                        .ok_or_else(|| {
                            io::Error::other("--log-segment-bytes requires a byte count")
                        })?
                        .parse()
                        .map_err(|_| io::Error::other("invalid --log-segment-bytes"))?,
                );
            }
            "--log-retention-s" => {
                log_retention_s = Some(
                    args.next()
                        .ok_or_else(|| io::Error::other("--log-retention-s requires seconds"))?
                        .parse()
                        .map_err(|_| io::Error::other("invalid --log-retention-s"))?,
                );
            }
            _ => {
                return Err(io::Error::other(
                    "usage: mag-gameplay [--application-id 20230|20231] [--validate-catalogs] --item-catalog ITEMS.json --spawn-catalog SPAWNS.json --log NEW_FILE [--match-profiles /absolute/MATCH_PROFILES.json] [--party-reservations] [--active-abilities /absolute/ACTIVE_ABILITIES.json] [--squad-orders /absolute/ORDERS.json] [--wire-profiles PROFILES.json] [--objective-catalog OBJECTIVES.json] [--ground-vehicles /absolute/VEHICLES.json] [--dynamic-bodies /absolute/BODIES.json] [--static-partition /absolute/PARTITION.json] [--acquisition-gameplay /absolute/GAMEPLAY.json] [--character-blast-rules /absolute/BLAST.json] [--retail-combat-metadata /absolute/RETAIL.json] [--bootstrap-loadouts /absolute/BOOTSTRAP.json] [--fall-damage-rules /absolute/FALL.json] [--incapacitation-rules /absolute/INCAPACITATION.json] [--character-damage-rules /absolute/DAMAGE.json] [--firearm-assist-rules /absolute/ASSISTS.json] [--skill-catalog /absolute/SKILLS.json] [--retail-armory-catalog /absolute/ARMORY.json] [--smoke-grenade-rules /absolute/SMOKE.json] [--leadership-passives /absolute/PASSIVES.json --leadership-role-policy /absolute/ROLES.json] [--bind IP:PORT] [--worker-id ID] [--development-control] [--development-grenade-radius] [--development-dummy] [--development-dummy-position x,y,z] [--collision-world WORLD.json [--collision-coverage COVERAGE.json]] [--navmesh-world MESH.mag-nav] [--regional-walk-coverage /absolute/COVERAGE.json] [--movement-profile /absolute/MOVEMENT.json] [--bot-fill-total N --bot-population POPULATION.json] [--bot-perception on|off] [--bot-names /absolute/NAMES.json] [--bot-resupply-catalog /absolute/SUPPLY.json] [--bot-resupply-approach-policy /absolute/APPROACH.json] [--tick-telemetry NEW_FILE] [--tick-budget-ms MS] [--log-segment-bytes BYTES [--log-retention-s SECONDS]]; query only: mag-gameplay --collision-world WORLD.json --collision-query x,y,z x,y,z",
                ));
            }
        }
    }
    if let Some(path) = &regional_walk_path
        && (!path.is_absolute()
            || application_id != Some(20231)
            || navmesh_path.is_none()
            || bot_population_path.is_none()
            || bot_fill_total == 0
            || match_profiles_path.is_some()
            || movement_profile_path.is_some()
            || development_dummy
            || collision_query.is_some())
    {
        return Err(io::Error::other(
            "--regional-walk-coverage requires an absolute path, direct retail config, regional bot population and default-profile navmesh",
        ));
    }
    if let Some(path) = &movement_profile_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--movement-profile requires an absolute path",
            ));
        }
        if application_id != Some(20231) {
            return Err(io::Error::other(
                "--movement-profile requires --application-id 20231",
            ));
        }
        if match_profiles_path.is_some() {
            return Err(io::Error::other(
                "configure movement profiles inside each match profile",
            ));
        }
        if navmesh_path.is_none() {
            return Err(io::Error::other(
                "--movement-profile requires --navmesh-world",
            ));
        }
        if collision_query.is_some() {
            return Err(io::Error::other(
                "--movement-profile requires a gameplay worker",
            ));
        }
    }
    if party_reservations && match_profiles_path.is_none() {
        return Err(io::Error::other(
            "--party-reservations requires --match-profiles",
        ));
    }
    if match_profiles_path.is_some() {
        if application_id != Some(20231) {
            return Err(io::Error::other(
                "--match-profiles requires --application-id 20231",
            ));
        }
        // Bot populations and resupply providers are map-bound; profiles carry
        // their own. `--bot-fill-total` and `--bot-names` remain worker-wide
        // defaults for profiles that do not set their own.
        if bot_population_path.is_some()
            || bot_resupply_catalog_path.is_some()
            || bot_resupply_approach_policy_path.is_some()
            || development_dummy
            || structure_path.is_some()
            || bunker_path.is_some()
            || bunker_exit_policy_path.is_some()
            || collision_query.is_some()
        {
            return Err(io::Error::other(
                "--match-profiles does not support global bunker arguments, global bot population or resupply (configure bots in each profile), dummy or collision-query",
            ));
        }
    }
    if let Some(path) = &smoke_grenade_rules_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--smoke-grenade-rules requires an absolute path",
            ));
        }
        if application_id != Some(20231) || collision_query.is_some() {
            return Err(io::Error::other(
                "--smoke-grenade-rules requires a retail gameplay worker",
            ));
        }
        // A registry worker binds the rules to each profile's own objective
        // catalog; a direct worker binds them to its base catalog.
        if match_profiles_path.is_none() && objective_path.is_none() {
            return Err(io::Error::other(
                "--smoke-grenade-rules requires --objective-catalog",
            ));
        }
    }
    if let Some(path) = &active_abilities_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--active-abilities requires an absolute manifest path",
            ));
        }
        if match_profiles_path.is_some() {
            return Err(io::Error::other(
                "configure active abilities inside each match profile",
            ));
        }
        if application_id != Some(20231) || collision_query.is_some() {
            return Err(io::Error::other(
                "--active-abilities requires a retail gameplay worker",
            ));
        }
    }
    if validate_catalogs && collision_query.is_some() {
        return Err(io::Error::other(
            "--validate-catalogs cannot be combined with --collision-query",
        ));
    }
    if development_dummy_position.is_some() && !development_dummy {
        return Err(io::Error::other(
            "--development-dummy-position requires --development-dummy",
        ));
    }
    // Geometry is loaded before opening logs/listeners. A bad file cannot
    // silently fall back to an empty collision world or accept a reservation.
    if collision_coverage_path.is_some() && match_profiles_path.is_some() {
        return Err(io::Error::other(
            "configure collision coverage inside each match profile",
        ));
    }
    let collision_world = collision_path
        .as_ref()
        .map(|path| {
            CollisionWorld::load(path, CollisionLimits::default()).map_err(io::Error::other)
        })
        .transpose()?;
    // A coverage sidecar must name this exact world; the world bytes stay unchanged.
    let collision_world = match (&collision_coverage_path, collision_world) {
        (Some(_), None) => {
            return Err(io::Error::other(
                "--collision-coverage requires --collision-world",
            ));
        }
        (Some(path), Some(world)) => Some(
            world
                .with_coverage_sidecar(mag_gameplay::collision_coverage::CoverageSidecar::load(
                    path,
                )?)
                .map_err(io::Error::other)?,
        ),
        (None, world) => world,
    };
    let navmesh_world = if let Some(path) = navmesh_path {
        let world = collision_world
            .as_ref()
            .ok_or_else(|| io::Error::other("--navmesh-world requires --collision-world"))?;
        if std::fs::metadata(&path)?.len() > mag_gameplay::navmesh::MAX_ARTIFACT_JSON_BYTES {
            return Err(io::Error::other("navmesh artifact size limit"));
        }
        use mag_gameplay::navmesh::{self, MovementProfile, NavMesh};
        let raw = std::fs::read(path)?;
        let mesh = if let Some(path) = regional_walk_path.as_ref() {
            let artifact = serde_json::from_slice(&raw).map_err(io::Error::other)?;
            let coverage = navmesh::regional::Coverage::load(path)?;
            let mesh =
                navmesh::regional::load(artifact, world, coverage).map_err(io::Error::other)?;
            let population = mag_gameplay::bot_registry::BotPopulation::load(
                bot_population_path
                    .as_ref()
                    .expect("checked regional bot population"),
            )
            .map_err(io::Error::other)?;
            population
                .regional_collision
                .as_ref()
                .filter(|_| !population.autonomous)
                .ok_or_else(|| io::Error::other("bot_regional_walking_requires_fixed_region"))?
                .validate_walking_mesh(&mesh)
                .map_err(io::Error::other)?;
            mesh
        } else {
            let artifact = serde_json::from_slice(&raw).map_err(io::Error::other)?;
            let hash = world.source_sha256();
            let map = &world.metadata().map_name;
            match movement_profile_path.as_ref() {
                Some(path) => NavMesh::load_with_movement_profile(
                    artifact,
                    hash,
                    map,
                    &MovementProfile::load(path)?,
                ),
                None => NavMesh::load(artifact, hash, map),
            }
            .map_err(io::Error::other)?
        };
        mesh.validate_coverage(world).map_err(io::Error::other)?;
        Some(mesh)
    } else {
        None
    };
    if let Some((start, end)) = collision_query {
        if log.is_some()
            || telemetry_path.is_some()
            || tick_budget_us.is_some()
            || log_segment_bytes.is_some()
            || log_retention_s.is_some()
        {
            return Err(io::Error::other(
                "--collision-query is standalone; omit --log and telemetry flags",
            ));
        }
        let world = collision_world
            .as_ref()
            .ok_or_else(|| io::Error::other("--collision-query requires --collision-world"))?;
        let query = world
            .raycast_segment(start, end)
            .map_err(io::Error::other)?;
        println!(
            "{}",
            serde_json::json!({"event":"collision_query", "source":collision_path,
            "stats":world.stats(), "query":query, "authority":"diagnostic_only"})
        );
        return Ok(());
    }
    if log_retention_s.is_some() && log_segment_bytes.is_none() {
        return Err(io::Error::other(
            "--log-retention-s requires --log-segment-bytes",
        ));
    }
    let rotation = log_segment_bytes
        .map(|segment_bytes| {
            mag_gameplay::rotating_log::RotationPolicy {
                segment_bytes,
                retention: log_retention_s.map(Duration::from_secs),
            }
            .validate()
            .map_err(io::Error::other)
        })
        .transpose()?;
    if tick_budget_us.is_some() && telemetry_path.is_none() {
        return Err(io::Error::other(
            "--tick-budget-ms requires --tick-telemetry",
        ));
    }
    if validate_catalogs && application_id.is_none() {
        return Err(io::Error::other(
            "--validate-catalogs requires --application-id",
        ));
    }
    validate_runtime_paths(
        application_id,
        &[
            ("--item-catalog", item_path.as_deref()),
            ("--spawn-catalog", spawn_path.as_deref()),
            ("--wire-profiles", wire_profiles_path.as_deref()),
            ("--character-blast-rules", blast_rules_path.as_deref()),
            ("--bootstrap-loadouts", bootstrap_loadouts_path.as_deref()),
            ("--retail-combat-metadata", retail_metadata_path.as_deref()),
            ("--retail-armory-catalog", retail_armory_path.as_deref()),
            ("--skill-catalog", skill_path.as_deref()),
            ("--fall-damage-rules", fall_rules_path.as_deref()),
        ],
    )?;
    if application_id == Some(20231) {
        // Required paths were checked above. Verify projections identify these exact
        // item bytes rather than accepting unrelated catalogs with the same app ID.
        validate_item_catalog_links(
            item_path.as_deref().unwrap(),
            &[
                (
                    "bootstrap loadouts",
                    bootstrap_loadouts_path.as_deref().unwrap(),
                    "/provenance/catalog_sha256",
                ),
                (
                    "retail combat",
                    retail_metadata_path.as_deref().unwrap(),
                    "/provenance/catalog_sha256",
                ),
                (
                    "retail Armory",
                    retail_armory_path.as_deref().unwrap(),
                    "/source_sha256/gameplay~1data~1retail-items.json",
                ),
                (
                    "retail skills",
                    skill_path.as_deref().unwrap(),
                    "/source_sha256/gameplay~1data~1retail-items.json",
                ),
            ],
        )?;
    }
    if application_id == Some(20231)
        && (structure_path.is_some() || bunker_path.is_some() || bunker_exit_policy_path.is_some())
    {
        return Err(io::Error::other(
            "retail structure/bunker authority is unverified; omit structure/bunker inputs for the supported no-bunker profile",
        ));
    }
    // Validate once before listening; the installed catalog resolves ordinary retail hits.
    let mut retail_combat_catalog = None;
    let mut retail_metadata = match &retail_metadata_path {
        Some(path) => {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "retail combat metadata requires an absolute path",
                ));
            }
            let data = mag_gameplay::retail_combat::RetailCombatCatalog::load(path)
                .map_err(|error| io::Error::other(error.to_string()))?;
            let info = Some(
                serde_json::json!({"path":path,"application_id":data.application_id(),"scope":"ordinary_client_reported_bullets",
                "bullets":data.bullets().len(),"grenades":data.grenades().len(),"armor":data.armor().len(),"body":data.body().len(),
                "healing":data.healing().len(),"authority":data.authority()}),
            );
            retail_combat_catalog = Some(data);
            info
        }
        None => None,
    };
    let spawn_path = spawn_path
        .ok_or_else(|| io::Error::other("--spawn-catalog is required for gameplay startup"))?;
    let catalog = mag_gameplay::spawn_catalog::SpawnCatalog::load(&spawn_path)?;
    let catalog = match initial_spawn_delay_ms {
        Some(delay) => catalog
            .with_initial_spawn_delay_ms(delay)
            .map_err(io::Error::other)?,
        None => catalog,
    };
    let effective_initial_spawn_delay_ms = catalog.match_profile().initial_spawn_delay_ms;
    if ground_vehicles_path
        .as_ref()
        .is_some_and(|path| !path.is_absolute())
    {
        return Err(io::Error::other(
            "--ground-vehicles requires an explicit absolute path",
        ));
    }
    if ground_vehicles_path.is_some() && application_id != Some(20231) {
        return Err(io::Error::other(
            "--ground-vehicles requires explicit retail application20231",
        ));
    }
    let mut ground_vehicles = ground_vehicles_path
        .as_ref()
        .map(|path| mag_gameplay::ground_vehicles::Catalog::load(path, &catalog))
        .transpose()?;
    if acquisition_gameplay_path.is_none()
        && ground_vehicles
            .as_ref()
            .is_some_and(|ground| ground.has_acquisition_apc_crew())
    {
        return Err(io::Error::other(
            "acquisition_apc_crew_requires_acquisition_gameplay",
        ));
    }
    let dynamic_bodies = dynamic_bodies_path
        .as_ref()
        .map(|path| -> io::Result<_> {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--dynamic-bodies requires an absolute path",
                ));
            }
            if application_id != Some(20231) || !matches!(catalog.match_profile().game_type, 1 | 2)
            {
                return Err(io::Error::other(
                    "dynamic bodies require retail Acquisition or Domination",
                ));
            }
            let bodies = mag_gameplay::dynamic_collision::Catalog::load(path)?;
            let bindings = bodies
                .runtime_bindings()
                .cloned()
                .ok_or_else(|| io::Error::other("dynamic_body_bindings_required"))?;
            bindings
                .validate_for(&bodies, ground_vehicles.as_ref())
                .map_err(io::Error::other)?;
            Ok((bodies, bindings))
        })
        .transpose()?;
    let static_partitions = static_partition_paths
        .iter()
        .map(|path| -> io::Result<_> {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--static-partition requires an absolute path",
                ));
            }
            if application_id != Some(20231) || !matches!(catalog.match_profile().game_type, 1 | 2)
            {
                return Err(io::Error::other(
                    "static partitions require retail Acquisition or Domination",
                ));
            }
            mag_gameplay::static_collision_partition::Partition::load(path)
        })
        .collect::<io::Result<Vec<_>>>()?;
    if !static_partitions.is_empty() {
        mag_gameplay::static_collision_partition::Partition::validate_collection(
            &static_partitions,
            catalog.map_name(),
        )
        .map_err(io::Error::other)?;
    }
    if application_id.is_some()
        && collision_world
            .as_ref()
            .is_some_and(|world| world.metadata().map_name != catalog.map_name())
    {
        return Err(io::Error::other(
            "collision world map does not match the selected spawn catalog",
        ));
    }
    if application_id == Some(20231) && catalog.match_profile().has_bunkers {
        return Err(io::Error::other(
            "retail bunker features are unsupported without verified retail authority",
        ));
    }
    let collision_info = collision_world.as_ref().map(|world| serde_json::json!({
        "source":collision_path,"metadata":world.metadata(),"coverage_sidecar":world.coverage_sidecar().map(|sidecar| serde_json::json!({"source":collision_coverage_path,"sha256":sidecar.source_sha256(),"basis":sidecar.basis(),"regions":sidecar.regions()})),"stats":world.stats(),"bullet_authority":"audit_only","grenade_policy":"disk_triangles_three_body_samples","grenade_radius_enabled":development_grenade_radius,"active_map_name":catalog.map_name(),"map_matches_active":world.metadata().map_name == catalog.map_name()}));
    let objective_catalog = objective_path
        .as_ref()
        .map(mag_gameplay::objective_catalog::ObjectiveCatalog::load)
        .transpose()?;
    if let Some(objectives) = &objective_catalog {
        objectives
            .validate_match(catalog.map_name(), catalog.match_profile())
            .map_err(io::Error::other)?;
    }
    catalog
        .validate_objectives(objective_catalog.as_ref())
        .map_err(io::Error::other)?;
    let (acquisition_config, acquisition_setup) = if let Some(path) = &acquisition_gameplay_path {
        if application_id != Some(20231) || catalog.match_profile().game_type != 1 {
            return Err(io::Error::other(
                "Acquisition gameplay requires retail mode1",
            ));
        }
        if match_profiles_path.is_some() {
            return Err(io::Error::other(
                "configure Acquisition gameplay inside each match profile",
            ));
        }
        let world = collision_world
            .as_ref()
            .ok_or_else(|| io::Error::other("acquisition_gameplay_requires_collision_world"))?;
        if character_damage_rules_path.is_none() {
            return Err(io::Error::other(
                "acquisition_gameplay_requires_character_damage_rules",
            ));
        }
        if world.stats().triangles == 0 {
            return Err(io::Error::other(
                "acquisition_gameplay_requires_nonempty_collision_world",
            ));
        }
        let (body_catalog, _) = dynamic_bodies
            .as_ref()
            .ok_or_else(|| io::Error::other("acquisition_gameplay_requires_dynamic_bodies"))?;
        let vehicles = ground_vehicles
            .as_ref()
            .ok_or_else(|| io::Error::other("Acquisition gameplay requires ground vehicles"))?;
        let objectives = objective_catalog.as_ref().ok_or_else(|| {
            io::Error::other("Acquisition gameplay requires an objective catalog")
        })?;
        let runtime = mag_gameplay::acquisition_ground_runtime::Runtime::from_catalog(vehicles)
            .map_err(io::Error::other)?;
        let config = mag_gameplay::acquisition_gameplay_config::Configuration::load(path)?;
        let setup = config.build(&catalog, objectives, &runtime)?;
        validate_acquisition_static_partition(
            catalog.map_name(),
            &static_partitions,
            &setup.filter_rows,
        )?;
        setup
            .validate_ground_drivers(vehicles)
            .map_err(io::Error::other)?;
        setup
            .validate_ground_hulk_templates(body_catalog)
            .map_err(io::Error::other)?;
        (Some(config), Some(setup))
    } else {
        if match_profiles_path.is_none()
            && objective_catalog
                .as_ref()
                .is_some_and(|o| o.source().capture_qualification == "native_source_gameplay")
        {
            return Err(io::Error::other(
                if objective_catalog
                    .as_ref()
                    .is_some_and(|o| o.is_domination_gameplay())
                {
                    "Domination source gameplay requires a connected gameplay backend"
                } else {
                    "source gameplay objectives require --acquisition-gameplay"
                },
            ));
        }
        (None, None)
    };
    let objective_info = objective_catalog.as_ref().map(|objectives| serde_json::json!({"path":objective_path,"object_count":objectives.object_count(),"source":objectives.source()}));
    let item_path = item_path
        .ok_or_else(|| io::Error::other("--item-catalog is required for gameplay startup"))?;
    let blast_rules_path =
        blast_rules_path.unwrap_or_else(|| item_path.with_file_name("character-blast-rules.json"));
    if !blast_rules_path.is_absolute() {
        return Err(io::Error::other(
            "character-blast rules require an absolute path",
        ));
    }
    let mut item_catalog = match application_id {
        Some(id) => mag_gameplay::item_catalog::ItemCatalog::load_for_application(
            &item_path,
            &blast_rules_path,
            id,
        )?,
        None => mag_gameplay::item_catalog::ItemCatalog::load_with_blast_rules(
            &item_path,
            &blast_rules_path,
        )?,
    };
    if let Some(path) = &bootstrap_loadouts_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--bootstrap-loadouts requires an absolute path",
            ));
        }
        item_catalog = item_catalog.with_bootstrap_loadouts(
            mag_gameplay::bootstrap_loadouts::BootstrapLoadouts::load(path)?,
        );
    }
    item_catalog
        .validate_bootstrap_faction(catalog.match_profile().defending_faction)
        .map_err(io::Error::other)?;
    if let Some(combat) = retail_combat_catalog {
        item_catalog = item_catalog.with_retail_combat(combat);
        // Character firearms beyond the projected rows come from the loaded
        // item catalog; record which ones did and did not qualify.
        if let (Some(info), Ok(combat)) = (retail_metadata.as_mut(), item_catalog.retail_combat()) {
            info["item_catalog_bullets"] = serde_json::json!(combat.derived_bullet_ids());
            info["item_catalog_bullet_skips"] = item_catalog
                .derived_bullet_skips()
                .iter()
                .map(|skip| serde_json::json!({"item":skip.item_id,"reason":skip.reason}))
                .collect();
        }
    }
    if let Some(path) = &retail_armory_path {
        if !path.is_absolute() {
            return Err(io::Error::other(
                "--retail-armory-catalog requires an absolute path",
            ));
        }
        item_catalog =
            item_catalog.with_retail_armory(mag_gameplay::retail_armory::RetailArmory::load(path)?);
    }
    if application_id == Some(20231) {
        let armory = item_catalog
            .retail_armory()
            .ok_or_else(|| io::Error::other("retail Armory required"))?;
        for faction in 0..3 {
            let bootstrap = item_catalog.bootstrap_loadout(faction, 0).ok_or_else(|| {
                io::Error::other("retail bootstrap must include all supported factions")
            })?;
            let saved = mag_gameplay::loadouts::SavedLoadouts {
                layout_revision: 0,
                selected_index: 0,
                update_ids: std::array::from_fn(|index| bootstrap.inventories[index].update_id),
                inventories: std::array::from_fn(|index| bootstrap.inventories[index].slots),
            };
            for inventory in &saved.inventories {
                for &item in inventory.iter().flatten() {
                    item_catalog.validate_item(item).map_err(io::Error::other)?;
                }
            }
            armory
                .validate_saved(faction, &saved)
                .map_err(io::Error::other)?;
        }
    }
    // Smoke rules come only from the command line, for every mode. A direct
    // worker installs them on its world; a registry worker binds them to
    // each of its profiles below.
    let mut smoke_grenade_rules = smoke_grenade_rules_path
        .as_ref()
        .map(|path| {
            let mut raw = Vec::new();
            std::fs::File::open(path)?
                .take((1 << 20) + 1)
                .read_to_end(&mut raw)?;
            if raw.len() > 1 << 20 {
                return Err(io::Error::other("invalid_smoke_grenade_rules"));
            }
            Ok::<_, io::Error>(raw)
        })
        .transpose()?;
    let smoke_grenade_source = if match_profiles_path.is_some() {
        None
    } else {
        smoke_grenade_rules.take()
    };
    let smoke_grenades = smoke_grenade_source
        .map(|source| {
            mag_gameplay::smoke_grenades::Catalog::from_bytes(
                &source,
                &item_catalog,
                &objective_catalog
                    .as_ref()
                    .ok_or_else(|| {
                        io::Error::other("smoke grenade rules require an objective catalog")
                    })?
                    .source()
                    .source_image_sha256,
            )
            .map_err(io::Error::other)
        })
        .transpose()?;
    let gas_grenades = acquisition_config
        .as_ref()
        .map(|config| config.gas_grenade_source())
        .transpose()?
        .flatten()
        .map(|source| {
            mag_gameplay::gas_grenades::Catalog::from_bytes(
                &source,
                &item_catalog,
                &objective_catalog
                    .as_ref()
                    .expect("direct Acquisition objectives")
                    .source()
                    .source_image_sha256,
            )
            .map_err(io::Error::other)
        })
        .transpose()?;
    if gas_grenades.is_some()
        && (dynamic_bodies.is_none()
            || acquisition_setup.as_ref().is_none_or(|setup| {
                setup.structural.is_none() || setup.filter_rows[6] & (1 << 21) == 0
            }))
    {
        return Err(io::Error::other("gas_grenades_require_character_collision"));
    }
    let flash_grenades = acquisition_config
        .as_ref()
        .map(|config| config.flash_grenade_source())
        .transpose()?
        .flatten()
        .map(|source| {
            mag_gameplay::flash_grenades::Catalog::from_bytes(
                &source,
                &item_catalog,
                &objective_catalog
                    .as_ref()
                    .expect("direct Acquisition objectives")
                    .source()
                    .source_image_sha256,
            )
            .map_err(io::Error::other)
        })
        .transpose()?;
    if flash_grenades.is_some()
        && (dynamic_bodies.is_none()
            || acquisition_setup.as_ref().is_none_or(|setup| {
                setup.structural.is_none()
                    || !setup.has_projectile_contacts()
                    || setup.filter_rows[24] != 0x0040_0601
                    || setup.filter_rows[6] & (1 << 21) == 0
            }))
    {
        return Err(io::Error::other(
            "flash_grenades_require_character_collision",
        ));
    }
    let underbarrel_launchers = acquisition_config
        .as_ref()
        .map(|config| config.underbarrel_launcher_source())
        .transpose()?
        .flatten()
        .map(|source| {
            mag_gameplay::underbarrel_launchers::map_support(
                mag_gameplay::acquisition_gameplay_config::validate_infantry_rocket_support(
                    acquisition_setup
                        .as_ref()
                        .expect("direct Acquisition setup"),
                ),
            )
            .map_err(io::Error::other)?;
            mag_gameplay::underbarrel_launchers::Catalog::from_bytes(
                &source,
                &item_catalog,
                &objective_catalog
                    .as_ref()
                    .expect("direct Acquisition objectives")
                    .source()
                    .source_image_sha256,
            )
            .map_err(io::Error::other)
        })
        .transpose()?;
    let guided_launchers = acquisition_config
        .as_ref()
        .map(|config| config.guided_launcher_source())
        .transpose()?
        .flatten()
        .map(|source| {
            mag_gameplay::acquisition_gameplay_config::validate_infantry_rocket_support(
                acquisition_setup
                    .as_ref()
                    .expect("direct Acquisition setup"),
            )
            .map_err(io::Error::other)?;
            mag_gameplay::guided_launchers::Catalog::from_bytes(
                &source,
                &item_catalog,
                &objective_catalog
                    .as_ref()
                    .expect("direct Acquisition objectives")
                    .source()
                    .source_image_sha256,
            )
            .map_err(io::Error::other)
        })
        .transpose()?;
    let infantry_rockets = if let Some(config) = acquisition_config {
        // Direct Acquisition setup already requires both catalogs.
        config.bind_ground_turrets(
            ground_vehicles.as_mut().unwrap(),
            &catalog,
            &item_catalog,
            objective_catalog.as_ref().unwrap(),
        )?;
        if let Some(source) = config.infantry_rocket_source()? {
            let setup = acquisition_setup
                .as_ref()
                .expect("direct Acquisition setup");
            mag_gameplay::acquisition_gameplay_config::validate_infantry_rocket_support(setup)
                .map_err(io::Error::other)?;
            Some(
                mag_gameplay::infantry_rockets::Catalog::from_bytes(
                    &source,
                    &item_catalog,
                    &objective_catalog
                        .as_ref()
                        .unwrap()
                        .source()
                        .source_image_sha256,
                )
                .map_err(io::Error::other)?,
            )
        } else {
            None
        }
    } else {
        None
    };
    let item_catalog = Arc::new(item_catalog);
    if let Some(setup) = &acquisition_setup {
        setup
            .validate_human_static_resupply(
                &catalog,
                collision_world.as_ref(),
                objective_catalog
                    .as_ref()
                    .expect("direct Acquisition objectives"),
                &item_catalog,
            )
            .map_err(io::Error::other)?;
        setup
            .validate_projectile_contacts(
                collision_world.as_ref(),
                dynamic_bodies.as_ref().map(|(catalog, _)| catalog),
            )
            .map_err(io::Error::other)?;
        setup
            .validate_frag_character_blast(&item_catalog, collision_world.as_ref())
            .map_err(io::Error::other)?;
        if setup.has_frag_ground_areas() {
            let vehicles = ground_vehicles
                .as_ref()
                .ok_or_else(|| io::Error::other("acquisition_frag_ground_requires_vehicles"))?;
            let objectives = objective_catalog
                .as_ref()
                .ok_or_else(|| io::Error::other("acquisition_frag_ground_requires_objectives"))?;
            setup
                .validate_frag_ground(
                    &item_catalog,
                    collision_world.as_ref(),
                    vehicles,
                    objectives,
                )
                .map_err(io::Error::other)?;
            let (bodies, bindings) = dynamic_bodies.as_ref().ok_or_else(|| {
                io::Error::other("acquisition_frag_ground_requires_dynamic_bodies")
            })?;
            setup
                .validate_frag_ground_dynamic(bodies, bindings, vehicles)
                .map_err(io::Error::other)?;
        }
    }
    let wire_profiles_path =
        wire_profiles_path.unwrap_or_else(|| item_path.with_file_name("wire-profiles.json"));
    let wire_profiles = mag_gameplay::wire_profile::WireProfileCatalog::load(&wire_profiles_path)?;
    if let Some(id) = application_id {
        let profile = wire_profiles.profile(id).map_err(io::Error::other)?;
        if let Some(objectives) = &objective_catalog {
            objectives
                .validate_profile(profile)
                .map_err(io::Error::other)?;
        } else if catalog.match_profile().has_objectives {
            return Err(io::Error::other(
                "objective catalog required when the match profile enables objectives",
            ));
        }
    }
    if skill_path.as_ref().is_some_and(|path| !path.is_absolute()) {
        return Err(io::Error::other(
            "--skill-catalog requires an absolute path",
        ));
    }
    let skill_path = match skill_path {
        Some(path) => path,
        None => std::env::current_dir()?
            .join(
                item_path
                    .parent()
                    .unwrap_or_else(|| std::path::Path::new(".")),
            )
            .join("prototype-skills.json"),
    };
    let skill_catalog = mag_gameplay::skill_catalog::SkillCatalog::load(&skill_path)?;
    if application_id.is_some_and(|id| skill_catalog.application_id() != id) {
        return Err(io::Error::other(
            "skill catalog application does not match --application-id",
        ));
    }
    let leadership = match (&leadership_passives_path, &leadership_role_policy_path) {
        (None, None) => None,
        (Some(passives), Some(policy)) => {
            if !passives.is_absolute() || !policy.is_absolute() {
                return Err(io::Error::other(
                    "leadership configuration paths must be absolute",
                ));
            }
            let catalog = mag_gameplay::leadership_passives::LeadershipCatalog::load(passives)?;
            let policy = mag_gameplay::leadership_runtime::RolePolicy::load(policy)?;
            if policy.has_development_bindings() && !development_control {
                return Err(io::Error::other(
                    "leadership development bindings require --development-control",
                ));
            }
            Some((catalog, policy))
        }
        _ => {
            return Err(io::Error::other(
                "--leadership-passives and --leadership-role-policy must be supplied together",
            ));
        }
    };
    let structure_path =
        structure_path.unwrap_or_else(|| item_path.with_file_name("prototype-structures.json"));
    let bunker_path =
        bunker_path.unwrap_or_else(|| item_path.with_file_name("prototype-bunkers.json"));
    let bunker_world = if application_id == Some(20231) {
        None
    } else if catalog.match_profile().has_bunkers {
        Some(mag_gameplay::structures::BunkerWorld::load(
            &bunker_path,
            &structure_path,
            catalog.map_name(),
        )?)
    } else {
        Some(mag_gameplay::structures::BunkerWorld::without_bunkers(
            &structure_path,
        )?)
    };
    let bunker_exit_policy = bunker_exit_policy_path
        .as_ref()
        .map(mag_gameplay::vehicle_exit::BunkerExitPolicy::load)
        .transpose()?;
    let bunker_info = match &bunker_world {
        Some(world) => {
            serde_json::json!({"active":world.active_count(),"unresolved":world.unresolved_count(),"placement_source":bunker_path,"combat_source":structure_path})
        }
        None => {
            serde_json::json!({"enabled":false,"reason":"retail_authority_unverified","placement_source":null,"combat_source":null})
        }
    };
    if let Some(position) = development_dummy_position {
        mag_gameplay::dummy::respawn_at_with_catalog_and_profile(
            2,
            0,
            position,
            &item_catalog,
            wire_profiles
                .profile(application_id.unwrap_or(20230))
                .map_err(io::Error::other)?,
        )
        .map_err(io::Error::other)?;
    }
    let character_damage_rules = character_damage_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--character-damage-rules requires an absolute path",
                ));
            }
            if application_id != Some(20231) {
                return Err(io::Error::other(
                    "character damage rules require the retail application profile",
                ));
            }
            let rules = mag_gameplay::character_damage_rules::Rules::load(path)?;
            rules
                .validate_match(20231, &catalog)
                .map_err(io::Error::other)?;
            Ok(rules)
        })
        .transpose()?;
    // Sabotage loads no projected launcher catalog or represented projectile
    // scene. Its stock launchers come from the item catalog and fly against the
    // loaded static collision world.
    let static_world_rockets = (infantry_rockets.is_none()
        && acquisition_setup.is_none()
        && match_profiles_path.is_none()
        && application_id == Some(20231)
        && catalog.match_profile().game_type == 0
        && collision_world.is_some())
    .then(|| {
        if character_damage_rules.is_none() {
            return Err("character_damage_rules_required");
        }
        mag_gameplay::infantry_rockets::Catalog::from_item_catalog(&item_catalog)
    });
    let static_world_rocket_info = match &static_world_rockets {
        None => serde_json::Value::Null,
        Some(Ok(rockets)) => serde_json::json!({"source":"item_catalog",
            "scene":"static_collision_world",
            "items":rockets.variants().iter().map(|v| v.item_id).collect::<Vec<_>>()}),
        Some(Err(reason)) => serde_json::json!({"enabled":false,"reason":reason}),
    };
    let bot_resupply_catalog = bot_resupply_catalog_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--bot-resupply-catalog requires an absolute path",
                ));
            }
            if bot_fill_total == 0 || bot_population_path.is_none() {
                return Err(io::Error::other(
                    "--bot-resupply-catalog requires configured bot population",
                ));
            }
            if application_id != Some(20231) || catalog.match_profile().game_type != 0 {
                return Err(io::Error::other(
                    "bot resupply requires the retail Sabotage profile",
                ));
            }
            let population = mag_gameplay::bot_registry::BotPopulation::load(
                bot_population_path.as_ref().unwrap(),
            )
            .map_err(io::Error::other)?;
            if !population.firearms.enabled || population.map_name != catalog.map_name() {
                return Err(io::Error::other(
                    "bot resupply requires current-map bot firearms",
                ));
            }
            let world = collision_world
                .as_ref()
                .ok_or_else(|| io::Error::other("bot resupply requires collision world"))?;
            let spawn_sha = catalog.source_sha256().ok_or_else(|| {
                io::Error::other("bot resupply requires source-bound spawn catalog")
            })?;
            let supply = mag_gameplay::bot_resupply_catalog::Catalog::load(path)?;
            supply
                .validate_binding(
                    20231,
                    catalog.map_name(),
                    catalog.match_profile().game_type,
                    spawn_sha,
                    world.source_sha256(),
                )
                .map_err(io::Error::other)?;
            Ok(supply)
        })
        .transpose()?;
    let bot_resupply_approach_policy = bot_resupply_approach_policy_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--bot-resupply-approach-policy requires an absolute path",
                ));
            }
            let supply = bot_resupply_catalog.as_ref().ok_or_else(|| {
                io::Error::other("--bot-resupply-approach-policy requires --bot-resupply-catalog")
            })?;
            let mesh = navmesh_world.as_ref().ok_or_else(|| {
                io::Error::other("--bot-resupply-approach-policy requires --navmesh-world")
            })?;
            let world = collision_world.as_ref().ok_or_else(|| {
                io::Error::other("--bot-resupply-approach-policy requires --collision-world")
            })?;
            let policy = mag_gameplay::bot_resupply_approach_policy::Policy::load(path)?;
            policy
                .validate_binding(supply, mesh, world)
                .map_err(io::Error::other)?;
            Ok(policy)
        })
        .transpose()?;
    let firearm_assist_rules = firearm_assist_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--firearm-assist-rules requires an absolute path",
                ));
            }
            if application_id != Some(20231) {
                return Err(io::Error::other(
                    "firearm assists require the retail application profile",
                ));
            }
            mag_gameplay::firearm_assists::Rules::load(path)
        })
        .transpose()?;
    let incapacitation_rules = incapacitation_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--incapacitation-rules requires an absolute path",
                ));
            }
            mag_gameplay::incapacitation::Rules::load(path)
        })
        .transpose()?;
    if incapacitation_rules.is_some() {
        if application_id != Some(20231) {
            return Err(io::Error::other(
                "incapacitation requires the retail application profile",
            ));
        }
        let combat = item_catalog.retail_combat().map_err(io::Error::other)?;
        for location in 0..16 {
            let value = combat
                .body_for_damage(location)
                .map_err(io::Error::other)?
                .incapacitated_multiplier
                .ok_or_else(|| {
                    io::Error::other("incapacitation requires explicit body multipliers")
                })?;
            if !(value as f32).is_finite() {
                return Err(io::Error::other(
                    "incapacitation body multiplier is not representable",
                ));
            }
        }
    }
    let fall_rules = fall_rules_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other(
                    "--fall-damage-rules requires an absolute path",
                ));
            }
            mag_gameplay::fall_damage::Rules::load(path)
        })
        .transpose()?;
    if let Some(policy) = bot_resupply_approach_policy.as_ref()
        && policy.allowed_drop_id().is_some()
    {
        let population_path = bot_population_path
            .as_ref()
            .ok_or_else(|| io::Error::other("bot resupply drop requires configured population"))?;
        let population = mag_gameplay::bot_registry::BotPopulation::load(population_path)
            .map_err(io::Error::other)?;
        policy
            .validate_drop_context(
                population.autonomous,
                population.objective_policy.is_some(),
                fall_rules.is_some(),
            )
            .map_err(io::Error::other)?;
    }
    let bot_names = bot_names_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() {
                return Err(io::Error::other("--bot-names requires an absolute path"));
            }
            mag_gameplay::bot_names::Catalog::load(path)
                .map_err(|e| io::Error::other(format!("failed to load bot names: {e}")))
        })
        .transpose()?
        .unwrap_or_default();
    let bot_names_count = bot_names.len();
    // Profile workers validate name capacity per profile (a row may bring its own names).
    if match_profiles_path.is_none() && bot_fill_total > bot_names_count {
        return Err(io::Error::other(
            "bot names have insufficient capacity for --bot-fill-total",
        ));
    }
    let actual_dummy_position = if development_dummy {
        Some(
            development_dummy_position
                .or_else(|| catalog.alternate_position())
                .ok_or_else(|| io::Error::other("missing_alternate_spawn"))?,
        )
    } else {
        None
    };
    let mut match_profiles = match_profiles_path
        .as_ref()
        .map(mag_gameplay::match_profiles::Registry::load)
        .transpose()?;
    if let (Some(registry), Some(raw)) = (&mut match_profiles, smoke_grenade_rules.take()) {
        registry
            .set_smoke_grenade_rules(Arc::from(raw))
            .map_err(io::Error::other)?;
    }
    let native_leadership = leadership
        .as_ref()
        .is_some_and(|(_, policy)| policy.supports_match_profiles());
    let squad_orders = squad_orders_path
        .as_ref()
        .map(|path| {
            if !path.is_absolute() || application_id != Some(20231) || !native_leadership {
                return Err(io::Error::other(
                    "squad_orders_require_absolute_path_and_retail_native_leadership",
                ));
            }
            mag_gameplay::squad_orders::Rules::load(path)
        })
        .transpose()?;
    let active_abilities_path = mag_gameplay::active_ability_config::manifest_path(
        active_abilities_path,
        acquisition_gameplay_path.as_deref(),
        catalog.map_name(),
    )?;
    let active_abilities = active_abilities_path
        .as_ref()
        .map(|path| {
            if !native_leadership {
                return Err(io::Error::other(
                    "active_abilities_require_native_leadership",
                ));
            }
            if acquisition_setup.is_none() {
                return Err(io::Error::other(
                    "active_abilities_require_gameplay_provider",
                ));
            }
            let world = collision_world
                .as_ref()
                .ok_or_else(|| io::Error::other("active_abilities_require_collision_world"))?;
            let objectives = objective_catalog
                .as_ref()
                .ok_or_else(|| io::Error::other("active_abilities_require_objective_catalog"))?;
            let objective_path = objective_path
                .as_ref()
                .ok_or_else(|| io::Error::other("active_abilities_require_objective_catalog"))?;
            mag_gameplay::active_ability_config::load_for_catalogs(
                path,
                &catalog,
                world,
                objectives,
                objective_path,
            )
        })
        .transpose()?;
    if leadership
        .as_ref()
        .is_some_and(|(_, policy)| policy.supports_oic_election())
        && ((catalog.match_profile().game_type == 2
            && active_abilities
                .as_ref()
                .is_some_and(|setup| !setup.has_strategic_effects()))
            || match_profiles
                .as_ref()
                .is_some_and(|profiles| profiles.has_legacy_company_abilities()))
    {
        return Err(io::Error::other(
            "oic_leadership_requires_strategic_effects",
        ));
    }
    let supports_platoon = leadership
        .as_ref()
        .is_some_and(|(_, policy)| policy.supports_platoon_election());
    let needs_platoon = active_abilities
        .as_ref()
        .is_some_and(|setup| setup.has_platoon_effects())
        || match_profiles
            .as_ref()
            .is_some_and(|profiles| profiles.has_platoon_abilities());
    if needs_platoon && !supports_platoon {
        return Err(io::Error::other(
            "active_abilities_require_native_platoon_leadership",
        ));
    }
    if supports_platoon
        && (active_abilities
            .as_ref()
            .is_some_and(|setup| !setup.has_platoon_effects())
            || match_profiles
                .as_ref()
                .is_some_and(|profiles| profiles.has_squad_only_abilities()))
    {
        return Err(io::Error::other(
            "platoon_leadership_requires_complete_active_abilities",
        ));
    }
    if let Some(registry) = &mut match_profiles {
        if leadership.is_some() && !native_leadership {
            return Err(io::Error::other(
                "match_profiles_unsupported_leadership_policy",
            ));
        }
        if registry.has_active_abilities() && !native_leadership {
            return Err(io::Error::other(
                "active_abilities_require_native_leadership",
            ));
        }
        if registry.has_acquisition_gameplay() {
            if dynamic_bodies.is_none() {
                return Err(io::Error::other(
                    "Acquisition match profiles require dynamic bodies",
                ));
            }
            registry
                .validate_acquisition_static_partitions(&static_partitions)
                .map_err(io::Error::other)?;
        }
        if registry.has_domination_gameplay() {
            if dynamic_bodies.is_none() {
                return Err(io::Error::other(
                    "Domination match profiles require dynamic bodies",
                ));
            }
            registry
                .validate_domination_static_partitions(&static_partitions)
                .map_err(io::Error::other)?;
        }
        registry
            .validate_runtime(
                &item_catalog,
                wire_profiles.profile(20231).map_err(io::Error::other)?,
            )
            .map_err(io::Error::other)?;
        registry
            .bind_hospital_turrets(&item_catalog)
            .map_err(io::Error::other)?;
        registry
            .bind_ground_turrets(&item_catalog)
            .map_err(io::Error::other)?;
        registry
            .bind_infantry_rockets(&item_catalog)
            .map_err(io::Error::other)?;
        registry
            .bind_guided_launchers(&item_catalog)
            .map_err(io::Error::other)?;
        registry
            .bind_smoke_grenades(&item_catalog)
            .map_err(io::Error::other)?;
        registry
            .bind_gas_grenades(&item_catalog)
            .map_err(io::Error::other)?;
        registry
            .bind_flash_grenades(&item_catalog)
            .map_err(io::Error::other)?;
        registry
            .bind_underbarrel_launchers(&item_catalog)
            .map_err(io::Error::other)?;
        if registry.has_flash_grenades() && dynamic_bodies.is_none() {
            return Err(io::Error::other("flash_grenades_require_dynamic_bodies"));
        }
        if registry.has_gas_grenades() && dynamic_bodies.is_none() {
            return Err(io::Error::other("gas_grenades_require_dynamic_bodies"));
        }
        if let Some((catalog, bindings)) = &dynamic_bodies {
            registry
                .validate_dynamic_bodies(catalog, bindings)
                .map_err(io::Error::other)?;
        }
        if bot_fill_total > 0 {
            registry
                .set_default_bot_fill_total(bot_fill_total)
                .map_err(io::Error::other)?;
        }
        // Each profile's bots are checked against its own world before any log opens.
        registry
            .validate_bots(&mag_gameplay::match_profiles::BotValidationInputs {
                items: &item_catalog,
                wire_profiles: &wire_profiles,
                names: &bot_names,
                fall_damage_rules: fall_rules.as_ref(),
            })
            .map_err(io::Error::other)?;
    }
    // Sabotage has no represented projectile scene; its frags dose Characters
    // against the loaded static collision world.
    let static_world_frags = (acquisition_setup.is_none()
        && match_profiles_path.is_none()
        && application_id == Some(20231)
        && catalog.match_profile().game_type == 0
        && collision_world.is_some())
    .then(|| {
        mag_gameplay::static_world_frags::qualified_items(
            &item_catalog,
            character_damage_rules.is_some(),
        )
    });
    let static_world_frag_info = match &static_world_frags {
        None => serde_json::Value::Null,
        Some(Ok(items)) => serde_json::json!({"enabled":true,
            "scene":"static_collision_world","cover":"any_static_crossing","items":items}),
        Some(Err(reason)) => serde_json::json!({"enabled":false,"reason":reason}),
    };
    let mut resources = match application_id {
        Some(id) => {
            EngineResources::for_application(item_catalog, catalog, bunker_world, wire_profiles, id)
                .map_err(io::Error::other)?
        }
        None => EngineResources::new(
            item_catalog,
            catalog,
            bunker_world.ok_or_else(|| io::Error::other("legacy bunker resources missing"))?,
        )
        .with_wire_profiles(wire_profiles),
    };
    if let Some(policy) = bunker_exit_policy {
        resources = resources
            .with_bunker_exit_policy(policy)
            .map_err(io::Error::other)?;
    }
    resources = resources.with_bot_names(bot_names);
    if validate_catalogs {
        if log.is_some() || telemetry_path.is_some() || rotation.is_some() {
            return Err(io::Error::other(
                "--validate-catalogs does not open logs; omit --log, --tick-telemetry and log rotation flags",
            ));
        }
        println!(
            "{}",
            serde_json::json!({"event":"catalogs_validated","application_id":application_id,"skill_application_id":skill_catalog.application_id(),"bunkers":bunker_info,"fall_damage_enabled":fall_rules.is_some(),"incapacitation_enabled":incapacitation_rules.is_some(),"character_damage_rules_enabled":character_damage_rules.is_some(),"firearm_assists_enabled":firearm_assist_rules.is_some(),"bot_resupply_validated":bot_resupply_catalog.is_some(),"bot_resupply_approach_validated":bot_resupply_approach_policy.is_some(),"bot_resupply_drop_validated":bot_resupply_approach_policy.as_ref().is_some_and(|policy|policy.allowed_drop_id().is_some()),"profile_bot_fill_validated":match_profiles.as_ref().is_some_and(|profiles|profiles.has_bot_fill()),"dynamic_body_catalog_validated":dynamic_bodies.is_some(),"static_partitions_validated":static_partitions.len(),"acquisition_gameplay_validated":acquisition_setup.is_some(),"acquisition_apc_crew_validated":ground_vehicles.as_ref().is_some_and(|ground|ground.has_acquisition_apc_crew()) || match_profiles.as_ref().is_some_and(|profiles|profiles.has_acquisition_apc_crew()),"squad_orders_validated":squad_orders.is_some(),"active_abilities_validated":active_abilities.is_some() || match_profiles.as_ref().is_some_and(|profiles|profiles.has_active_abilities()),"domination_gameplay_validated":match_profiles.as_ref().is_some_and(|profiles|profiles.has_domination_gameplay()),"guided_launchers_validated":guided_launchers.is_some() ||match_profiles.as_ref().is_some_and(|p|p.has_guided_launchers()),"infantry_rockets_validated":infantry_rockets.is_some() || matches!(static_world_rockets, Some(Ok(_))) || match_profiles.as_ref().is_some_and(|profiles|profiles.has_infantry_rockets()),"flash_grenades_validated":flash_grenades.is_some() || match_profiles.as_ref().is_some_and(|profiles|profiles.has_flash_grenades()),"gas_grenades_validated":gas_grenades.is_some() || match_profiles.as_ref().is_some_and(|profiles|profiles.has_gas_grenades()),"smoke_grenades_validated":smoke_grenades.is_some() || match_profiles.as_ref().is_some_and(|profiles|profiles.has_smoke_grenades()),"static_world_infantry_rockets":static_world_rocket_info,"underbarrel_launchers_validated":underbarrel_launchers.is_some() || match_profiles.as_ref().is_some_and(|profiles|profiles.has_underbarrel_launchers()),"hospital_rockets_validated":match_profiles.as_ref().is_some_and(|profiles|profiles.has_hospital_rockets()),"hospital_rocket_ground_validated":match_profiles.as_ref().is_some_and(|profiles|profiles.has_hospital_rocket_ground()),"hospital_rocket_helo_validated":match_profiles.as_ref().is_some_and(|profiles|profiles.has_hospital_rocket_helo()),"frag_character_blast_validated":acquisition_setup.as_ref().is_some_and(|setup|setup.frag_character_blast) || match_profiles.as_ref().is_some_and(|profiles|profiles.has_frag_character_blast()),"static_world_frag_blast":static_world_frag_info,"human_static_resupply_validated":acquisition_setup.as_ref().is_some_and(|setup|setup.has_human_static_resupply()) || match_profiles.as_ref().is_some_and(|profiles|profiles.has_human_static_resupply()),"frag_ground_areas_validated":acquisition_setup.as_ref().is_some_and(|setup|setup.has_frag_ground_areas()) || match_profiles.as_ref().is_some_and(|profiles|profiles.has_frag_ground_areas()),"frag_helo_areas_validated":match_profiles.as_ref().is_some_and(|profiles|profiles.has_frag_helo_areas()),"hospital_blast_areas_validated":match_profiles.as_ref().is_some_and(|profiles|profiles.has_hospital_blast_areas()),"projectile_contacts_validated":acquisition_setup.as_ref().is_some_and(|setup|setup.has_projectile_contacts()) || match_profiles.as_ref().is_some_and(|profiles|profiles.has_projectile_contacts()),"scope":"schema_and_profile_validation_not_client_acceptance"})
        );
        return Ok(());
    }
    run_worker_startup(move || {
        let path = log.ok_or_else(|| {
            io::Error::other("--log is required; existing logs are never overwritten")
        })?;
        // Rotation removes the whole-file cap; retention, not a byte ceiling, then bounds disk use.
        let max_log = if rotation.is_some() {
            u64::MAX
        } else {
            MAX_LOG
        };
        let telemetry = telemetry_path
            .as_ref()
            .map(|path| open_log(path, rotation).map(|writer| TickRecorder::start(writer, max_log)))
            .transpose()?;
        // Storage stalls must not hold the world lock; see async_log.
        let writer: LogWriter = Box::new(mag_gameplay::async_log::AsyncLogWriter::start(open_log(
            &path, rotation,
        )?));
        let mut engine =
            Engine::from_resources(writer, max_log, resources).with_skill_catalog(skill_catalog);
        if let Some(vehicles) = ground_vehicles {
            engine = engine
                .with_ground_vehicles(vehicles)
                .map_err(io::Error::other)?;
        }
        if let Some((catalog, bindings)) = dynamic_bodies {
            engine = engine
                .with_dynamic_collision_bodies(catalog, bindings)
                .map_err(io::Error::other)?;
        }
        if !static_partitions.is_empty() {
            engine = engine
                .with_acquisition_static_partitions(static_partitions)
                .map_err(io::Error::other)?;
        }
        if let Some(objectives) = objective_catalog {
            engine = engine
                .with_objective_catalog(objectives)
                .map_err(io::Error::other)?;
        }
        if let Some(rules) = character_damage_rules {
            engine = engine
                .with_character_damage_rules(rules)
                .map_err(io::Error::other)?;
        }
        if let Some(world) = collision_world {
            engine = engine.with_collision_world(world);
        }
        if matches!(static_world_frags, Some(Ok(_))) {
            engine = engine
                .with_static_world_frag_blast()
                .map_err(io::Error::other)?;
        }
        if let Some(Ok(rockets)) = static_world_rockets {
            engine = engine
                .with_static_world_infantry_rockets(rockets)
                .map_err(io::Error::other)?;
        }
        if let Some(setup) = acquisition_setup {
            engine = engine
                .with_acquisition_gameplay(setup)
                .map_err(io::Error::other)?;
        }
        if let Some(catalog) = smoke_grenades {
            engine = engine
                .with_smoke_grenades(Arc::new(catalog))
                .map_err(io::Error::other)?;
        }
        if let Some(catalog) = gas_grenades {
            engine = engine
                .with_gas_grenades(Arc::new(catalog))
                .map_err(io::Error::other)?;
        }
        if let Some(catalog) = flash_grenades {
            engine = engine
                .with_flash_grenades(Arc::new(catalog))
                .map_err(io::Error::other)?;
        }
        if let Some(catalog) = underbarrel_launchers {
            engine = engine
                .with_underbarrel_launchers(Arc::new(catalog))
                .map_err(io::Error::other)?;
        }
        if let Some(catalog) = guided_launchers {
            engine = engine
                .with_guided_launchers(catalog)
                .map_err(io::Error::other)?;
        }
        if let Some(catalog) = infantry_rockets {
            engine = engine
                .with_infantry_rockets(catalog)
                .map_err(io::Error::other)?;
        }
        if let Some(rules) = firearm_assist_rules {
            engine = engine
                .with_firearm_assist_rules(rules)
                .map_err(io::Error::other)?;
        }
        if let Some(rules) = incapacitation_rules {
            engine = engine
                .with_incapacitation_rules(rules)
                .map_err(io::Error::other)?;
        }
        if let Some(rules) = fall_rules {
            engine = engine.with_fall_damage_rules(rules);
        }
        if let Some(mesh) = navmesh_world {
            engine = engine.with_navmesh_world(mesh);
        }
        if development_control {
            engine = engine.with_development_control();
        }
        if development_grenade_radius {
            engine = engine.with_development_grenade_radius();
        }
        if development_dummy {
            engine = engine.with_development_dummy();
        }
        if let Some(position) = development_dummy_position {
            engine = engine
                .with_development_dummy_position(position)
                .map_err(io::Error::other)?;
        }
        if let Some((catalog, policy)) = leadership {
            engine = engine
                .with_leadership_passives(catalog, policy)
                .map_err(io::Error::other)?;
        }
        if let Some(setup) = active_abilities {
            engine = engine
                .with_active_abilities(setup)
                .map_err(io::Error::other)?;
        }
        if let Some(rules) = squad_orders {
            engine = engine.with_squad_orders(rules).map_err(io::Error::other)?;
        }
        if bot_fill_total > 0 && match_profiles_path.is_none() {
            let path = bot_population_path
                .as_ref()
                .ok_or_else(|| io::Error::other("--bot-fill-total requires --bot-population"))?;
            let population = mag_gameplay::bot_registry::BotPopulation::load(path)
                .map_err(|e| io::Error::other(e.to_string()))?;
            engine = engine
                .with_bot_population(population, bot_fill_total)
                .map_err(io::Error::other)?;
        }
        if let Some(supply) = bot_resupply_catalog {
            engine
                .configure_bot_resupply_catalog(supply)
                .map_err(io::Error::other)?;
        }
        if let Some(policy) = bot_resupply_approach_policy {
            engine
                .configure_bot_resupply_approach_policy(policy)
                .map_err(io::Error::other)?;
        }
        if let Some(registry) = match_profiles {
            engine = engine
                .with_match_profiles(registry)
                .map_err(io::Error::other)?;
        }
        if party_reservations {
            engine
                .configure_party_reservations()
                .map_err(io::Error::other)?;
        }
        engine.set_bot_perception_override(bot_perception);
        if let Some(seconds) = headless_sim_seconds {
            // LOCAL DIAGNOSTIC: a virtual-time match with never-spawning observers.
            let options =
                mag_gameplay::headless_sim::HeadlessSimOptions::from_lookup(seconds, |name| {
                    std::env::var(name).ok()
                })
                .map_err(io::Error::other)?;
            let mut ticks = std::env::var_os("MAG_SIM_TICKS")
                .map(|path| std::fs::File::create(path).map(io::BufWriter::new))
                .transpose()?;
            let wall0 = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let began = Instant::now();
            let report = engine
                .run_headless_sim(
                    &options,
                    wall0,
                    ticks.as_mut().map(|out| out as &mut dyn Write),
                )
                .map_err(io::Error::other)?;
            if let Some(out) = ticks.as_mut() {
                out.flush()?;
            }
            for error in &report.first_tick_errors {
                eprintln!("tick error {error}");
            }
            let mut summary = serde_json::to_value(&report).map_err(io::Error::other)?;
            summary["event"] = "headless_sim_summary".into();
            eprintln!("{summary}");
            eprintln!(
                "{}",
                serde_json::json!({"event":"headless_sim_done","simulated_s":seconds,"wall_ms":began.elapsed().as_millis(),"tick_errors":report.tick_errors})
            );
            return Ok(());
        }
        // Served workers wait for Horizon's configure_match before admission.
        engine.require_match_configuration();
        let engine = Arc::new(Mutex::new(engine));
        let listener = TcpListener::bind(bind)?;
        let started = Instant::now();
        let active = Arc::new(AtomicUsize::new(0));
        let subscriber: SharedSubscriber = Arc::new(Mutex::new(None));
        let journal = Arc::new(Mutex::new(Journal::new(world_stream::new_epoch()?)));
        spawn_world_scheduler(
            Arc::clone(&engine),
            Arc::clone(&subscriber),
            Arc::clone(&journal),
            started,
            telemetry.clone(),
            tick_budget_us,
        );
        println!(
            "{}",
            serde_json::json!({"event":"listening","application_id":application_id.unwrap_or(20230),"match_profiles":match_profiles_path,"active_abilities":active_abilities_path,"squad_orders":squad_orders_path,"profile_selection":if application_id.is_some(){"explicit"}else{"legacy"},"protocol":mag_gameplay::BRIDGE_VERSION,"address":listener.local_addr()?.to_string(),"collision":collision_info,"spawn_catalog":spawn_path,"objective_catalog":objective_info,"item_catalog":item_path,"character_blast_rules":blast_rules_path,"retail_combat_metadata":retail_metadata,"bootstrap_loadouts":bootstrap_loadouts_path,"fall_damage_rules":fall_rules_path,"incapacitation_rules":incapacitation_rules_path,"character_damage_rules":character_damage_rules_path,"firearm_assist_rules":firearm_assist_rules_path,"skill_catalog":skill_path,"retail_armory_catalog":retail_armory_path,"smoke_grenade_rules":smoke_grenade_rules_path,"leadership_passives":leadership_passives_path,"leadership_role_policy":leadership_role_policy_path,"bunkers":bunker_info,"tick_telemetry":telemetry_path,"tick_budget_us":tick_budget_us,"scheduler":"independent_50ms_when_subscribed","bot_fill_total":bot_fill_total,"initial_spawn_delay_ms":effective_initial_spawn_delay_ms,"initial_spawn_delay_source":if initial_spawn_delay_ms.is_some(){"override"}else{"catalog"},"bot_population":bot_population_path,"bot_resupply_catalog":bot_resupply_catalog_path,"bot_resupply_approach_policy":bot_resupply_approach_policy_path,"bot_names":bot_names_path,"bot_names_count":bot_names_count,"development_dummy_position":actual_dummy_position,"infantry_rockets":static_world_rocket_info,"static_world_frag_blast":static_world_frag_info})
        );
        let context = Arc::new(ConnectionContext {
            engine,
            started,
            telemetry,
            tick_budget_us,
            subscriber,
            worker_identity,
            journal,
            event_replies: Mutex::new(EventReplies::default()),
        });
        for connection in listener.incoming() {
            let stream = connection?;
            if active
                .try_update(Ordering::AcqRel, Ordering::Acquire, |n| {
                    (n < 8).then_some(n + 1)
                })
                .is_err()
            {
                drop(stream);
                continue;
            }
            let context = Arc::clone(&context);
            let active = Arc::clone(&active);
            std::thread::spawn(move || {
                let slot = ConnectionSlot(Some(active));
                if let Err(failure) = serve(stream, &context, slot) {
                    // Fixed categories only: no payload, request ID, credentials or
                    // operating-system error text can reach this diagnostic.
                    let _ = writeln!(io::stderr().lock(), "{}", failure.diagnostic());
                }
            });
        }
        Ok(())
    })
}
#[derive(Clone, Copy, serde::Serialize)]
#[serde(rename_all = "snake_case")]
enum ConnectionStage {
    ReadRequest,
    EncodeResponse,
    WriteResponse,
}

struct ConnectionFailure {
    stage: ConnectionStage,
    kind: io::ErrorKind,
}
impl ConnectionFailure {
    fn io(stage: ConnectionStage, error: io::Error) -> Self {
        Self {
            stage,
            kind: error.kind(),
        }
    }
    fn diagnostic(&self) -> serde_json::Value {
        let category = match (self.stage, self.kind) {
            (ConnectionStage::EncodeResponse, _) => "encoding_failed",
            (_, io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => "timeout",
            (_, io::ErrorKind::InvalidData) => "invalid_framing",
            (_, io::ErrorKind::UnexpectedEof) => "incomplete_request",
            (
                _,
                io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::ConnectionAborted
                | io::ErrorKind::NotConnected,
            ) => "disconnected",
            _ => "io_error",
        };
        serde_json::json!({"event":"connection_error", "stage":self.stage, "category":category})
    }
}

struct ConnectionContext {
    engine: Arc<Mutex<Engine<LogWriter>>>,
    started: Instant,
    telemetry: Option<TickRecorder>,
    tick_budget_us: Option<u64>,
    subscriber: SharedSubscriber,
    worker_identity: Option<WorkerIdentity>,
    journal: Arc<Mutex<Journal>>,
    event_replies: Mutex<EventReplies>,
}

fn serve(
    mut stream: TcpStream,
    context: &ConnectionContext,
    slot: ConnectionSlot,
) -> Result<(), ConnectionFailure> {
    let ConnectionContext {
        engine,
        started,
        telemetry,
        tick_budget_us,
        subscriber,
        worker_identity,
        journal,
        event_replies,
    } = context;
    let worker_identity = worker_identity.as_ref();
    let line = read_line(&mut stream, IO_DEADLINE)
        .map_err(|error| ConnectionFailure::io(ConnectionStage::ReadRequest, error))?;
    let (line, fenced) = match worker_identity {
        Some(identity) => match identity.strip_request_envelope(&line) {
            Ok(envelope) => envelope,
            Err(error) => {
                return write_response(
                    &mut stream,
                    Response::error(error.request_id, error.code),
                    worker_identity,
                );
            }
        },
        None => (line, false),
    };
    if serde_json::from_slice::<serde_json::Value>(&line)
        .ok()
        .is_some_and(|v| v["op"] == "subscribe_world")
    {
        return serve_subscription(
            stream,
            &line,
            engine,
            Arc::clone(subscriber),
            journal,
            slot,
            worker_identity,
        );
    }
    let parsed = parse_request(&line);
    let hello = matches!(parsed, Ok(Request::Hello { .. }));
    let mut response = match parsed {
        Ok(Request::Hello { request_id, .. }) if worker_identity.is_some() => match engine.lock() {
            Ok(engine) => engine.describe_worker(request_id, fenced),
            Err(_) => Response::error(request_id, "engine_unavailable"),
        },
        Ok(request) => {
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let update = telemetry
                .as_ref()
                .is_some_and(|_| request.is_gameplay_update());
            let waiting = Instant::now();
            let started_us = started.elapsed().as_micros() as u64;
            let (response, wait_us, engine_us) = match engine.lock() {
                Ok(mut engine) => {
                    let wait_us = waiting.elapsed().as_micros() as u64;
                    let processing = Instant::now();
                    let event_id = match &request {
                        Request::Event { request_id, .. } => Some(request_id.clone()),
                        _ => None,
                    };
                    let response = handle_once(event_replies, event_id, || {
                        engine.handle(request, started.elapsed(), wall)
                    });
                    (response, wait_us, processing.elapsed().as_micros() as u64)
                }
                Err(_) => (
                    Response::error(String::new(), "engine_unavailable"),
                    waiting.elapsed().as_micros() as u64,
                    0,
                ),
            };
            // The engine lock has been released before telemetry enqueue.
            if let Some(recorder) = telemetry.as_ref() {
                recorder.record(TickSample::new(
                    update,
                    started_us,
                    wait_us,
                    engine_us,
                    response.ok,
                    *tick_budget_us,
                ));
            }
            response
        }
        Err(error) => Response::error(error.request_id, error.code),
    };
    if hello && response.ok {
        // Served-worker bridge features; in-process engines do not offer them.
        // Horizon accepts at most 32 capabilities; a full discovery reply with
        // these two and worker_registry_v1 lists 16.
        response.capabilities.extend([
            world_stream::CAPABILITY.to_owned(),
            world_stream::EVENT_REPLAY_CAPABILITY.to_owned(),
        ]);
    }
    write_response(&mut stream, response, worker_identity)
}

/// Apply a request; an event whose ID was already applied is answered with its
/// original reply instead. Horizon retries an unanswered event with the same
/// ID, and the engine lock serialises the check with the application.
fn handle_once(
    replies: &Mutex<EventReplies>,
    event_id: Option<String>,
    apply: impl FnOnce() -> Response,
) -> Response {
    if let Some(id) = &event_id
        && let Some(reply) = replies.lock().ok().and_then(|replies| replies.get(id))
    {
        return reply;
    }
    let response = apply();
    if let Some(id) = event_id
        && let Ok(mut replies) = replies.lock()
    {
        replies.insert(id, response.clone());
    }
    response
}

fn parse_point(value: &str) -> io::Result<[f64; 3]> {
    let parts: Vec<_> = value.split(',').collect();
    if parts.len() != 3 {
        return Err(io::Error::other("point requires x,y,z"));
    }
    let mut result = [0.0_f64; 3];
    for (out, part) in result.iter_mut().zip(parts) {
        *out = part
            .parse()
            .map_err(|_| io::Error::other("invalid point coordinate"))?;
        if !out.is_finite() {
            return Err(io::Error::other("point must be finite"));
        }
    }
    Ok(result)
}

struct ConnectionSlot(Option<Arc<AtomicUsize>>);
impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        if let Some(active) = self.0.take() {
            active.fetch_sub(1, Ordering::AcqRel);
        }
    }
}
struct WorldSubscriber {
    generation: u64,
    world_id: u32,
    request_id: String,
    sender: SyncSender<Response>,
    /// The subscriber asked for frame positions (world_stream_resume_v1).
    positions: bool,
}
type SharedSubscriber = Arc<Mutex<Option<WorldSubscriber>>>;

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct SubscribeWorld {
    op: String,
    v: u8,
    request_id: String,
    world_id: u32,
    #[serde(default)]
    positions: bool,
    #[serde(default)]
    resume: Option<ResumeFrom>,
}
fn write_response(
    stream: &mut TcpStream,
    mut response: Response,
    worker_identity: Option<&WorkerIdentity>,
) -> Result<(), ConnectionFailure> {
    response.worker_identity = worker_identity.cloned();
    let mut bytes = serde_json::to_vec(&response).map_err(|_| ConnectionFailure {
        stage: ConnectionStage::EncodeResponse,
        kind: io::ErrorKind::InvalidData,
    })?;
    bytes.push(b'\n');
    stream
        .set_write_timeout(Some(IO_DEADLINE))
        .and_then(|_| stream.write_all(&bytes))
        .map_err(|e| ConnectionFailure::io(ConnectionStage::WriteResponse, e))
}
fn serve_subscription(
    mut stream: TcpStream,
    line: &[u8],
    engine: &Mutex<Engine<LogWriter>>,
    shared: SharedSubscriber,
    journal: &Mutex<Journal>,
    slot: ConnectionSlot,
    worker_identity: Option<&WorkerIdentity>,
) -> Result<(), ConnectionFailure> {
    let request = serde_json::from_slice::<SubscribeWorld>(line)
        .ok()
        .filter(|r| {
            r.op == "subscribe_world"
                && r.v == mag_gameplay::BRIDGE_VERSION
                && (1..=64).contains(&r.request_id.len())
                && r.request_id
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
        });
    let Some(request) = request else {
        return write_response(
            &mut stream,
            Response::error(String::new(), "invalid_subscription"),
            worker_identity,
        );
    };
    static NEXT_GENERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    let generation = NEXT_GENERATION.fetch_add(1, Ordering::Relaxed);
    let receiver = {
        let Some(mut guard) = vacant_subscriber_slot(&shared)? else {
            return write_response(
                &mut stream,
                Response::error(request.request_id, "world_subscription_exists"),
                worker_identity,
            );
        };
        let mut engine = engine.lock().map_err(|_| ConnectionFailure {
            stage: ConnectionStage::WriteResponse,
            kind: io::ErrorKind::Other,
        })?;
        if !engine.accepts_subscription_world(request.world_id) {
            drop(engine);
            drop(guard);
            return write_response(
                &mut stream,
                Response::error(request.request_id, "subscription_world_mismatch"),
                worker_identity,
            );
        }
        let receiver = match register_subscriber(&mut guard, journal, request, generation) {
            Ok(receiver) => receiver,
            Err((request_id, code)) => {
                drop(engine);
                drop(guard);
                return write_response(
                    &mut stream,
                    Response::error(request_id, code),
                    worker_identity,
                );
            }
        };
        engine.enable_world_ticks();
        receiver
    };
    // Persistent subscriptions have their own single slot, separate from request workers.
    drop(slot);
    forward_subscription(
        stream,
        shared,
        generation,
        receiver,
        worker_identity,
        WORLD_FRAME_DEADLINE,
    )
}

/// Fill the vacant subscriber slot. The acknowledgement and any replay are
/// queued before the scheduler can see the subscriber, and the scheduler
/// appends and delivers each frame under the subscriber lock the caller holds,
/// so every frame after the resume point is either replayed or sent live.
fn register_subscriber(
    slot: &mut Option<WorldSubscriber>,
    journal: &Mutex<Journal>,
    request: SubscribeWorld,
    generation: u64,
) -> Result<mpsc::Receiver<Response>, (String, &'static str)> {
    let positions = request.positions || request.resume.is_some();
    let (acknowledged, replay) = {
        let Ok(journal) = journal.lock() else {
            return Err((request.request_id, "engine_unavailable"));
        };
        match &request.resume {
            Some(from) => match journal.replay(from) {
                Ok(frames) => (
                    Some(world_stream::Position {
                        epoch: from.epoch.clone(),
                        sequence: from.after,
                    }),
                    frames,
                ),
                Err(code) => return Err((request.request_id, code)),
            },
            None => (positions.then(|| journal.position()), Vec::new()),
        }
    };
    let (sender, receiver) = mpsc::sync_channel(WORLD_FRAME_QUEUE + replay.len());
    // The acknowledgement names the position the next frame follows.
    let mut acknowledgement = Response::success(request.request_id.clone());
    acknowledgement.world_stream = acknowledged;
    sender.send(acknowledgement).unwrap();
    for mut frame in replay {
        frame.request_id = request.request_id.clone();
        sender.send(frame).unwrap();
    }
    *slot = Some(WorldSubscriber {
        generation,
        world_id: request.world_id,
        request_id: request.request_id,
        sender,
        positions,
    });
    Ok(receiver)
}

// Keep the occupancy check and registration under the same lock: a new
// connection can never replace a live subscription, even during reconnects.
fn vacant_subscriber_slot(
    shared: &SharedSubscriber,
) -> Result<Option<std::sync::MutexGuard<'_, Option<WorldSubscriber>>>, ConnectionFailure> {
    let guard = shared.lock().map_err(|_| ConnectionFailure {
        stage: ConnectionStage::WriteResponse,
        kind: io::ErrorKind::Other,
    })?;
    Ok(if guard.is_none() { Some(guard) } else { None })
}

/// Longest silence a subscription tolerates before ending itself. The scheduler
/// sends a heartbeat every second; this bound only detects a worker that has
/// stopped producing frames. A brief world-lock or storage stall must not end
/// the stream, because Horizon releases every player on a stream gap.
const WORLD_FRAME_DEADLINE: Duration = Duration::from_secs(10);
/// Frames buffered for a subscriber that reads slowly, for example while
/// Horizon commits a reward to durable storage. A full queue ends the stream
/// and releases every player, so hold about as long as the frame deadline
/// (20 frames per second) rather than the former 1.6 seconds.
const WORLD_FRAME_QUEUE: usize = 256;

fn forward_subscription(
    mut stream: TcpStream,
    shared: SharedSubscriber,
    generation: u64,
    receiver: mpsc::Receiver<Response>,
    worker_identity: Option<&WorkerIdentity>,
    frame_timeout: Duration,
) -> Result<(), ConnectionFailure> {
    let _owner = SubscriptionOwner {
        shared: Arc::clone(&shared),
        generation,
    };
    // A peer can send FIN while its read side continues accepting every
    // heartbeat. Poll the read side too; successful writes cannot detect that.
    // The subscription owns both TCP directions: read-side EOF ends ownership,
    // including a peer half-close that keeps consuming responses.
    stream
        .set_read_timeout(Some(Duration::from_millis(1)))
        .map_err(|error| ConnectionFailure::io(ConnectionStage::ReadRequest, error))?;
    let mut frame_deadline = Instant::now() + frame_timeout;
    loop {
        if shared
            .lock()
            .ok()
            .is_none_or(|guard| !guard.as_ref().is_some_and(|s| s.generation == generation))
        {
            return Err(ConnectionFailure {
                stage: ConnectionStage::WriteResponse,
                kind: io::ErrorKind::ConnectionAborted,
            });
        }
        let mut incoming = [0u8; 1];
        match stream.read(&mut incoming) {
            Ok(0) => {
                return Err(ConnectionFailure {
                    stage: ConnectionStage::ReadRequest,
                    kind: io::ErrorKind::ConnectionAborted,
                });
            }
            Ok(_) => {
                return Err(ConnectionFailure {
                    stage: ConnectionStage::ReadRequest,
                    kind: io::ErrorKind::InvalidData,
                });
            }
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                        | io::ErrorKind::Interrupted
                ) => {}
            Err(error) => return Err(ConnectionFailure::io(ConnectionStage::ReadRequest, error)),
        }
        let remaining = frame_deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(ConnectionFailure {
                stage: ConnectionStage::WriteResponse,
                kind: io::ErrorKind::ConnectionAborted,
            });
        }
        match receiver.recv_timeout(remaining.min(Duration::from_millis(100))) {
            Ok(response) => {
                write_response(&mut stream, response, worker_identity)?;
                frame_deadline = Instant::now() + frame_timeout;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                return Err(ConnectionFailure {
                    stage: ConnectionStage::WriteResponse,
                    kind: io::ErrorKind::ConnectionAborted,
                });
            }
        }
    }
}

struct SubscriptionOwner {
    shared: SharedSubscriber,
    generation: u64,
}
impl Drop for SubscriptionOwner {
    fn drop(&mut self) {
        if let Ok(mut guard) = self.shared.lock()
            && guard
                .as_ref()
                .is_some_and(|s| s.generation == self.generation)
        {
            *guard = None;
        }
    }
}

fn spawn_world_scheduler(
    engine: Arc<Mutex<Engine<LogWriter>>>,
    subscriber: SharedSubscriber,
    journal: Arc<Mutex<Journal>>,
    started: Instant,
    telemetry: Option<TickRecorder>,
    budget: Option<u64>,
) {
    std::thread::spawn(move || {
        let interval = Duration::from_millis(50);
        let mut deadline = Instant::now() + interval;
        let mut heartbeat = Instant::now();
        loop {
            std::thread::sleep(deadline.saturating_duration_since(Instant::now()));
            let now = Instant::now();
            // Skip missed deadlines: never run an unbounded burst of catch-up steps.
            deadline += interval;
            if deadline <= now {
                deadline = now + interval;
            }
            let target = subscriber.lock().ok().and_then(|s| {
                s.as_ref()
                    .map(|s| (s.generation, s.world_id, s.request_id.clone()))
            });
            let Some((generation, world_id, request_id)) = target else {
                continue;
            };
            let wall = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let waiting = Instant::now();
            let start_us = started.elapsed().as_micros() as u64;
            let (response, wait_us, engine_us) = match engine.lock() {
                Ok(mut engine) => {
                    let wait = waiting.elapsed().as_micros() as u64;
                    let processing = Instant::now();
                    let response = engine.tick_world(started.elapsed(), wall, request_id);
                    (response, wait, processing.elapsed().as_micros() as u64)
                }
                Err(_) => continue,
            };
            if let Some(recorder) = telemetry.as_ref() {
                recorder.record(
                    TickSample::new(true, start_us, wait_us, engine_us, response.ok, budget)
                        .independent(),
                );
            }
            let mismatch = response.messages.iter().any(|m| m.world_id != world_id);
            if mismatch || !response.ok {
                if let Ok(mut guard) = subscriber.lock()
                    && guard.as_ref().is_some_and(|s| s.generation == generation)
                {
                    *guard = None;
                }
                continue;
            }
            let due = heartbeat.elapsed() >= Duration::from_secs(1);
            if response.messages.is_empty()
                && response.rewards.is_empty()
                && response.match_terminal.is_none()
                && !due
            {
                continue;
            }
            heartbeat = Instant::now();
            publish_frame(&subscriber, &journal, response);
        }
    });
}

/// Number a produced frame and deliver it to the current subscriber. A frame
/// whose subscriber ended while the world ticked stays in the journal for the
/// next subscription to resume from, instead of being lost with the old queue.
fn publish_frame(subscriber: &SharedSubscriber, journal: &Mutex<Journal>, frame: Response) {
    let Ok(mut guard) = subscriber.lock() else {
        return;
    };
    let mut frame = match journal.lock() {
        Ok(mut journal) => journal.append(frame),
        Err(_) => frame,
    };
    let Some(current) = guard.as_ref() else {
        return;
    };
    frame.request_id = current.request_id.clone();
    if !current.positions {
        frame.world_stream = None;
    }
    if let Err(TrySendError::Full(_) | TrySendError::Disconnected(_)) =
        current.sender.try_send(frame)
    {
        // Closing the stream is an explicit gap; Horizon resumes or releases.
        *guard = None;
    }
}

#[cfg(test)]
mod subscription_tests {
    use super::*;
    use std::net::{Ipv4Addr, Shutdown};
    use std::sync::atomic::AtomicBool;

    fn response() -> Response {
        Response::success("synthetic-world-stream".into())
    }

    fn sockets() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
        let client =
            TcpStream::connect_timeout(&listener.local_addr().unwrap(), Duration::from_secs(1))
                .unwrap();
        let (server, _) = listener.accept().unwrap();
        (client, server)
    }

    fn register(
        shared: &SharedSubscriber,
        generation: u64,
    ) -> (SyncSender<Response>, mpsc::Receiver<Response>) {
        let (sender, receiver) = mpsc::sync_channel(32);
        let mut guard = vacant_subscriber_slot(shared)
            .ok()
            .flatten()
            .expect("vacant subscriber");
        *guard = Some(WorldSubscriber {
            generation,
            world_id: 1,
            request_id: "synthetic-world-stream".into(),
            sender: sender.clone(),
            positions: false,
        });
        (sender, receiver)
    }

    #[test]
    fn peer_fin_releases_subscription_while_heartbeats_keep_writing() {
        let (client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (sender, receiver) = register(&shared, 1);
        assert!(
            vacant_subscriber_slot(&shared).ok().flatten().is_none(),
            "a live owner must reject another registration"
        );
        let running = Arc::new(AtomicBool::new(true));
        let pulse = Arc::clone(&running);
        let heartbeat = std::thread::spawn(move || {
            while pulse.load(Ordering::Acquire) {
                if sender.try_send(response()).is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || {
            forward_subscription(
                server,
                worker_shared,
                1,
                receiver,
                None,
                WORLD_FRAME_DEADLINE,
            )
        });
        // FIN leaves the peer's read side open: all tiny heartbeats can still
        // write successfully. Only checking writes/queue timeouts misses it.
        client.shutdown(Shutdown::Write).unwrap();
        let deadline = Instant::now() + Duration::from_millis(500);
        while shared.lock().unwrap().is_some() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        let released = shared.lock().unwrap().is_none();
        running.store(false, Ordering::Release);
        *shared.lock().unwrap() = None;
        heartbeat.join().unwrap();
        let _ = worker.join().unwrap();
        assert!(
            released,
            "peer FIN must release the owner despite successful heartbeat writes"
        );

        // Exercise a replacement through the same atomic slot reservation and
        // forwarding loop: its first acknowledgement must reach the new peer.
        let (mut replacement, server) = sockets();
        let (sender, receiver) = register(&shared, 2);
        sender.send(response()).unwrap();
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || {
            forward_subscription(
                server,
                worker_shared,
                2,
                receiver,
                None,
                WORLD_FRAME_DEADLINE,
            )
        });
        let line = mag_gameplay::read_line(&mut replacement, Duration::from_secs(1)).unwrap();
        let reply: Response = serde_json::from_slice(&line).unwrap();
        assert!(reply.ok);
        assert_eq!(reply.request_id, "synthetic-world-stream");
        replacement.shutdown(Shutdown::Write).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(shared.lock().unwrap().is_none());
    }

    fn subscribe(
        shared: &SharedSubscriber,
        journal: &Mutex<Journal>,
        request_id: &str,
        positions: bool,
        resume: Option<ResumeFrom>,
        generation: u64,
    ) -> Result<mpsc::Receiver<Response>, (String, &'static str)> {
        let mut guard = vacant_subscriber_slot(shared)
            .ok()
            .flatten()
            .expect("vacant subscriber");
        let request = SubscribeWorld {
            op: "subscribe_world".into(),
            v: mag_gameplay::BRIDGE_VERSION,
            request_id: request_id.into(),
            world_id: 1,
            positions,
            resume,
        };
        register_subscriber(&mut guard, journal, request, generation)
    }

    fn frame(payload: &str) -> Response {
        let mut frame = Response::success("synthetic-tick".into());
        frame.messages.push(mag_gameplay::OutboundMessage {
            reservation_id: "synthetic-reservation-0001".into(),
            world_id: 1,
            player_id: 1,
            plugin_id: 1,
            payload_hex: payload.into(),
        });
        frame
    }

    fn position(response: &Response) -> u64 {
        response.world_stream.as_ref().expect("position").sequence
    }

    #[test]
    fn resumed_subscriber_receives_the_frames_its_broken_stream_lost() {
        let shared: SharedSubscriber = Arc::new(Mutex::new(None));
        let journal = Mutex::new(Journal::new("synthetic-epoch".into()));
        let first = subscribe(&shared, &journal, "synthetic-first", true, None, 1).unwrap();
        let ack = first.recv().unwrap();
        assert_eq!(position(&ack), 0);
        assert_eq!(ack.world_stream.as_ref().unwrap().epoch, "synthetic-epoch");
        for payload in ["01", "02", "03"] {
            publish_frame(&shared, &journal, frame(payload));
        }
        let delivered: Vec<_> = (0..3).map(|_| first.recv().unwrap()).collect();
        assert_eq!(
            delivered.iter().map(position).collect::<Vec<_>>(),
            [1, 2, 3]
        );
        assert!(delivered.iter().all(|f| f.request_id == "synthetic-first"));

        // The stream breaks: Horizon handled frames through 2, frame 3 was in
        // flight, and a tick that began before the break produces 4 and 5.
        drop(first);
        publish_frame(&shared, &journal, frame("04"));
        assert!(
            shared.lock().unwrap().is_none(),
            "a dead queue keeps its owner"
        );
        publish_frame(&shared, &journal, frame("05"));

        let resume = |epoch: &str, after| {
            Some(ResumeFrom {
                epoch: epoch.into(),
                after,
            })
        };
        assert_eq!(
            subscribe(
                &shared,
                &journal,
                "synthetic-restart",
                true,
                resume("synthetic-other", 2),
                2
            )
            .err(),
            Some(("synthetic-restart".into(), world_stream::RESUME_UNAVAILABLE))
        );
        assert!(
            shared.lock().unwrap().is_none(),
            "a refused resume took the slot"
        );

        let second = subscribe(
            &shared,
            &journal,
            "synthetic-second",
            true,
            resume("synthetic-epoch", 2),
            3,
        )
        .unwrap();
        let ack = second.recv().unwrap();
        assert!(ack.ok);
        assert_eq!(position(&ack), 2);
        publish_frame(&shared, &journal, frame("06"));
        let resumed: Vec<_> = (0..4).map(|_| second.recv().unwrap()).collect();
        assert_eq!(
            resumed.iter().map(position).collect::<Vec<_>>(),
            [3, 4, 5, 6]
        );
        assert_eq!(
            resumed
                .iter()
                .map(|f| f.messages[0].payload_hex.as_str())
                .collect::<Vec<_>>(),
            ["03", "04", "05", "06"]
        );
        assert!(resumed.iter().all(|f| f.request_id == "synthetic-second"));
    }

    #[test]
    fn subscribers_without_positions_keep_the_previous_frames() {
        let shared: SharedSubscriber = Arc::new(Mutex::new(None));
        let journal = Mutex::new(Journal::new("synthetic-epoch".into()));
        let legacy = subscribe(&shared, &journal, "synthetic-legacy", false, None, 1).unwrap();
        assert!(legacy.recv().unwrap().world_stream.is_none());
        publish_frame(&shared, &journal, frame("01"));
        let delivered = legacy.recv().unwrap();
        assert!(delivered.world_stream.is_none());
        assert_eq!(delivered.request_id, "synthetic-legacy");
        let encoded = serde_json::to_value(&delivered).unwrap();
        assert!(encoded.get("world_stream").is_none());
    }

    #[test]
    fn a_retried_event_is_applied_once_and_answered_alike() {
        let replies = Mutex::new(EventReplies::default());
        let mut applied = 0;
        let mut apply = |payload: &str| {
            handle_once(&replies, Some("synthetic-event".into()), || {
                applied += 1;
                frame(payload)
            })
        };
        let first = apply("01");
        let retry = apply("02");
        assert_eq!(first.messages[0].payload_hex, "01");
        assert_eq!(retry.messages[0].payload_hex, "01");
        let other = handle_once(&replies, Some("synthetic-other".into()), || frame("03"));
        assert_eq!(other.messages[0].payload_hex, "03");
        let mut controls = 0;
        for _ in 0..2 {
            handle_once(&replies, None, || {
                controls += 1;
                frame("04")
            });
        }
        assert_eq!(controls, 2, "only events are deduplicated");
        drop(apply);
        assert_eq!(applied, 1);
    }

    #[test]
    fn idle_read_timeout_keeps_the_active_subscription_exclusive() {
        let (client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (_sender, receiver) = register(&shared, 1);
        let worker_shared = Arc::clone(&shared);
        let worker = std::thread::spawn(move || {
            forward_subscription(
                server,
                worker_shared,
                1,
                receiver,
                None,
                WORLD_FRAME_DEADLINE,
            )
        });
        std::thread::sleep(Duration::from_millis(250));
        assert!(vacant_subscriber_slot(&shared).ok().flatten().is_none());
        client.shutdown(Shutdown::Write).unwrap();
        assert!(worker.join().unwrap().is_err());
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn missing_scheduler_frames_still_expire_the_subscription() {
        let (_client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (_sender, receiver) = register(&shared, 1);
        let began = Instant::now();
        let timeout = Duration::from_millis(400);
        let failure = forward_subscription(server, Arc::clone(&shared), 1, receiver, None, timeout)
            .err()
            .unwrap();
        assert!(began.elapsed() >= timeout);
        assert!(began.elapsed() < timeout + Duration::from_secs(1));
        assert_eq!(failure.kind, io::ErrorKind::ConnectionAborted);
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn a_stall_shorter_than_the_frame_deadline_keeps_the_subscription() {
        // Live engine stalls of two to four seconds ended the stream and
        // released every player; the deadline must outlast such a stall.
        assert!(WORLD_FRAME_DEADLINE >= Duration::from_secs(10));
        // A reader that pauses for a durable reward commit keeps its stream:
        // ten seconds of 50 ms frames fit the queue.
        const { assert!(WORLD_FRAME_QUEUE >= 200) };
        let (mut client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (sender, receiver) = register(&shared, 1);
        let worker_shared = Arc::clone(&shared);
        let timeout = Duration::from_millis(900);
        let worker = std::thread::spawn(move || {
            forward_subscription(server, worker_shared, 1, receiver, None, timeout)
        });
        // Frames arrive with gaps shorter than the deadline.
        for _ in 0..4 {
            std::thread::sleep(Duration::from_millis(500));
            sender.send(response()).unwrap();
            let line = mag_gameplay::read_line(&mut client, Duration::from_secs(1)).unwrap();
            assert!(serde_json::from_slice::<Response>(&line).unwrap().ok);
        }
        assert!(
            shared.lock().unwrap().is_some(),
            "a short stall ended the stream"
        );
        client.shutdown(Shutdown::Write).unwrap();
        assert!(worker.join().unwrap().is_err());
    }

    #[test]
    fn unexpected_subscription_input_releases_its_owner() {
        let (mut client, server) = sockets();
        let shared = Arc::new(Mutex::new(None));
        let (_sender, receiver) = register(&shared, 1);
        client.write_all(b"x").unwrap();
        let failure = forward_subscription(
            server,
            Arc::clone(&shared),
            1,
            receiver,
            None,
            WORLD_FRAME_DEADLINE,
        )
        .err()
        .unwrap();
        assert_eq!(failure.kind, io::ErrorKind::InvalidData);
        assert_eq!(failure.diagnostic()["category"], "invalid_framing");
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn unwinding_subscription_owner_releases_the_slot() {
        let shared = Arc::new(Mutex::new(None));
        let (_sender, _receiver) = register(&shared, 1);
        let worker_shared = Arc::clone(&shared);
        let result = std::panic::catch_unwind(move || {
            let _owner = SubscriptionOwner {
                shared: worker_shared,
                generation: 1,
            };
            panic!("synthetic subscription worker failure");
        });
        assert!(result.is_err());
        assert!(shared.lock().unwrap().is_none());
    }

    #[test]
    fn late_cleanup_cannot_remove_a_newer_generation() {
        let shared = Arc::new(Mutex::new(None));
        let (_sender, _receiver) = register(&shared, 1);
        let old_owner = SubscriptionOwner {
            shared: Arc::clone(&shared),
            generation: 1,
        };
        // The scheduler can retire an old generation before its writer exits.
        *shared.lock().unwrap() = None;
        let (_sender, _receiver) = register(&shared, 2);
        drop(old_owner);
        assert_eq!(shared.lock().unwrap().as_ref().unwrap().generation, 2);
    }
}

#[cfg(test)]
#[path = "../tests/support/synthetic_runtime_catalogs.rs"]
mod synthetic_runtime_catalogs;

#[cfg(test)]
mod runtime_profile_tests {
    use super::*;

    #[test]
    fn offline_catalog_validation_cannot_be_replaced_by_a_collision_query() {
        let error = run([
            "--validate-catalogs".to_string(),
            "--collision-query".to_string(),
            "0,0,0".to_string(),
            "1,0,0".to_string(),
        ])
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("--validate-catalogs cannot be combined with --collision-query")
        );
    }

    use super::synthetic_runtime_catalogs::SyntheticCatalogs;

    #[test]
    fn offline_cli_validates_explicit_bot_names_without_opening_logs() {
        let files = SyntheticCatalogs::new();
        files.write(
            "names.json",
            &serde_json::json!({"entries":[{"name":"Synthetic Pilot"}]}),
        );
        let args = || {
            let mut args = files.args();
            args.extend([
                "--bot-names".to_string(),
                files.0.join("names.json").display().to_string(),
            ]);
            args
        };
        assert!(run(args()).is_ok());
        let mut insufficient = args();
        insufficient.extend(["--bot-fill-total".to_string(), "64".to_string()]);
        assert!(
            run(insufficient)
                .unwrap_err()
                .to_string()
                .contains("insufficient capacity")
        );
        files.write("names.json", &serde_json::json!({"entries":[{"name":""}]}));
        assert!(run(args()).unwrap_err().to_string().contains("bot names"));
        std::fs::remove_file(files.0.join("names.json")).unwrap();
        assert!(run(args()).is_err());
        let mut relative = files.args();
        relative.extend(["--bot-names".to_string(), "names.json".to_string()]);
        assert!(run(relative).unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn direct_smoke_rules_cli_requires_an_absolute_path_and_objective_provenance() {
        let files = SyntheticCatalogs::new();
        let mut relative = files.args();
        relative.extend([
            "--smoke-grenade-rules".to_string(),
            "smoke.json".to_string(),
        ]);
        assert!(
            run(relative)
                .unwrap_err()
                .to_string()
                .contains("--smoke-grenade-rules requires an absolute path")
        );
        let mut unbound = files.args();
        unbound.extend([
            "--smoke-grenade-rules".to_string(),
            files.0.join("smoke.json").display().to_string(),
        ]);
        assert!(
            run(unbound)
                .unwrap_err()
                .to_string()
                .contains("--smoke-grenade-rules requires --objective-catalog")
        );
    }

    #[test]
    fn offline_retail_cli_validates_only_explicit_synthetic_resources() {
        let files = SyntheticCatalogs::new();
        assert!(run(files.args()).is_ok());
        assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 9);
    }

    #[test]
    fn acquisition_gameplay_cli_rejects_incomplete_mode_before_opening_files() {
        let files = SyntheticCatalogs::new();
        let mut relative = files.args();
        relative.extend(["--acquisition-gameplay".into(), "config.json".into()]);
        assert!(
            run(relative)
                .unwrap_err()
                .to_string()
                .contains("--acquisition-gameplay requires an absolute")
        );
        let mut wrong_mode = files.args();
        wrong_mode.extend([
            "--acquisition-gameplay".into(),
            files.0.join("gameplay.json").display().to_string(),
        ]);
        assert!(
            run(wrong_mode)
                .unwrap_err()
                .to_string()
                .contains("Acquisition gameplay requires retail mode1")
        );
        assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 9);
    }

    #[test]
    fn ground_vehicle_cli_requires_explicit_qualified_retail_bindings() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["game_type"] = serde_json::json!(1);
        spawns["maps"][0]["owner_faction"] = serde_json::json!(0);
        spawns["maps"][0]["runtime_indices_verified"] = serde_json::json!(true);
        spawns["maps"][0]["runtime_indices_basis"] = serde_json::json!("SYNTHETIC identity proof");
        files.write("spawns.json", &spawns);
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(files.0.join("spawns.json")).unwrap())
        );
        files.write("vehicles.json", &serde_json::json!({"format":"mag_ground_vehicles","schema_version":1,
            "source":{"spawn_catalog_sha256":digest,"native_elf_sha256":"a".repeat(64),
                "native_binding_sha256":"b".repeat(64),"vehicle_properties_sha256":"c".repeat(64)},
            "vehicles":[{"index":0,"spawn_point_index":0,"spawn_point_token":spawns["maps"][0]["locations"][0]["token"],
                "vehicle_type_token":123,"faction":spawns["maps"][0]["owner_faction"],"team":1,"modifier":0,
                "position":{"x":100.,"y":3.,"z":100.},"rotation":[0.,0.,0.,1.],"health_quarter_points":600}]}));
        let mut args = files.args();
        args.extend([
            "--ground-vehicles".into(),
            files.0.join("vehicles.json").display().to_string(),
        ]);
        assert!(run(args.clone()).is_ok());
        spawns["maps"][0]["runtime_indices_verified"] = serde_json::json!(false);
        files.write("spawns.json", &spawns);
        assert!(
            run(args)
                .unwrap_err()
                .to_string()
                .contains("verified_spawn_pool")
        );
    }

    #[test]
    fn dynamic_body_cli_validates_explicit_resources_without_opening_logs() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["game_type"] = serde_json::json!(1);
        files.write("spawns.json", &spawns);
        let body = serde_json::json!({"format":"mag_dynamic_bodies","schema_version":1,
         "source":{"native_binding_sha256":"a".repeat(64),"shape_data_sha256":"b".repeat(64)},
         "runtime_bindings":{"character_template":"invented","vehicles":[]},
         "templates":[{"id":"invented","kind":"character","roots":[{"id":"body","layer":21,"dead_layer":6,"translation":[0.,0.,0.],"rotation":"identity","shape":{"kind":"sphere","radius":0.25}}]}]});
        files.write("bodies.json", &body);
        let args = || {
            let mut v = files.args();
            v.extend([
                "--dynamic-bodies".to_owned(),
                files.0.join("bodies.json").display().to_string(),
            ]);
            v
        };
        for game_type in [1, 2] {
            spawns["match_profile"]["game_type"] = serde_json::json!(game_type);
            files.write("spawns.json", &spawns);
            assert!(
                run(args()).is_ok(),
                "dynamic bodies rejected mode {game_type}"
            );
        }
        let mut bad = body.clone();
        bad["runtime_bindings"]["character_template"] = serde_json::json!("missing");
        files.write("bodies.json", &bad);
        assert!(
            run(args())
                .unwrap_err()
                .to_string()
                .contains("dynamic_body_bindings")
        );
        files.write("bodies.json", &body);
        spawns["match_profile"]["game_type"] = serde_json::json!(3);
        files.write("spawns.json", &spawns);
        assert_eq!(
            run(args()).unwrap_err().to_string(),
            "dynamic bodies require retail Acquisition or Domination"
        );
        let mut relative = files.args();
        relative.extend(["--dynamic-bodies".to_owned(), "bodies.json".to_owned()]);
        assert!(run(relative).unwrap_err().to_string().contains("absolute"));
        assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 10);
    }
    #[test]
    fn standalone_acquisition_rejects_missing_character_resources_before_opening_log() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["game_type"] = serde_json::json!(1);
        files.write("spawns.json", &spawns);
        let log = files.0.join("should-not-exist.jsonl");
        let mut args = files.runtime_args();
        args.extend([
            "--acquisition-gameplay".into(),
            files.0.join("gameplay.json").display().to_string(),
            "--log".into(),
            log.display().to_string(),
        ]);
        assert_eq!(
            run(args.clone()).unwrap_err().to_string(),
            "acquisition_gameplay_requires_collision_world"
        );
        files.write(
            "world.json",
            &serde_json::json!({
                "format":"mag_collision_world",
                "schema_version":1,
                "coordinate_space":"world",
                "metadata":{"map_name":spawns["maps"][0]["name"],"build":"fixture",
                    "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
                "instances":[]
            }),
        );
        args.extend([
            "--collision-world".into(),
            files.0.join("world.json").display().to_string(),
        ]);
        assert_eq!(
            run(args).unwrap_err().to_string(),
            "acquisition_gameplay_requires_character_damage_rules"
        );
        assert!(!log.exists());
    }

    #[test]
    fn collision_coverage_sidecar_is_hash_bound_before_opening_log() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["game_type"] = serde_json::json!(1);
        files.write("spawns.json", &spawns);
        let map = spawns["maps"][0]["name"].clone();
        files.write(
            "world.json",
            &serde_json::json!({
                "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
                "metadata":{"map_name":map,"build":"fixture",
                    "coverage":"provisional","provenance":["synthetic"],"unknowns":[]},
                "instances":[]
            }),
        );
        let world_sha256 =
            mag_gameplay::navmesh::sha256(&std::fs::read(files.0.join("world.json")).unwrap());
        let sidecar = |world_sha256: &str| {
            serde_json::json!({
                "format":"mag_collision_coverage_v1","map_name":map,"world_sha256":world_sha256,
                "basis":"operator_assertion","provenance":["synthetic"],
                "physics_regions":[{"min":[-10.,-3.,-10.],"max":[10.,3.,10.],
                    "ground_verified":true,"obstacles_verified":true,"provenance":["synthetic"]}]
            })
        };
        files.write("coverage.json", &sidecar(&world_sha256));
        files.write("foreign.json", &sidecar(&"f".repeat(64)));
        let log = files.0.join("should-not-exist.jsonl");
        let mut args = files.runtime_args();
        args.extend([
            "--acquisition-gameplay".into(),
            files.0.join("gameplay.json").display().to_string(),
            "--log".into(),
            log.display().to_string(),
        ]);
        let with = |extra: &[&str]| {
            let mut args = args.clone();
            for name in extra {
                args.push(if name.ends_with(".json") {
                    files.0.join(name).display().to_string()
                } else {
                    name.to_string()
                });
            }
            run(args).unwrap_err().to_string()
        };
        assert_eq!(
            with(&["--collision-coverage", "coverage.json"]),
            "--collision-coverage requires --collision-world"
        );
        assert_eq!(
            with(&[
                "--collision-world",
                "world.json",
                "--collision-coverage",
                "foreign.json"
            ]),
            "collision_coverage_world_mismatch"
        );
        // The matching sidecar is accepted and startup proceeds to the next input.
        assert_eq!(
            with(&[
                "--collision-world",
                "world.json",
                "--collision-coverage",
                "coverage.json"
            ]),
            "acquisition_gameplay_requires_character_damage_rules"
        );
        assert!(!log.exists());
    }

    #[test]
    fn static_partition_cli_is_explicit_bounded_and_map_checked_before_logs() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["game_type"] = serde_json::json!(1);
        files.write("spawns.json", &spawns);
        let mut partition = serde_json::json!({"format":"mag_static_filter_partition","schema_version":1,"map_name":spawns["maps"][0]["name"],"scope":"initial_compiled_collision_and_type1_terrain","source":{"native_binding_sha256":"a".repeat(64),"root_inventory_sha256":"b".repeat(64),"level_archive_sha256":"c".repeat(64),"compiled_catalog_sha256":"d".repeat(64)},"root_count":1,"root_filters":[{"filter_info":9,"root_count":1}]});
        files.write("partition.json", &partition);
        let selected = mag_gameplay::static_collision_partition::Partition::load(
            files.0.join("partition.json"),
        )
        .unwrap();
        let mut rows = [0; 32];
        assert!(
            validate_acquisition_static_partition(
                "synthetic_arena",
                std::slice::from_ref(&selected),
                &rows,
            )
            .is_ok()
        );
        rows[6] |= 1 << 9;
        rows[9] |= 1 << 6;
        assert_eq!(
            validate_acquisition_static_partition("synthetic_arena", &[selected], &rows)
                .unwrap_err()
                .to_string(),
            "static_partition_not_excluded"
        );
        let args = || {
            let mut v = files.args();
            v.extend([
                "--static-partition".to_owned(),
                files.0.join("partition.json").display().to_string(),
            ]);
            v
        };
        for game_type in [1, 2] {
            spawns["match_profile"]["game_type"] = serde_json::json!(game_type);
            files.write("spawns.json", &spawns);
            assert!(
                run(args()).is_ok(),
                "static partition rejected mode {game_type}"
            );
        }
        let mut duplicate = args();
        duplicate.extend([
            "--static-partition".to_owned(),
            files.0.join("partition.json").display().to_string(),
        ]);
        assert!(
            run(duplicate)
                .unwrap_err()
                .to_string()
                .contains("duplicate_static_partition_map")
        );
        partition["map_name"] = serde_json::json!("wrong-map");
        files.write("partition.json", &partition);
        assert!(
            run(args())
                .unwrap_err()
                .to_string()
                .contains("static_partition_base_map_missing")
        );
        assert_eq!(std::fs::read_dir(&files.0).unwrap().count(), 10);
    }

    fn synthetic_match_profile(files: &SyntheticCatalogs) -> PathBuf {
        files.write("profile-spawns.json", &serde_json::json!({
            "format":"mag_spawn_catalog","schema_version":1,"source":"SYNTHETIC CLI registry",
            "active":{"map_name":"synthetic_home","squad_index":0,"default_spawn":0,"available":[0]},
            "match_profile":{"game_type":0,"auto_assign_teams":true,"team_capacity":1,"squad_capacity":1,"attacking_faction":0,"has_bunkers":false,"has_objectives":false},
            "maps":[{"name":"synthetic_home","owner_faction":1,"spawn_count":1,
                "locations":[{"index":0,"token":1,"kind":"on_foot","position":{"x":10.,"y":2.,"z":12.}}],
                "squads":[{"index":0,"team":0,"platoon":0,"squad":0,"faction":null,"available":[0],"default_spawn":0},
                {"index":16,"team":1,"platoon":0,"squad":0,"faction":1,"available":[0],"default_spawn":0}]}]}));
        files.write("profiles.json", &serde_json::json!({"format":"mag_match_profiles_v1","profiles":[{"profile_id":"synthetic-profile","spawn_catalog":files.0.join("profile-spawns.json")}]}));
        files.0.join("profiles.json")
    }

    #[test]
    fn authored_movement_profile_requires_the_explicit_retail_runtime_mesh() {
        let request = |flags: &[&str]| {
            run(flags.iter().map(|s| (*s).to_owned()))
                .unwrap_err()
                .to_string()
        };
        assert_eq!(
            request(&["--movement-profile", "movement.json"]),
            "--movement-profile requires an absolute path"
        );
        assert_eq!(
            request(&["--movement-profile", "/synthetic/movement.json"]),
            "--movement-profile requires --application-id 20231"
        );
        assert_eq!(
            request(&[
                "--application-id",
                "20231",
                "--movement-profile",
                "/synthetic/movement.json"
            ]),
            "--movement-profile requires --navmesh-world"
        );
    }

    #[test]
    fn offline_registry_validation_is_explicit_and_still_requires_base_inputs() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let files = SyntheticCatalogs::new();
                let manifest = synthetic_match_profile(&files);
                let mut args = files.args();
                args.extend(["--match-profiles".into(), manifest.display().to_string()]);
                assert!(run(args.clone()).is_ok());
                let spawn_flag = args.iter().position(|s| s == "--spawn-catalog").unwrap();
                args.drain(spawn_flag..=spawn_flag + 1);
                assert!(
                    run(args)
                        .unwrap_err()
                        .to_string()
                        .contains("--spawn-catalog")
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn one_smoke_flag_binds_every_registry_profile_without_a_base_objective_catalog() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let files = SyntheticCatalogs::new();
                let manifest = synthetic_match_profile(&files);
                files.write("smoke.json", &serde_json::json!({}));
                let smoke = files.0.join("smoke.json").display().to_string();
                let mut args = files.args();
                args.extend([
                    "--match-profiles".into(),
                    manifest.display().to_string(),
                    "--smoke-grenade-rules".into(),
                    smoke,
                ]);
                // No base --objective-catalog: each row binds the rules to its
                // own objective catalog, which this Sabotage row lacks.
                let error = run(args.clone()).unwrap_err().to_string();
                assert!(
                    error.contains("smoke_grenade_objectives_required"),
                    "{error}"
                );
                let mut relative = args.clone();
                *relative.last_mut().unwrap() = "smoke.json".into();
                assert!(
                    run(relative)
                        .unwrap_err()
                        .to_string()
                        .contains("--smoke-grenade-rules requires an absolute path")
                );
                // A game-type-1 row needs Acquisition gameplay to carry smoke.
                let mut spawns = files.read("profile-spawns.json");
                spawns["match_profile"]["game_type"] = serde_json::json!(1);
                files.write("profile-spawns.json", &spawns);
                let error = run(args).unwrap_err().to_string();
                assert!(error.contains("smoke_grenade_mode_required"), "{error}");
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn direct_acquisition_worker_takes_the_smoke_flag() {
        let files = SyntheticCatalogs::new();
        let mut args = files.args();
        args.extend([
            "--smoke-grenade-rules".into(),
            files.0.join("smoke.json").display().to_string(),
            "--objective-catalog".into(),
            files.0.join("objectives.json").display().to_string(),
            "--acquisition-gameplay".into(),
            files.0.join("acquisition.json").display().to_string(),
        ]);
        // The flag passes argument validation; the invented worker then
        // stops at its missing Acquisition configuration file.
        let error = run(args).unwrap_err().to_string();
        assert!(
            !error.contains("smoke") && error.contains("os error"),
            "{error}"
        );
    }

    #[test]
    fn sabotage_registry_has_no_active_command_setup() {
        let files = SyntheticCatalogs::new();
        let manifest = synthetic_match_profile(&files);
        files.write(
            "synthetic_home--active-abilities.json",
            &serde_json::json!({}),
        );
        let registry = mag_gameplay::match_profiles::Registry::load(&manifest).unwrap();
        assert!(!registry.has_active_abilities());
        let mut args = files.args();
        args.extend(["--match-profiles".into(), manifest.display().to_string()]);
        assert!(run(args).is_ok());
    }

    #[test]
    fn invalid_registry_fails_before_creating_log_or_listener() {
        let files = SyntheticCatalogs::new();
        let manifest = synthetic_match_profile(&files);
        let mut broken = files.read("profiles.json");
        broken["profiles"][0]["spawn_catalog"] = serde_json::json!(files.0.join("missing.json"));
        files.write("profiles.json", &broken);
        let mut args = files.runtime_args();
        args.extend([
            "--match-profiles".into(),
            manifest.display().to_string(),
            "--log".into(),
            files.0.join("should-not-exist.jsonl").display().to_string(),
        ]);
        assert!(run(args).is_err());
        assert!(!files.0.join("should-not-exist.jsonl").exists());
    }

    #[test]
    fn registry_rejects_unswitchable_world_extras_before_loading() {
        for flags in [
            vec!["--bot-population", "/synthetic/missing.json"],
            vec!["--bot-resupply-approach-policy", "/synthetic/missing.json"],
            vec!["--development-dummy"],
            vec!["--leadership-passives", "/synthetic/missing.json"],
            vec!["--bunker-catalog", "/synthetic/missing.json"],
        ] {
            let mut args = vec![
                "--application-id".to_string(),
                "20231".into(),
                "--match-profiles".into(),
                "/synthetic/profiles.json".into(),
            ];
            args.extend(flags.into_iter().map(str::to_owned));
            assert!(run(args).is_err());
        }
    }

    #[test]
    fn registry_profile_bots_are_validated_per_profile_before_logs() {
        std::thread::Builder::new()
            .stack_size(1024 * 1024)
            .spawn(|| {
                let files = SyntheticCatalogs::new();
                let manifest = synthetic_match_profile(&files);
                let mut args = files.args();
                args.extend(["--match-profiles".into(), manifest.display().to_string()]);
                // A worker-wide fill total needs at least one profile population.
                let mut defaulted = args.clone();
                defaulted.extend(["--bot-fill-total".into(), "2".into()]);
                assert!(
                    run(defaulted)
                        .unwrap_err()
                        .to_string()
                        .contains("bot_fill_total_requires_profile_bot_population")
                );
                // A population authored for another map is refused, naming its profile.
                files.write("profile-bots.json", &serde_json::json!({
                    "format":"mag_bot_population_v1","map_name":"synthetic_other","firearms":{"enabled":false},
                    "slots":[{"team":0,"position":{"x":10.,"y":2.,"z":12.}},{"team":1,"position":{"x":14.,"y":2.,"z":12.}}]}));
                files.write("profiles.json", &serde_json::json!({"format":"mag_match_profiles_v1","profiles":[{
                    "profile_id":"synthetic-profile","spawn_catalog":files.0.join("profile-spawns.json"),
                    "bot_population":files.0.join("profile-bots.json"),"bot_fill_total":2}]}));
                let error = run(args).unwrap_err().to_string();
                assert!(
                    error.contains("match profile synthetic-profile: bot_population_map_profile_mismatch"),
                    "{error}"
                );
            })
            .unwrap()
            .join()
            .unwrap();
    }

    #[test]
    fn bot_resupply_catalog_is_explicit_and_requires_population_before_logs() {
        let files = SyntheticCatalogs::new();
        files.write(
            "bot-supply.json",
            &serde_json::json!({"invalid":"not implicitly enabled"}),
        );
        assert!(run(files.args()).is_ok());
        let mut relative = files.args();
        relative.extend(["--bot-resupply-catalog".into(), "bot-supply.json".into()]);
        assert!(
            run(relative)
                .unwrap_err()
                .to_string()
                .contains("--bot-resupply-catalog requires an absolute")
        );
        let mut missing_population = files.args();
        missing_population.extend([
            "--bot-resupply-catalog".into(),
            files.0.join("bot-supply.json").display().to_string(),
        ]);
        assert!(
            run(missing_population)
                .unwrap_err()
                .to_string()
                .contains("requires configured bot population")
        );
    }

    #[test]
    fn bot_resupply_approach_is_explicit_and_requires_bound_service_before_logs() {
        let files = SyntheticCatalogs::new();
        files.write(
            "approach.json",
            &serde_json::json!({"invalid":"not implicitly enabled"}),
        );
        assert!(run(files.args()).is_ok());
        for (path, reason) in [
            ("approach.json".to_owned(), "requires an absolute path"),
            (
                files.0.join("approach.json").display().to_string(),
                "requires --bot-resupply-catalog",
            ),
        ] {
            let mut args = files.runtime_args();
            let log = files.0.join("should-not-exist.jsonl");
            args.extend([
                "--log".into(),
                log.display().to_string(),
                "--bot-resupply-approach-policy".into(),
                path,
            ]);
            let error = run(args).unwrap_err().to_string();
            assert!(error.contains(reason), "unexpected error: {error}");
            assert!(
                !log.exists(),
                "invalid approach must fail before opening the log"
            );
        }
        let mut missing_value = files.args();
        missing_value.push("--bot-resupply-approach-policy".into());
        assert!(
            run(missing_value)
                .unwrap_err()
                .to_string()
                .contains("requires an absolute path")
        );
    }

    #[test]
    fn firearm_assist_rules_are_explicit_retail_bound_and_allow_zero_timeout() {
        let files = SyntheticCatalogs::new();
        files.write(
            "assist.json",
            &serde_json::json!({"invalid":"not implicitly enabled"}),
        );
        assert!(run(files.args()).is_ok());
        let args = || {
            let mut args = files.args();
            args.extend([
                "--firearm-assist-rules".into(),
                files.0.join("assist.json").display().to_string(),
            ]);
            args
        };
        for timeout in [0, 37] {
            files.write("assist.json", &serde_json::json!({"schema_version":1,"application_id":20231,"assist_timeout_ms":timeout}));
            assert!(
                run(args()).is_ok(),
                "valid explicit assist rules must load before logs"
            );
        }
        files.write(
            "assist.json",
            &serde_json::json!({"schema_version":1,"application_id":20230,"assist_timeout_ms":37}),
        );
        assert!(run(args()).is_err());
        files.write(
            "assist.json",
            &serde_json::json!({"schema_version":1,"application_id":20231,"assist_timeout_ms":-1}),
        );
        assert!(run(args()).is_err());
        let mut relative = files.args();
        relative.extend(["--firearm-assist-rules".into(), "assist.json".into()]);
        assert!(run(relative).unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn character_damage_rules_are_optional_explicit_and_bound_to_exact_spawn_bytes() {
        let files = SyntheticCatalogs::new();
        files.write(
            "character-damage.json",
            &serde_json::json!({"invalid":"not implicitly loaded"}),
        );
        assert!(run(files.args()).is_ok());
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(files.0.join("spawns.json")).unwrap())
        );
        files.write(
            "character-damage.json",
            &serde_json::json!({"schema_version":1,"application_id":20231,
            "map_name":"synthetic_arena","game_type":2,"friendly_fire_multiplier":0.375,
            "provenance":{"spawn_catalog_sha256":digest,"source":"synthetic"}}),
        );
        let args = || {
            let mut args = files.args();
            args.extend([
                "--character-damage-rules".into(),
                files.0.join("character-damage.json").display().to_string(),
            ]);
            args
        };
        assert!(run(args()).is_ok());
        // Same parsed match identity, different original file bytes: fail clearly.
        let mut bytes = std::fs::read(files.0.join("spawns.json")).unwrap();
        bytes.push(b' ');
        std::fs::write(files.0.join("spawns.json"), bytes).unwrap();
        assert!(
            run(args())
                .unwrap_err()
                .to_string()
                .contains("match_mismatch")
        );
        let mut relative = files.args();
        relative.extend([
            "--character-damage-rules".into(),
            "character-damage.json".into(),
        ]);
        assert!(run(relative).unwrap_err().to_string().contains("absolute"));
    }

    #[test]
    fn incapacitation_is_optional_and_requires_explicit_disk_body_data_when_enabled() {
        let files = SyntheticCatalogs::new();
        assert!(run(files.args()).is_ok());
        files.write(
            "incapacitation.json",
            &serde_json::json!({"schema_version":1,
            "incapacitated_damage_threshold_hp":83.0,"incapacitated_duration_seconds":7.0,
            "excluded_character_states":[22]}),
        );
        let args = || {
            let mut args = files.args();
            args.extend([
                "--incapacitation-rules".into(),
                files.0.join("incapacitation.json").display().to_string(),
            ]);
            args
        };
        assert!(
            run(args())
                .unwrap_err()
                .to_string()
                .contains("explicit body multipliers")
        );
        let mut combat = files.read("combat.json");
        for body in combat["body"].as_array_mut().unwrap() {
            body["incapacitated_multiplier"] = serde_json::json!(0.37);
        }
        files.write("combat.json", &combat);
        assert!(run(args()).is_ok());
        // Projected rules carry an audit-only provenance object.
        files.write(
            "incapacitation.json",
            &serde_json::json!({"schema_version":1,
            "incapacitated_damage_threshold_hp":83.0,"incapacitated_duration_seconds":7.0,
            "excluded_character_states":[],"provenance":{"member_sha256":"invented"}}),
        );
        assert!(run(args()).is_ok());
        let mut relative = files.args();
        relative.extend([
            "--incapacitation-rules".into(),
            "incapacitation.json".into(),
        ]);
        assert!(
            run(relative)
                .unwrap_err()
                .to_string()
                .contains("absolute path")
        );
        files.write(
            "incapacitation.json",
            &serde_json::json!({"schema_version":1}),
        );
        assert!(run(args()).is_err());
    }

    #[test]
    fn offline_retail_cli_rejects_unresolved_raw_items_in_armory_bootstrap() {
        let files = SyntheticCatalogs::new();
        let mut items = files.read("items.json");
        items["items"]["FactionItemList0:2"]["definition"] = serde_json::Value::Null;
        files.write("items.json", &items);
        let digest = format!(
            "{:x}",
            Sha256::digest(std::fs::read(files.0.join("items.json")).unwrap())
        );
        for (name, pointer) in [
            ("bootstrap.json", "/provenance/catalog_sha256"),
            ("combat.json", "/provenance/catalog_sha256"),
            (
                "armory.json",
                "/source_sha256/gameplay~1data~1retail-items.json",
            ),
            (
                "skills.json",
                "/source_sha256/gameplay~1data~1retail-items.json",
            ),
        ] {
            let mut value = files.read(name);
            *value.pointer_mut(pointer).unwrap() = serde_json::json!(digest);
            files.write(name, &value);
        }
        assert!(
            run(files.args())
                .unwrap_err()
                .to_string()
                .contains("unresolved_saved_item")
        );
    }

    #[test]
    fn offline_retail_cli_requires_selected_collision_world_to_match_the_map() {
        let files = SyntheticCatalogs::new();
        let mut world = serde_json::json!({
            "format":"mag_collision_world","schema_version":1,"coordinate_space":"world",
            "metadata":{"map_name":"different_synthetic_arena","build":"synthetic",
                "coverage":"provisional","provenance":["Synthetic CLI test"],"unknowns":["Not native geometry"]},
            "instances":[{"id":"synthetic-wall","vertices_xyz":[[2.,-2.,-2.],[2.,2.,-2.],[2.,0.,2.]],"triangles":[[0,1,2]]}]
        });
        files.write("world.json", &world);
        let mut args = files.args();
        args.extend([
            "--collision-world".into(),
            files.0.join("world.json").to_str().unwrap().into(),
        ]);
        assert!(
            run(args.clone())
                .unwrap_err()
                .to_string()
                .contains("collision world map does not match")
        );
        world["metadata"]["map_name"] = serde_json::json!("synthetic_arena");
        files.write("world.json", &world);
        assert!(run(args).is_ok());
    }

    #[test]
    fn offline_retail_cli_rejects_skill_profile_mismatch() {
        let files = SyntheticCatalogs::new();
        let mut skills = files.read("skills.json");
        skills["schema_version"] = serde_json::json!(1);
        skills["application_id"] = serde_json::json!(20230);
        files.write("skills.json", &skills);
        let error = run(files.args()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("skill catalog application does not match")
        );
    }

    #[test]
    fn offline_retail_cli_rejects_item_projection_hash_mismatch() {
        let files = SyntheticCatalogs::new();
        let mut combat = files.read("combat.json");
        combat["provenance"]["catalog_sha256"] = serde_json::json!("0".repeat(64));
        files.write("combat.json", &combat);
        let error = run(files.args()).unwrap_err();
        assert!(error.to_string().contains("retail combat does not match"));
    }

    #[test]
    fn retail_cli_keeps_bunker_and_alpha_inputs_unsupported() {
        for flag in [
            "--structure-catalog",
            "--bunker-catalog",
            "--bunker-exit-policy",
        ] {
            let files = SyntheticCatalogs::new();
            let mut args = files.args();
            args.extend([
                flag.into(),
                files
                    .0
                    .join("unused-synthetic.json")
                    .to_str()
                    .unwrap()
                    .into(),
            ]);
            let error = run(args).unwrap_err();
            assert!(
                error
                    .to_string()
                    .contains("retail structure/bunker authority is unverified"),
                "{flag}: {error}"
            );
        }
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["has_bunkers"] = serde_json::json!(true);
        files.write("spawns.json", &spawns);
        assert!(
            run(files.args()).unwrap_err().to_string().contains(
                "retail bunker features are unsupported without verified retail authority"
            )
        );
    }

    #[test]
    fn offline_retail_cli_requires_catalog_for_enabled_objectives() {
        let files = SyntheticCatalogs::new();
        let mut spawns = files.read("spawns.json");
        spawns["match_profile"]["has_objectives"] = serde_json::json!(true);
        files.write("spawns.json", &spawns);
        let error = run(files.args()).unwrap_err();
        assert!(error.to_string().contains("objective catalog required"));
    }

    #[test]
    fn offline_retail_cli_requires_bootstrap_for_all_factions() {
        let files = SyntheticCatalogs::new();
        let mut bootstrap = files.read("bootstrap.json");
        bootstrap["factions"].as_array_mut().unwrap().pop();
        files.write("bootstrap.json", &bootstrap);
        let error = run(files.args()).unwrap_err();
        assert!(
            error
                .to_string()
                .contains("retail bootstrap must include all supported factions")
        );
    }

    #[test]
    fn offline_retail_cli_does_not_create_runtime_logs() {
        let files = SyntheticCatalogs::new();
        let mut args = files.args();
        for (flag, name) in [("--log", "runtime.log"), ("--tick-telemetry", "ticks.log")] {
            args.push(flag.into());
            args.push(files.0.join(name).to_str().unwrap().into());
        }
        assert!(
            run(args)
                .unwrap_err()
                .to_string()
                .contains("does not open logs")
        );
        assert!(!files.0.join("runtime.log").exists());
        assert!(!files.0.join("ticks.log").exists());
    }

    #[test]
    fn log_rotation_flags_are_validated_before_any_log_opens() {
        let files = SyntheticCatalogs::new();
        let with = |extra: &[&str]| {
            let mut args = files.args();
            args.extend(extra.iter().map(|value| value.to_string()));
            run(args).unwrap_err().to_string()
        };
        assert!(with(&["--log-retention-s", "3600"]).contains("requires --log-segment-bytes"));
        assert!(with(&["--log-segment-bytes", "1024"]).contains("log_segment_bytes_too_small"));
        assert!(
            with(&["--log-segment-bytes", "1048576", "--log-retention-s", "0"])
                .contains("log_retention_must_be_positive")
        );
        assert!(with(&["--log-segment-bytes", "x"]).contains("invalid --log-segment-bytes"));
        assert!(
            with(&[
                "--log-segment-bytes",
                "1048576",
                "--log-retention-s",
                "3600"
            ])
            .contains("does not open logs")
        );
    }

    #[test]
    fn application_selection_is_explicit_and_bounded() {
        assert_eq!(parse_application_id("20230").unwrap(), 20230);
        assert_eq!(parse_application_id("20231").unwrap(), 20231);
        for invalid in ["", "0", "20232", " 20231", "20231 "] {
            assert!(parse_application_id(invalid).is_err());
        }
    }
    #[test]
    fn retail_paths_cannot_fall_back_to_sibling_defaults() {
        assert!(validate_runtime_paths(Some(20231), &[("--item-catalog", None)]).is_err());
        assert!(
            validate_runtime_paths(
                Some(20231),
                &[("--item-catalog", Some(Path::new("relative.json")))]
            )
            .is_err()
        );
        assert!(
            validate_runtime_paths(
                Some(20231),
                &[("--item-catalog", Some(Path::new("/catalogs/items.json")))]
            )
            .is_ok()
        );
        assert!(validate_runtime_paths(None, &[("legacy-default", None)]).is_ok());
    }
    #[test]
    fn retail_projections_must_identify_the_exact_item_bytes() {
        let temporary = std::env::temp_dir().join(format!(
            "synthetic-retail-links-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&temporary).unwrap();
        let items = temporary.join("items.json");
        let projection = temporary.join("projection.json");
        std::fs::write(&items, b"synthetic item bytes").unwrap();
        let digest = format!("{:x}", Sha256::digest(b"synthetic item bytes"));
        std::fs::write(
            &projection,
            serde_json::to_vec(&serde_json::json!({"provenance":{"catalog_sha256":digest}}))
                .unwrap(),
        )
        .unwrap();
        let matched = validate_item_catalog_links(
            &items,
            &[("projection", &projection, "/provenance/catalog_sha256")],
        )
        .is_ok();
        std::fs::write(&items, b"different synthetic bytes").unwrap();
        let mismatched = validate_item_catalog_links(
            &items,
            &[("projection", &projection, "/provenance/catalog_sha256")],
        )
        .is_err();
        std::fs::write(&projection, b"{}").unwrap();
        let missing = validate_item_catalog_links(
            &items,
            &[("projection", &projection, "/provenance/catalog_sha256")],
        )
        .is_err();
        std::fs::remove_dir_all(temporary).unwrap();
        assert!(matched && mismatched && missing);
    }
}

#[cfg(test)]
mod initial_spawn_delay_flag_tests {
    use super::parse_team_delays;

    #[test]
    fn one_value_applies_to_both_teams_and_pairs_are_per_team() {
        assert_eq!(parse_team_delays("10000"), Some([10000, 10000]));
        assert_eq!(parse_team_delays("10000,15000"), Some([10000, 15000]));
        for invalid in ["", "x", "10000,", "1,2,3", "-5", "10000;15000"] {
            assert_eq!(parse_team_delays(invalid), None, "{invalid:?}");
        }
    }
}
